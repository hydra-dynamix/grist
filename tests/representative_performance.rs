#![cfg(all(feature = "csv", feature = "model-output"))]

use grist::core::SourceInfo;
use std::time::{Duration, Instant};

const REGRESSION_LIMIT: Duration = Duration::from_secs(20);

fn within_threshold(name: &str, operation: impl FnOnce()) {
    let started = Instant::now();
    operation();
    assert!(
        started.elapsed() <= REGRESSION_LIMIT,
        "{name} exceeded the representative {REGRESSION_LIMIT:?} regression threshold"
    );
}

#[test]
fn representative_text_csv_and_model_output_workloads_stay_bounded() {
    // Keep these fixtures large enough to exercise allocation and source-map
    // paths while remaining suitable for the unoptimized CI test profile.
    let text = "representative text line\n".repeat(1_000);
    within_threshold("plain text", || {
        let envelope = grist::text::parse_text_bytes(
            text.as_bytes(),
            SourceInfo::stdin("performance.txt"),
            &grist::text::TextOptions::default(),
        );
        assert!(envelope.payload.is_some());
    });

    let csv = (0..2_000)
        .map(|index| format!("row-{index},{index}\n"))
        .fold(String::from("name,value\n"), |mut output, row| {
            output.push_str(&row);
            output
        });
    within_threshold("CSV", || {
        let envelope = grist::csv::parse_csv(
            &csv,
            SourceInfo::stdin("performance.csv"),
            &grist::csv::CsvOptions::default(),
        );
        assert_eq!(envelope.payload.unwrap().rows.len(), 2_000);
    });

    let response = format!(
        "{}{{\"name\":\"representative\",\"arguments\":{{\"rows\":10000}}}}",
        "model prose ".repeat(1_000)
    );
    within_threshold("model output", || {
        let envelope = grist::model_output::parse_model_output(
            &response,
            SourceInfo::stdin("performance-response.txt"),
            &grist::model_output::ModelOutputOptions::default(),
        );
        assert!(envelope.payload.is_some());
    });
}
