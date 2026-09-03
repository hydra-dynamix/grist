#![cfg(all(feature = "model-output", feature = "structured-binary"))]

use grist::core::{Limits, SourceInfo};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

#[test]
fn checked_fuzz_seeds_never_escape_public_parser_boundaries() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/universal_bytes");
    let mut seeds = fs::read_dir(&corpus)
        .expect("fuzz corpus")
        .map(|entry| entry.expect("corpus entry").path())
        .collect::<Vec<_>>();
    seeds.sort();
    assert!(seeds.len() >= 3, "universal fuzz corpus was weakened");

    for path in seeds {
        let bytes = fs::read(&path).expect("fuzz seed bytes");
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            let _ = grist::detect::detect_path(Path::new(&name), &bytes, &Limits::default());
            let _ = grist::text::parse_text_bytes(
                &bytes,
                SourceInfo::stdin(name.clone()),
                &grist::text::TextOptions::default(),
            );
            let _ = grist::structured_binary::parse_cbor(&bytes, SourceInfo::stdin(name.clone()));
            let _ = grist::structured_binary::parse_messagepack(
                &bytes,
                SourceInfo::stdin(name.clone()),
            );
            if let Ok(text) = std::str::from_utf8(&bytes) {
                let _ = grist::model_output::parse_model_output(
                    text,
                    SourceInfo::stdin(name.clone()),
                    &grist::model_output::ModelOutputOptions::default(),
                );
            }
        }));
        assert!(
            outcome.is_ok(),
            "public parser panic for {}",
            path.display()
        );
    }
}
