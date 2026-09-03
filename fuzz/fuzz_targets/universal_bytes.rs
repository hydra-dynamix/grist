#![no_main]

use grist::core::{Limits, SourceInfo};
use libfuzzer_sys::fuzz_target;
use std::path::Path;

fuzz_target!(|data: &[u8]| {
    let _ = grist::detect::detect_path(Path::new("fuzz.bin"), data, &Limits::default());
    let _ = grist::text::parse_text_bytes(
        data,
        SourceInfo::stdin("fuzz.txt"),
        &grist::text::TextOptions::default(),
    );
    let _ = grist::structured_binary::parse_cbor(data, SourceInfo::stdin("fuzz.cbor"));
    let _ = grist::structured_binary::parse_messagepack(data, SourceInfo::stdin("fuzz.msgpack"));
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = grist::model_output::parse_model_output(
            text,
            SourceInfo::stdin("fuzz-response.txt"),
            &grist::model_output::ModelOutputOptions::default(),
        );
    }
});
