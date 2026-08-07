use std::collections::{BTreeMap, BTreeSet};

const CONTRACT: &str = include_str!("../docs/complete-parser-contract.md");

#[test]
fn normative_index_accounts_for_every_modal_section_and_owner() {
    let profiles: BTreeSet<_> = CONTRACT
        .lines()
        .filter(|line| line.starts_with('|') && !line.starts_with("|---"))
        .filter_map(|line| {
            let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
            (cells.len() == 6 && cells[0].chars().all(|c| c.is_ascii_uppercase()))
                .then_some(cells[0])
        })
        .collect();
    assert!(profiles.len() >= 20, "ownership profiles were lost");

    let mut shall = 0_u32;
    let mut should = 0_u32;
    let mut may = 0_u32;
    let mut sections = BTreeSet::new();
    let mut status_counts = BTreeMap::new();
    let mut rows = 0_usize;

    for line in CONTRACT.lines().filter(|line| line.starts_with("| CP-")) {
        let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
        assert_eq!(cells.len(), 7, "malformed normative row: {line}");
        assert!(
            cells.iter().all(|cell| !cell.is_empty()),
            "empty cell: {line}"
        );
        assert!(
            profiles.contains(cells[4]),
            "unknown ownership profile: {line}"
        );
        assert!(
            cells[5].contains('-'),
            "owner must be an LDGR work slug: {line}"
        );

        let section = cells[1]
            .strip_prefix('§')
            .expect("reference must start with §")
            .split(['.', '-'])
            .next()
            .unwrap()
            .parse::<u8>()
            .expect("numeric section");
        sections.insert(section);
        shall += cells[2].parse::<u32>().expect("numeric SHALL count");
        for modal in cells[3].split(';').map(str::trim) {
            if let Some(n) = modal.strip_prefix("SHOULD") {
                should += n.parse::<u32>().expect("numeric SHOULD count");
            } else if let Some(n) = modal.strip_prefix("MAY") {
                may += n.parse::<u32>().expect("numeric MAY count");
            } else {
                assert_eq!(modal, "0", "unknown advisory modal: {line}");
            }
        }
        let status = ["**conforming**", "**partial**", "**absent**"]
            .into_iter()
            .find(|status| cells[6].contains(status))
            .unwrap_or_else(|| panic!("missing strict status: {line}"));
        *status_counts.entry(status).or_insert(0_usize) += 1;
        rows += 1;
    }

    assert_eq!((shall, should, may), (155, 3, 11));
    assert_eq!(sections, (1_u8..=19).collect());
    assert!(rows >= 100, "normative clauses were over-consolidated");
    assert!(status_counts["**conforming**"] > 0);
    assert!(status_counts["**partial**"] > 0);
    assert!(status_counts["**absent**"] > 0);
}

#[test]
fn every_required_format_and_gate_is_explicit() {
    let tokens = [
        "Plain text",
        "Markdown CommonMark",
        "reStructuredText",
        "AsciiDoc",
        "HTML5",
        "XHTML",
        "JATS XML",
        "EPUB 2 and EPUB 3",
        "LaTeX projects",
        "BibTeX and BibLaTeX",
        "Born-digital, scanned, hybrid PDF",
        "DOCX, DOCM, DOTX, DOTM",
        "Legacy DOC",
        "WordProcessingML 2003",
        "Flat OPC",
        "ODT and OTT",
        "RTF",
        "PPTX, PPTM, POTX, PPSX",
        "ODP and OTP",
        "Legacy PPT",
        "CSV and TSV",
        "XLSX, XLSM, XLSB",
        "ODS and OTS",
        "Legacy XLS",
        "SpreadsheetML 2003",
        "JSON, JSONL/NDJSON, YAML, TOML",
        "CBOR",
        "MessagePack",
        "Protocol Buffers",
        "Apache Arrow IPC and Parquet",
        "SQLite",
        "EML/RFC 5322 and MIME",
        "MBOX",
        "MSG",
        "PST and OST",
        "TNEF and S/MIME",
        "iCalendar/ICS",
        "vCard/VCF",
        "Jupyter Notebook/IPYNB",
        "R Markdown and Quarto",
        "Rust, Python, JavaScript, TypeScript, TSX, JSX",
        "Go, Java, Kotlin, C, C++",
        "C#, Ruby, PHP, Swift, Bash, SQL, CSS",
        "ZIP, ZIP64, TAR",
        "GZIP, BZIP2, XZ",
        "Zstandard, 7z",
        "PNG, JPEG, TIFF, WebP, GIF, BMP, HEIF/HEIC, SVG",
        "SRT, WebVTT, TTML",
        "Audio/video metadata",
        "OpenAI calls",
        "MCP JSON-RPC",
    ];
    for token in tokens {
        assert!(CONTRACT.contains(token), "required format missing: {token}");
    }
    for gate in 1..=11 {
        assert!(
            CONTRACT.contains(&format!("| {gate}.")),
            "gate {gate} missing"
        );
    }
    assert!(CONTRACT.contains("all eleven controls"));
    assert!(CONTRACT.contains("`fuzz/` byte-facing targets/corpora"));
    assert!(CONTRACT.contains("not permission to weaken the target"));
}
