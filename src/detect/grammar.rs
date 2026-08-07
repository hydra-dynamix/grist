use super::Signal;
#[cfg(any(feature = "rust", feature = "python", feature = "typescript"))]
use crate::core::DetectionEvidenceKind;

#[cfg(any(feature = "rust", feature = "python", feature = "typescript"))]
pub(super) fn signals(text: &str) -> Vec<Signal> {
    let mut signals = Vec::new();
    #[cfg(feature = "rust")]
    if rust_markers(text) {
        if let Some(signal) = probe_rust(text) {
            signals.push(signal);
        }
    }
    #[cfg(feature = "python")]
    if python_markers(text) {
        if let Some(signal) = probe_python(text) {
            signals.push(signal);
        }
    }
    #[cfg(feature = "typescript")]
    if typescript_markers(text) {
        if let Some(signal) = probe_typescript(text) {
            signals.push(signal);
        }
    }
    signals
}

#[cfg(not(any(feature = "rust", feature = "python", feature = "typescript")))]
pub(super) fn signals(_text: &str) -> Vec<Signal> {
    Vec::new()
}

#[cfg(any(feature = "rust", feature = "python", feature = "typescript"))]
fn probe_signal(
    format: &str,
    media: &str,
    parser: &str,
    has_error: bool,
    named_children: usize,
) -> Option<Signal> {
    if named_children == 0 {
        return None;
    }
    let weight = if has_error { 0.36 } else { 0.72 };
    Some(Signal::new(
        format,
        Some(media),
        weight,
        DetectionEvidenceKind::GrammarProbe,
        format!(
            "{parser} grammar probe produced {named_children} top-level nodes{}",
            if has_error {
                " with recovery"
            } else {
                " without errors"
            }
        ),
    ))
}

#[cfg(feature = "rust")]
fn probe_rust(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "rust",
        "text/x-rust",
        "tree-sitter-rust",
        root.has_error(),
        root.named_child_count(),
    )
}

#[cfg(feature = "python")]
fn probe_python(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "python",
        "text/x-python",
        "tree-sitter-python",
        root.has_error(),
        root.named_child_count(),
    )
}

#[cfg(feature = "typescript")]
fn probe_typescript(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "typescript",
        "text/typescript",
        "tree-sitter-typescript",
        root.has_error(),
        root.named_child_count(),
    )
}

#[cfg(feature = "rust")]
fn rust_markers(text: &str) -> bool {
    [
        "fn ", "pub ", "impl ", "struct ", "enum ", "use ", "let mut ", "::",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "python")]
fn python_markers(text: &str) -> bool {
    ["def ", "class ", "import ", "from ", "print(", " = "]
        .iter()
        .any(|marker| text.contains(marker))
}

#[cfg(feature = "typescript")]
fn typescript_markers(text: &str) -> bool {
    [
        "interface ",
        "type ",
        "const ",
        "let ",
        "function ",
        "export ",
        "import ",
        "=>",
        " = ",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}
