#![cfg(feature = "media")]

use grist::core::{
    BudgetProfile, BudgetSelection, CancellationToken, ContentIdentity, Input, LocationComponent,
    OperationControl, OperationStatus, ParseRequest, ProviderSet, RequestId, ResourceBudget,
    SourceInfo,
};
use grist::detect::{ContentKind, DetectionOptions, DetectionStatus, detect_with_registry};
use grist::document_graph::{DocumentGraphContext, DocumentNodeKind, ToDocumentGraph};
use grist::registry::{ParserSelection, builtin_parser_registry};
use grist::render::{RenderFormat, RenderOptions, render_document_graph};
use grist::segment::{SegmentOptions, segment_document_graph};
use grist::subtitle::{
    SubtitleFormat, SubtitleOptions, parse_srt, parse_subtitle_with_operation_control, parse_ttml,
    parse_webvtt,
};

const SRT: &str = "2\r\n00:00:03,000 --> 00:00:05,000\r\nBOB: Later\r\n\r\n1\r\n00:00:01,000 --> 00:00:04,000\r\nALICE: First\r\n";
const VTT: &str = "WEBVTT Example\n\nSTYLE\n::cue(.loud) { color: lime; }\n\nREGION\nid:bottom\nwidth:80%\n\nintro\n00:01.000 --> 00:02.500 region:bottom line:90%\n<v Ada><c.loud>Hello &amp; welcome</c></v>\n";
const TTML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<tt xmlns="http://www.w3.org/ns/ttml" xmlns:tts="http://www.w3.org/ns/ttml#styling" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" xmlns:ttm="http://www.w3.org/ns/ttml#metadata" xml:lang="en" ttp:frameRate="25">
<head><styling><style xml:id="loud" tts:fontWeight="bold"/></styling><layout><region xml:id="bottom" tts:origin="10% 80%"/></layout></head>
<body><div><p xml:id="p1" region="bottom" begin="1s" dur="2s"><span style="loud" ttm:agent="Alice">Hello</span><br/>world</p></div></body>
</tt>"#;

