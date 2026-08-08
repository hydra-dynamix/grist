use super::Signal;
use crate::core::DetectionEvidenceKind;
use crate::registry::ParserRegistry;

pub(super) fn signals(text: &str, registry: &ParserRegistry) -> Vec<Signal> {
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
    #[cfg(feature = "javascript")]
    if javascript_markers(text) && !typescript_markers(text) && !jsx_markers(text) {
        if let Some(signal) = probe_javascript(text) {
            signals.push(signal);
        }
    }
    #[cfg(feature = "javascript")]
    if jsx_markers(text) && !typescript_markers(text) {
        if let Some(signal) = probe_jsx(text) {
            signals.push(signal);
        }
    }
    #[cfg(feature = "typescript")]
    if typescript_markers(text) && !jsx_markers(text) {
        if let Some(signal) = probe_typescript(text) {
            signals.push(signal);
        }
    }
    #[cfg(feature = "typescript")]
    if typescript_markers(text) && jsx_markers(text) {
        if let Some(signal) = probe_tsx(text) {
            signals.push(signal);
        }
    }
    for probe in registry.grammar_probes(text) {
        if let Some(signal) = probe_signal(
            &probe.format,
            probe.media_type.as_deref().unwrap_or("text/plain"),
            &probe.parser_id,
            probe.has_error,
            probe.named_children,
        ) {
            signals.push(signal);
        }
    }
    signals
}
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

#[cfg(feature = "javascript")]
fn probe_javascript(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "javascript",
        "text/javascript",
        "tree-sitter-javascript",
        root.has_error(),
        root.named_child_count(),
    )
}

#[cfg(feature = "javascript")]
fn probe_jsx(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "jsx",
        "text/jsx",
        "tree-sitter-javascript",
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

#[cfg(feature = "typescript")]
fn probe_tsx(text: &str) -> Option<Signal> {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
        .ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    probe_signal(
        "tsx",
        "text/tsx",
        "tree-sitter-tsx",
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

#[cfg(feature = "javascript")]
fn javascript_markers(text: &str) -> bool {
    [
        "const ",
        "let ",
        "function ",
        "class ",
        "export ",
        "import ",
        "=>",
        "require(",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(any(feature = "javascript", feature = "typescript"))]
fn typescript_markers(text: &str) -> bool {
    [
        "interface ",
        "type ",
        "enum ",
        "namespace ",
        "declare ",
        " implements ",
        " satisfies ",
        ": string",
        ": number",
        ": boolean",
        ": unknown",
        ": never",
        ": any",
        " as const",
        " = ",
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

#[cfg(feature = "javascript")]
fn jsx_markers(text: &str) -> bool {
    ["</", "/>", "return <", "=> <", "=<"]
        .iter()
        .any(|marker| text.contains(marker))
}
