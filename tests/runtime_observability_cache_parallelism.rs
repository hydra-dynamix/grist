use grist::core::{
    BudgetProfile, BudgetSelection, ContentIdentity, Input, OperationKind, ParseRequest,
    ParserInfo, ProviderSet, RequestId, SourceInfo, canonical_json_bytes, options_digest,
    sha256_hex,
};
use grist::ingest::Ingestor;
use grist::runtime::{
    CacheEntry, CacheKey, CanonicalOrderKey, ContentAddressedCache, MetricEvent, MetricsHook,
    MetricsSink, ParallelJob, ParallelOutput, ParallelUnitKind, ParallelismOptions,
    deterministic_parallel_collect, deterministic_parallel_for_each_ordered, load_or_compute,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::convert::Infallible;
use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Default)]
struct RecordingMetrics(Arc<Mutex<Vec<MetricEvent>>>);

impl MetricsSink for RecordingMetrics {
    fn record(&self, event: &MetricEvent) {
        self.0.lock().unwrap().push(event.clone());
    }
}

#[test]
fn default_metrics_hook_exposes_counts_without_source_or_metadata_content() {
    let recording = RecordingMetrics::default();
    let retained = recording.clone();
    let ingestor = Ingestor::builtin().unwrap().with_metrics(recording);
    let secret = "private-alice@example.test-quarterly-plan.md";
    let content = "# prompt\n\npassword=hunter42\nAlice@example.test";
    let request = ParseRequest::new(
        RequestId::new("safe-metric-request").unwrap(),
        Input::utf8(content),
        SourceInfo::new(secret)
            .with_uri("mailbox://alice@example.test/secret")
            .with_repository_relative_path(secret),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        ProviderSet::none(),
    );

    let envelope = ingestor.ingest(request).unwrap();
    assert!(envelope.payload.is_some());
    let events = retained.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    let encoded = serde_json::to_string(&events[0]).unwrap();
    for forbidden in [
        secret,
        content,
        "alice@example.test",
        "hunter42",
        "mailbox://",
        "prompt",
    ] {
        assert!(!encoded.contains(forbidden), "metrics leaked {forbidden}");
    }
    assert!(events[0].values.input_bytes >= content.len() as u64);
}

#[derive(Default)]
struct MemoryCache(Mutex<BTreeMap<String, CacheEntry>>);

impl ContentAddressedCache for MemoryCache {
    type Error = io::Error;

    fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, Self::Error> {
        Ok(self.0.lock().unwrap().get(&key.digest).cloned())
    }

    fn put(&self, key: &CacheKey, entry: &CacheEntry) -> Result<(), Self::Error> {
        self.0
            .lock()
            .unwrap()
            .insert(key.digest.clone(), entry.clone());
        Ok(())
    }
}

fn cache_key(options: Value) -> CacheKey {
    CacheKey::for_identity(
        OperationKind::Parse,
        &ContentIdentity::for_raw_bytes(b"same exact source bytes"),
        &ParserInfo::new("fixture-parser").with_implementation("fixture-backend", "7"),
        "grist/fixture/v1",
        options_digest(&options).unwrap(),
        [options_digest(&json!({"provider": "recording-v2"})).unwrap()],
    )
    .unwrap()
}

#[test]
fn cache_keys_validate_material_and_hits_expose_reuse_provenance() {
    let cache = MemoryCache::default();
    let key = cache_key(json!({"mode": "strict"}));
    let equivalent = cache_key(json!({"mode": "strict"}));
    let changed = cache_key(json!({"mode": "lossy"}));
    assert_eq!(key, equivalent);
    assert_ne!(key.digest, changed.digest);
    let mut tampered = key.clone();
    tampered.payload_schema_version = "grist/fixture/v2".into();
    assert!(tampered.validate().is_err());

    let computes = AtomicUsize::new(0);
    let first = load_or_compute(&cache, &key, OperationKind::Parse, || {
        computes.fetch_add(1, Ordering::SeqCst);
        Ok::<_, io::Error>(json!({"nodes": [1, 2, 3], "status": "complete"}))
    })
    .unwrap();
    assert!(!first.reuse.reused());

    let second = load_or_compute(&cache, &key, OperationKind::Parse, || {
        computes.fetch_add(1, Ordering::SeqCst);
        Ok::<_, io::Error>(json!({"unexpected": true}))
    })
    .unwrap();
    assert_eq!(computes.load(Ordering::SeqCst), 1);
    assert!(second.reuse.reused());
    assert_eq!(
        canonical_json_bytes(&first.value).unwrap(),
        canonical_json_bytes(&second.value).unwrap()
    );
    let provenance = second
        .reuse
        .provenance_step(OperationKind::Parse)
        .unwrap()
        .unwrap();
    assert_eq!(provenance.implementation, "grist.cache.reuse/v1");
    assert_eq!(provenance.warnings, ["grist.cache.reused"]);
}