#[test]
fn srt_preserves_exact_ranges_speakers_overlap_and_time_order() {
    let envelope = parse_srt(
        SRT.as_bytes(),
        SourceInfo::new("captions.srt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(envelope.status, OperationStatus::Partial);
    let document = envelope.payload().unwrap();
    assert_eq!(
        document
            .cues
            .iter()
            .map(|cue| cue.timing.start_ms)
            .collect::<Vec<_>>(),
        [Some(3000), Some(1000)]
    );
    assert_eq!(document.cues[0].speaker.as_deref(), Some("BOB"));
    assert!(
        document
            .diagnostics
            .iter()
            .any(|item| item.code == "subtitle.timing.overlap")
    );
    let first_by_time = &document.cues[1];
    let text_range = first_by_time.text_locator.components()[0]
        .as_text_range()
        .unwrap();
    assert_eq!(
        &document.decoded_text[text_range.byte_start..text_range.byte_end],
        "ALICE: First"
    );
    assert!(matches!(
        first_by_time.timing.locator.as_ref().unwrap().components()[0],
        LocationComponent::MediaTime {
            start_ms: 1000,
            end_ms: 4000,
            ..
        }
    ));
    assert_eq!(document.transcript.entries[0].text, "First");
}
#[test]
fn webvtt_preserves_regions_styles_settings_voice_and_entities() {
    let envelope = parse_webvtt(
        VTT.as_bytes(),
        SourceInfo::new("captions.vtt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.styles.len(), 1);
    assert_eq!(document.styles[0].properties["color"], "lime");
    assert_eq!(document.tracks.len(), 1);
    assert!(document.regions.iter().any(|region| region.id == "bottom"));
    assert_eq!(document.cues[0].track_id, "track-0");
    assert_eq!(document.cues[0].region_id.as_deref(), Some("bottom"));
    assert_eq!(document.cues[0].settings["line"], "90%");
    assert_eq!(document.cues[0].speaker.as_deref(), Some("Ada"));
    assert_eq!(document.cues[0].text, "Hello & welcome");
    assert_eq!(document.cues[0].runs[0].style.as_deref(), Some("loud"));
}

#[test]
fn ttml_preserves_deep_structure_styles_tracks_runs_and_duration_timing() {
    let nested = TTML
        .replace("<body><div>", &format!("<body><div>{}", "<div>".repeat(40)))
        .replace(
            "</div></body>",
            &format!("{}</div></body>", "</div>".repeat(40)),
        );
    let envelope = parse_ttml(
        nested.as_bytes(),
        SourceInfo::new("captions.ttml"),
        &SubtitleOptions::default(),
    );
    assert_eq!(
        envelope.status,
        OperationStatus::Complete,
        "{:?}",
        envelope.diagnostics
    );
    let document = envelope.payload().unwrap();
    assert_eq!(document.styles[0].id, "loud");
    assert!(document.regions.iter().any(|region| region.id == "bottom"));
    assert_eq!(document.cues[0].timing.start_ms, Some(1000));
    assert_eq!(document.cues[0].timing.end_ms, Some(3000));
    assert_eq!(document.cues[0].speaker.as_deref(), Some("Alice"));
    assert_eq!(document.cues[0].text, "Hello\nworld");
    assert_eq!(document.cues[0].runs.len(), 3);
    assert!(document.cues[0].structure_locator.is_some());
}

#[test]
fn encodings_registry_detection_graph_segment_render_and_schema_are_stable() {
    let mut utf16 = vec![0xff, 0xfe];
    for unit in SRT.encode_utf16() {
        utf16.extend(unit.to_le_bytes());
    }
    let envelope = parse_srt(
        &utf16,
        SourceInfo::new("utf16.srt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(envelope.payload().unwrap().encoding.label(), "utf-16le");

    let registry = builtin_parser_registry().unwrap();
    for format in ["srt", "webvtt", "ttml"] {
        assert!(matches!(
            registry.select_format(format),
            ParserSelection::Available(_)
        ));
    }
    let detected = detect_with_registry(
        std::path::Path::new("extensionless"),
        VTT.as_bytes(),
        None,
        None,
        &grist::core::Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_eq!(detected.status, DetectionStatus::Selected);
    assert_eq!(detected.content_kind, ContentKind::WebVtt);

    let parsed = parse_webvtt(
        VTT.as_bytes(),
        SourceInfo::new("captions.vtt"),
        &SubtitleOptions::default(),
    );
    let source_identity = parsed.identity.clone().unwrap();
    let document = parsed.payload().unwrap();
    let graph = document
        .to_document_graph(DocumentGraphContext::new("subtitle:test"))
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::Cue)
            .count(),
        1
    );
    let graph_identity = ContentIdentity::default()
        .with_canonical_payload(graph.schema_version.as_str(), &graph)
        .unwrap();
    let segments = segment_document_graph(
        &graph,
        &source_identity,
        &graph_identity,
        &SegmentOptions::default(),
        None,
    )
    .unwrap();
    assert!(
        segments
            .segments
            .iter()
            .any(|segment| segment.text.contains("Hello & welcome"))
    );
    let rendered =
        render_document_graph(&graph, RenderFormat::PlainText, &RenderOptions::default()).unwrap();
    assert_eq!(rendered.content.trim(), "Hello & welcome");
    rendered.validate_source_map().unwrap();

    #[cfg(feature = "schemas")]
    for (name, value) in [
        ("subtitle", serde_json::to_value(document).unwrap()),
        ("subtitle-envelope", serde_json::to_value(&parsed).unwrap()),
        (
            "subtitle-options",
            serde_json::to_value(SubtitleOptions::default()).unwrap(),
        ),
    ] {
        let schema = grist::schema::schema_json(name).unwrap_or_else(|| panic!("missing {name}"));
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors = validator
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect::<Vec<_>>();
        assert!(errors.is_empty(), "{name} schema errors: {errors:?}");
    }
}

#[test]
fn format_specific_clocks_ttml_structure_and_inherited_timing_fail_closed() {
    let bad_srt = "1\n00:00:01.000 --> 00:00:02.000\nwrong separator\n";
    let parsed = parse_srt(
        bad_srt.as_bytes(),
        SourceInfo::new("bad-clock.srt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    assert_eq!(parsed.payload().unwrap().cues[0].timing.start_ms, None);

    let bad_vtt = "WEBVTT\n\n00:00:01,000 --> 00:00:02,000\nwrong separator\n";
    let parsed = parse_webvtt(
        bad_vtt.as_bytes(),
        SourceInfo::new("bad-clock.vtt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    assert_eq!(parsed.payload().unwrap().cues[0].timing.start_ms, None);

    let inherited = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body begin="1s"><div begin="2s" timeContainer="seq"><p dur="1s">one</p><p begin="500ms" dur="1s">two</p></div></body></tt>"#;
    let parsed = parse_ttml(
        inherited.as_bytes(),
        SourceInfo::new("inherited.ttml"),
        &SubtitleOptions::default(),
    );
    assert_eq!(
        parsed.status,
        OperationStatus::Complete,
        "{:?}",
        parsed.diagnostics
    );
    let document = parsed.payload().unwrap();
    assert_eq!(document.cues[0].timing.start_ms, Some(3_000));
    assert_eq!(document.cues[0].timing.end_ms, Some(4_000));
    assert_eq!(document.cues[1].timing.start_ms, Some(4_500));
    assert_eq!(document.cues[1].timing.end_ms, Some(5_500));

    for malformed in [
        "<root><p begin=\"1s\" dur=\"1s\">not TTML</p></root>",
        "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><p begin=\"1s\" dur=\"1s\"></body></tt>",
        "<tt xmlns=\"http://www.w3.org/ns/ttml\"><body><p begin=\"01:00:00:99999999999999999999999999999999999999\" dur=\"1s\">overflow</p></body></tt>",
    ] {
        let envelope = parse_ttml(
            malformed.as_bytes(),
            SourceInfo::new("malformed.ttml"),
            &SubtitleOptions::default(),
        );
        assert_eq!(envelope.status, OperationStatus::Partial);
    }

    let registry = builtin_parser_registry().unwrap();
    let generic_xml = br#"<?xml version="1.0"?><root><body>mentions ttml</body></root>"#;
    let detected = detect_with_registry(
        std::path::Path::new("extensionless"),
        generic_xml,
        None,
        None,
        &grist::core::Limits::default(),
        &registry,
        &DetectionOptions::default(),
    )
    .unwrap();
    assert_ne!(detected.content_kind, ContentKind::Ttml);
}

#[test]
fn source_order_time_projection_regions_and_run_locators_are_distinct() {
    let document = parse_srt(
        SRT.as_bytes(),
        SourceInfo::new("order.srt"),
        &SubtitleOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(document.cues[0].source_index, 0);
    assert_eq!(document.cues[0].timing.start_ms, Some(3_000));
    assert_eq!(document.transcript.entries[0].source_index, 1);
    assert_eq!(document.transcript.entries[0].start_ms, 1_000);

    let nested_vtt =
        "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Ada>one <c.red>two</c> three</v>\n";
    let document = parse_webvtt(
        nested_vtt.as_bytes(),
        SourceInfo::new("runs.vtt"),
        &SubtitleOptions::default(),
    )
    .payload
    .unwrap();
    let ranges = document.cues[0]
        .runs
        .iter()
        .filter_map(|run| run.locator.components()[0].as_text_range())
        .map(|range| (range.byte_start, range.byte_end))
        .collect::<std::collections::BTreeSet<_>>();
    assert!(ranges.len() >= 3);
    assert_eq!(document.cues[0].speaker.as_deref(), Some("Ada"));

    let tied = "2\n00:00:01,000 --> 00:00:02,000\nfirst in source\n\n1\n00:00:01,000 --> 00:00:02,000\nsecond in source\n";
    let document = parse_srt(
        tied.as_bytes(),
        SourceInfo::new("ties.srt"),
        &SubtitleOptions::default(),
    )
    .payload
    .unwrap();
    assert_eq!(
        document
            .transcript
            .entries
            .iter()
            .map(|entry| entry.source_index)
            .collect::<Vec<_>>(),
        [0, 1]
    );
}

#[test]
fn ttml_time_parameters_vtt_order_and_malformed_graph_cues_fail_closed() {
    let parameterized = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:frameRate="30" ttp:frameRateMultiplier="1000 1001" ttp:subFrameRate="2"><body><p begin="00:00:00:15.1" dur="1s">subframe</p></body></tt>"#;
    let parsed = parse_ttml(
        parameterized.as_bytes(),
        SourceInfo::new("subframe.ttml"),
        &SubtitleOptions::default(),
    );
    assert_eq!(
        parsed.status,
        OperationStatus::Complete,
        "{:?}",
        parsed.diagnostics
    );
    assert_eq!(parsed.payload().unwrap().cues[0].timing.start_ms, Some(517));

    for malformed in [
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:frameRate="0"><body><p begin="1f" dur="1s">invalid rate</p></body></tt>"#,
        r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:dropMode="dropNTSC"><body><p begin="00:00:00:15" dur="1s">drop frame</p></body></tt>"#,
    ] {
        let parsed = parse_ttml(
            malformed.as_bytes(),
            SourceInfo::new("time-parameters.ttml"),
            &SubtitleOptions::default(),
        );
        assert_eq!(parsed.status, OperationStatus::Partial);
        assert_eq!(parsed.payload().unwrap().cues[0].timing.start_ms, None);
    }

    let late_style =
        "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\ncue\n\nSTYLE\n::cue { color: lime; }\n";
    let parsed = parse_webvtt(
        late_style.as_bytes(),
        SourceInfo::new("late-style.vtt"),
        &SubtitleOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Partial);
    assert_eq!(parsed.payload().unwrap().styles.len(), 1);

    let malformed_srt = "1\n00:00:01.000 --> 00:00:02.000\nretained\n";
    let parsed = parse_srt(
        malformed_srt.as_bytes(),
        SourceInfo::new("malformed-clock.srt"),
        &SubtitleOptions::default(),
    );
    let document = parsed.payload().unwrap();
    assert!(document.transcript.entries.is_empty());
    let graph = document
        .to_document_graph(DocumentGraphContext::new("subtitle:malformed"))
        .unwrap();
    assert_eq!(
        graph
            .nodes
            .iter()
            .filter(|node| node.kind == DocumentNodeKind::Cue)
            .count(),
        1
    );
}

#[test]
fn ttml_namespaces_and_time_base_are_enforced_without_losing_foreign_facts() {
    let namespaced = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:x="urn:foreign"><body><x:p begin="9s" dur="1s">foreign cue</x:p><p x:begin="8s" begin="1s" dur="1s">native cue</p><p x:begin="7s" dur="1s">foreign timing</p><p begin="3s" dur="1s">safe<x:span>foreign<![CDATA[cdata]]></x:span></p></body></tt>"#;
    let parsed = parse_ttml(
        namespaced.as_bytes(),
        SourceInfo::new("namespaces.ttml"),
        &SubtitleOptions::default(),
    );
    assert_eq!(
        parsed.status,
        OperationStatus::Complete,
        "{:?}",
        parsed.diagnostics
    );
    let document = parsed.payload().unwrap();
    assert_eq!(document.cues.len(), 3);
    assert_eq!(document.cues[0].text, "native cue");
    assert_eq!(document.cues[0].timing.start_ms, Some(1_000));
    assert_eq!(document.cues[1].text, "foreign timing");
    assert_eq!(document.cues[1].timing.start_ms, Some(0));
    assert_eq!(document.cues[1].settings["x:begin"], "7s");
    assert_eq!(document.cues[2].text, "safe");
    assert!(document.decoded_text.contains("foreign cue"));
    assert!(document.decoded_text.contains("<![CDATA[cdata]]>"));

    let media = r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:timeBase="media"><body><p begin="1s" dur="1s">media</p></body></tt>"#;
    let parsed = parse_ttml(
        media.as_bytes(),
        SourceInfo::new("media-time-base.ttml"),
        &SubtitleOptions::default(),
    );
    assert_eq!(parsed.status, OperationStatus::Complete);
    assert_eq!(
        parsed.payload().unwrap().cues[0].timing.start_ms,
        Some(1_000)
    );

    for (time_base, code) in [
        ("smpte", "subtitle.ttml.time_base_unsupported"),
        ("clock", "subtitle.ttml.time_base_unsupported"),
        ("future", "subtitle.ttml.time_parameters"),
    ] {
        let input = format!(
            r#"<tt xmlns="http://www.w3.org/ns/ttml" xmlns:ttp="http://www.w3.org/ns/ttml#parameter" ttp:timeBase="{time_base}"><body><p begin="1s" dur="1s">unsupported</p></body></tt>"#
        );
        let parsed = parse_ttml(
            input.as_bytes(),
            SourceInfo::new("unsupported-time-base.ttml"),
            &SubtitleOptions::default(),
        );
        assert_eq!(parsed.status, OperationStatus::Partial);
        assert_eq!(parsed.payload().unwrap().cues[0].timing.start_ms, None);
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == code)
        );
    }
}

fn dispatch_subtitle(
    bytes: Vec<u8>,
    budget: BudgetSelection,
    cancellation: Option<CancellationToken>,
) -> grist::core::Envelope<serde_json::Value> {
    let mut request = ParseRequest::new(
        RequestId::new("subtitle-controlled").unwrap(),
        Input::bytes(bytes),
        SourceInfo::stdin("controlled.srt"),
        budget,
        ProviderSet::none(),
    );
    if let Some(cancellation) = cancellation {
        request = request.with_cancellation(cancellation);
    }
    builtin_parser_registry()
        .unwrap()
        .dispatch("srt", request, None)
        .unwrap()
}

fn controlled_subtitle_memory(
    bytes: &[u8],
    format: SubtitleFormat,
    options: &SubtitleOptions,
    budget: ResourceBudget,
) -> (grist::subtitle::SubtitleEnvelope, u64) {
    let control =
        OperationControl::new(&BudgetSelection::custom(budget), Default::default()).unwrap();
    let envelope = parse_subtitle_with_operation_control(
        bytes,
        SourceInfo::new("memory-controlled.subtitle"),
        format,
        options,
        &control,
    );
    (envelope, control.usage().memory_bytes)
}

#[test]
fn parser_limits_shared_budgets_and_cancellation_are_incremental() {
    let options = SubtitleOptions {
        max_cues: 1,
        ..SubtitleOptions::default()
    };
    let limited = parse_srt(SRT.as_bytes(), SourceInfo::new("limited.srt"), &options);
    assert_eq!(limited.status, OperationStatus::Partial);
    assert_eq!(limited.payload().unwrap().cues.len(), 1);
    assert!(
        limited
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code.as_str() == "subtitle.limit.cues")
    );

    let mut node_budget = ResourceBudget::trusted_unbounded();
    node_budget.max_nodes = Some(2);
    let node_limited = dispatch_subtitle(
        SRT.as_bytes().to_vec(),
        BudgetSelection::custom(node_budget),
        None,
    );
    assert_eq!(node_limited.status, OperationStatus::Failed);
    assert!(
        node_limited
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.budget.nodes.exhausted" })
    );

    let memory_ttml = r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><div><p begin="1s" dur="2s"><span style="loud">retained text run</span></p></div></body></tt>"#;
    let (unbounded, observed_memory) = controlled_subtitle_memory(
        memory_ttml.as_bytes(),
        SubtitleFormat::Ttml,
        &SubtitleOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(unbounded.status, OperationStatus::Complete);
    assert!(observed_memory > memory_ttml.len() as u64 * 5);
    let mut memory_budget = ResourceBudget::trusted_unbounded();
    memory_budget.max_memory_bytes = Some(observed_memory - 1);
    let (memory_limited, _) = controlled_subtitle_memory(
        memory_ttml.as_bytes(),
        SubtitleFormat::Ttml,
        &SubtitleOptions::default(),
        memory_budget,
    );
    assert_eq!(memory_limited.status, OperationStatus::Failed);
    assert!(
        memory_limited.diagnostics.iter().any(|diagnostic| {
            diagnostic.code.as_str() == "grist.budget.memory_bytes.exhausted"
        })
    );

    let short = "1\n00:00:01,000 --> 00:00:02,000\nretained";
    let capped = format!("{short}\n\n2\n00:00:03,000 --> 00:00:04,000\nignored");
    let cap_options = SubtitleOptions {
        max_cues: 1,
        ..SubtitleOptions::default()
    };
    let (_, short_memory) = controlled_subtitle_memory(
        short.as_bytes(),
        SubtitleFormat::Srt,
        &cap_options,
        ResourceBudget::trusted_unbounded(),
    );
    let (capped_envelope, capped_memory) = controlled_subtitle_memory(
        capped.as_bytes(),
        SubtitleFormat::Srt,
        &cap_options,
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(capped_envelope.status, OperationStatus::Partial);
    let added_source_bytes = (capped.len() - short.len()) as u64;
    assert!(capped_memory > short_memory + added_source_bytes * 5);

    let cancellation = CancellationToken::new();
    let trigger = cancellation.clone();
    let mut large = String::new();
    for index in 0..200_000u64 {
        large.push_str(&format!(
            "{index}\n00:00:01,000 --> 00:00:02,000\ncaption {index}\n\n"
        ));
    }
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(10));
        trigger.cancel();
    });
    let cancelled = dispatch_subtitle(
        large.into_bytes(),
        BudgetSelection::Profile(BudgetProfile::TrustedUnboundedV1),
        Some(cancellation),
    );
    canceller.join().unwrap();
    assert_eq!(cancelled.status, OperationStatus::Cancelled);
    assert!(
        cancelled
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.code.as_str() == "grist.operation.cancelled" })
    );
}

#[test]
fn retained_memory_is_preflighted_for_caps_ttml_runs_and_transcript_rendering() {
    for (declaration, styles_capped) in [
        ("STYLE\n::cue { color: lime; }", true),
        ("REGION\nid:bottom\nwidth:80%", false),
    ] {
        let one = format!("WEBVTT\n\n{}", format!("{declaration}\n\n").repeat(2));
        let capped_prefix = format!("WEBVTT\n\n{}", format!("{declaration}\n\n").repeat(12));
        let later_cue = "00:00:01.000 --> 00:00:02.000\nlater work\n";
        let with_later_cue = format!("{capped_prefix}{later_cue}");
        let mut options = SubtitleOptions::default();
        if styles_capped {
            options.max_styles = 1;
        } else {
            options.max_regions = 1;
        }
        let (_, one_memory) = controlled_subtitle_memory(
            one.as_bytes(),
            SubtitleFormat::WebVtt,
            &options,
            ResourceBudget::trusted_unbounded(),
        );
        let (capped_envelope, capped_memory) = controlled_subtitle_memory(
            capped_prefix.as_bytes(),
            SubtitleFormat::WebVtt,
            &options,
            ResourceBudget::trusted_unbounded(),
        );
        assert_eq!(capped_envelope.status, OperationStatus::Partial);
        let source_only_floor = one_memory + ((capped_prefix.len() - one.len()) as u64 * 5);
        assert!(capped_memory > source_only_floor);

        let control_base = format!("WEBVTT \n\n{declaration}\n\n");
        let padding = "x".repeat(capped_prefix.len() - control_base.len());
        let control_prefix = format!("WEBVTT {padding}\n\n{declaration}\n\n");
        let control_with_later_cue = format!("{control_prefix}{later_cue}");
        assert_eq!(control_with_later_cue.len(), with_later_cue.len());

        let run_limited = |input: &str, memory: u64| {
            let mut budget = ResourceBudget::trusted_unbounded();
            budget.max_memory_bytes = Some(memory);
            // Track + first accepted declaration consume both nodes. If cap
            // diagnostics were bulk-accounted after parsing, the later cue
            // would reach its run allocation and fail this node limit first.
            budget.max_nodes = Some(2);
            controlled_subtitle_memory(input.as_bytes(), SubtitleFormat::WebVtt, &options, budget).0
        };
        let has_code = |envelope: &grist::subtitle::SubtitleEnvelope, code: &str| {
            envelope
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code.as_str() == code)
        };

        // Find the first memory limit at which an equal-length control reaches
        // the later cue's node failure. The capped input must instead exhaust
        // memory while accumulating its earlier declarations' diagnostics.
        let mut low = 0u64;
        let mut high = capped_memory.saturating_mul(2);
        assert!(has_code(
            &run_limited(&control_with_later_cue, high),
            "grist.budget.nodes.exhausted"
        ));
        while low + 1 < high {
            let middle = low + (high - low) / 2;
            if has_code(
                &run_limited(&control_with_later_cue, middle),
                "grist.budget.nodes.exhausted",
            ) {
                high = middle;
            } else {
                low = middle;
            }
        }
        let limited = run_limited(&with_later_cue, high);
        assert_eq!(limited.status, OperationStatus::Failed);
        assert!(
            has_code(&limited, "grist.budget.memory_bytes.exhausted"),
            "control_limit={high} diagnostics={:?}",
            limited.diagnostics
        );
        assert!(!has_code(&limited, "grist.budget.nodes.exhausted"));
    }

    let ttml_wrapper = |text: &str| {
        format!(
            r#"<tt xmlns="http://www.w3.org/ns/ttml"><body><p begin="1s" dur="1s"><span>{text}</span></p></body></tt>"#
        )
    };
    let retained_text = "x".repeat(16 * 1024);
    let empty_ttml = ttml_wrapper("");
    let long_ttml = ttml_wrapper(&retained_text);
    let (_, empty_memory) = controlled_subtitle_memory(
        empty_ttml.as_bytes(),
        SubtitleFormat::Ttml,
        &SubtitleOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    let (long_envelope, long_memory) = controlled_subtitle_memory(
        long_ttml.as_bytes(),
        SubtitleFormat::Ttml,
        &SubtitleOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(long_envelope.status, OperationStatus::Complete);
    let old_single_copy_ceiling = empty_memory + retained_text.len() as u64 * 10;
    assert!(long_memory > old_single_copy_ceiling);
    let mut ttml_budget = ResourceBudget::trusted_unbounded();
    ttml_budget.max_memory_bytes = Some(old_single_copy_ceiling);
    let (limited_ttml, _) = controlled_subtitle_memory(
        long_ttml.as_bytes(),
        SubtitleFormat::Ttml,
        &SubtitleOptions::default(),
        ttml_budget,
    );
    assert_eq!(limited_ttml.status, OperationStatus::Failed);

    let srt_wrapper = |text: &str| format!("1\n00:00:01,000 --> 00:00:02,000\n{text}\n");
    let empty_srt = srt_wrapper("");
    let long_srt = srt_wrapper(&retained_text);
    let (_, empty_memory) = controlled_subtitle_memory(
        empty_srt.as_bytes(),
        SubtitleFormat::Srt,
        &SubtitleOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    let (long_envelope, long_memory) = controlled_subtitle_memory(
        long_srt.as_bytes(),
        SubtitleFormat::Srt,
        &SubtitleOptions::default(),
        ResourceBudget::trusted_unbounded(),
    );
    assert_eq!(long_envelope.status, OperationStatus::Complete);
    let no_render_preflight_ceiling = empty_memory + retained_text.len() as u64 * 10;
    assert!(long_memory > no_render_preflight_ceiling);
    let mut transcript_budget = ResourceBudget::trusted_unbounded();
    transcript_budget.max_memory_bytes = Some(no_render_preflight_ceiling);
    let (limited_transcript, _) = controlled_subtitle_memory(
        long_srt.as_bytes(),
        SubtitleFormat::Srt,
        &SubtitleOptions::default(),
        transcript_budget,
    );
    assert_eq!(limited_transcript.status, OperationStatus::Failed);
}

#[cfg(feature = "cli")]
#[test]
fn cli_dispatch_projects_subtitle_payload_to_graph() {
    let envelope = grist::cli::parse_bytes(
        "webvtt",
        VTT.as_bytes().to_vec(),
        SourceInfo::stdin("captions.vtt"),
        grist::core::RequestId::new("cli-subtitle-round-trip").unwrap(),
        None,
    )
    .unwrap();
    assert_eq!(envelope.status, OperationStatus::Complete);
    let graph = grist::cli::project_envelope_to_graph(&envelope, "subtitle:cli").unwrap();
    assert_eq!(graph.kind, grist::document_graph::DocumentKind::Subtitle);
    assert!(
        graph
            .nodes
            .iter()
            .any(|node| node.kind == DocumentNodeKind::Cue)
    );
}
