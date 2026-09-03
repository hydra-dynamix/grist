use grist::core::SourceInfo;
use std::hint::black_box;
use std::time::{Duration, Instant};

const REGRESSION_LIMIT: Duration = Duration::from_secs(15);

fn measured(name: &str, operation: impl FnOnce()) {
    let started = Instant::now();
    operation();
    let elapsed = started.elapsed();
    assert!(
        elapsed <= REGRESSION_LIMIT,
        "{name} exceeded the representative {REGRESSION_LIMIT:?} threshold: {elapsed:?}"
    );
    println!("{name}: {elapsed:?}");
}

fn main() {
    let text = "representative text line\n".repeat(4_000);
    measured("plain-text-100kb", || {
        let envelope = grist::text::parse_text_bytes(
            black_box(text.as_bytes()),
            SourceInfo::stdin("representative.txt"),
            &grist::text::TextOptions::default(),
        );
        black_box(envelope);
    });

    let csv = (0..4_000)
        .map(|index| format!("row-{index},{index}\n"))
        .fold(String::from("name,value\n"), |mut output, row| {
            output.push_str(&row);
            output
        });
    measured("csv-4k-records", || {
        let envelope = grist::csv::parse_csv(
            black_box(&csv),
            SourceInfo::stdin("representative.csv"),
            &grist::csv::CsvOptions::default(),
        );
        black_box(envelope);
    });

    let response = format!(
        "{}{{\"name\":\"representative\",\"arguments\":{{\"rows\":20000}}}}",
        "model prose ".repeat(2_000)
    );
    measured("model-output-24kb", || {
        let envelope = grist::model_output::parse_model_output(
            black_box(&response),
            SourceInfo::stdin("representative-response.txt"),
            &grist::model_output::ModelOutputOptions::default(),
        );
        black_box(envelope);
    });
}