#[test]
fn corrupt_or_noncanonical_cache_entries_are_rejected_instead_of_reused() {
    let cache = MemoryCache::default();
    let key = cache_key(json!({"mode": "strict"}));
    let value = json!({"a": 1, "b": 2});
    let mut entry = CacheEntry::from_value(&key, &value).unwrap();
    entry.canonical_output.push(b' ');
    entry.canonical_output_sha256 = sha256_hex(&entry.canonical_output);
    cache.put(&key, &entry).unwrap();

    let error = load_or_compute(&cache, &key, OperationKind::Parse, || {
        Ok::<_, io::Error>(value.clone())
    })
    .unwrap_err();
    assert!(error.to_string().contains("not canonical JSON"));
}

fn unit_jobs() -> Vec<ParallelJob<(u64, &'static str)>> {
    [
        ParallelUnitKind::RepositoryFile,
        ParallelUnitKind::ArchiveMember,
        ParallelUnitKind::Slide,
        ParallelUnitKind::Sheet,
        ParallelUnitKind::Page,
    ]
    .into_iter()
    .enumerate()
    .rev()
    .map(|(index, kind)| {
        ParallelJob::new(
            CanonicalOrderKey::source_order(index as u64),
            kind,
            (index as u64, "parsed"),
        )
    })
    .collect()
}

#[test]
fn serial_and_scrambled_parallel_completion_serialize_identically() {
    let parse = |(index, text): (u64, &'static str)| -> Result<Value, Infallible> {
        thread::sleep(Duration::from_millis((5 - index) * 2));
        Ok(json!({"index": index, "text": text}))
    };
    let serial =
        deterministic_parallel_collect(unit_jobs(), ParallelismOptions::serial(), parse).unwrap();
    let parallel =
        deterministic_parallel_collect(unit_jobs(), ParallelismOptions::new(4, 5), parse).unwrap();
    let project = |values: Vec<ParallelOutput<Value>>| {
        values
            .into_iter()
            .map(|value| (value.order, value.kind, value.output))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        canonical_json_bytes(&project(serial)).unwrap(),
        canonical_json_bytes(&project(parallel)).unwrap()
    );
}

#[test]
fn ordered_parallel_delivery_never_prefetches_beyond_max_in_flight() {
    let produced = Arc::new(AtomicUsize::new(0));
    let emitted = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let jobs = {
        let produced = produced.clone();
        let emitted = emitted.clone();
        let peak = peak.clone();
        (0_u64..19).map(move |index| {
            let now = produced.fetch_add(1, Ordering::SeqCst) + 1 - emitted.load(Ordering::SeqCst);
            peak.fetch_max(now, Ordering::SeqCst);
            ParallelJob::new(
                CanonicalOrderKey::source_order(index),
                ParallelUnitKind::Page,
                index,
            )
        })
    };
    deterministic_parallel_for_each_ordered(
        jobs,
        ParallelismOptions::new(3, 4),
        |value| Ok::<_, io::Error>(value * 2),
        |output| {
            assert_eq!(output.output, output.order.primary * 2);
            emitted.fetch_add(1, Ordering::SeqCst);
        },
    )
    .unwrap();
    assert_eq!(produced.load(Ordering::SeqCst), 19);
    assert_eq!(emitted.load(Ordering::SeqCst), 19);
    assert!(peak.load(Ordering::SeqCst) <= 4);
}

#[test]
fn metrics_adapter_panics_cannot_change_operation_results() {
    struct PanickingSink;
    impl MetricsSink for PanickingSink {
        fn record(&self, _event: &MetricEvent) {
            panic!("telemetry backend failure")
        }
    }
    let hook = MetricsHook::new(PanickingSink);
    hook.emit(&MetricEvent::new(
        OperationKind::Parse,
        grist::runtime::MetricPhase::Complete,
        Default::default(),
    ));
}
