//! Escaping helpers for active-capable render targets.

pub fn escape_active_html(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub fn sanitize_link_destination(destination: &str) -> String {
    let compact = destination
        .chars()
        .filter(|character| !character.is_ascii_control() && !character.is_ascii_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.starts_with("javascript:")
        || compact.starts_with("vbscript:")
        || compact.starts_with("data:text/html")
        || compact.starts_with("data:image/svg+xml")
        || compact.starts_with("file:")
    {
        "#grist-blocked-active-uri".into()
    } else {
        destination.replace(')', "%29").replace('(', "%28")
    }
}

/// Use a fence longer than every run of backticks in the input.
pub fn inert_markdown_code(text: &str) -> String {
    let fence = "`".repeat(longest_run(text, '`').saturating_add(1).max(3));
    format!("{fence}text\n{text}\n{fence}\n")
}

/// Escape once, without recursively escaping replacement strings.
pub fn escape_latex_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\\' => escaped.push_str("\\textbackslash{}"),
            '&' => escaped.push_str("\\&"),
            '%' => escaped.push_str("\\%"),
            '$' => escaped.push_str("\\$"),
            '#' => escaped.push_str("\\#"),
            '_' => escaped.push_str("\\_"),
            '{' => escaped.push_str("\\{"),
            '}' => escaped.push_str("\\}"),
            '~' => escaped.push_str("\\textasciitilde{}"),
            '^' => escaped.push_str("\\textasciicircum{}"),
            '\n' => escaped.push_str("\\\\\n"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub fn inert_latex_literal(text: &str) -> String {
    format!("\\texttt{{{}}}", escape_latex_text(text))
}

fn longest_run(text: &str, needle: char) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for character in text.chars() {
        if character == needle {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}
