# Rich Text Format

The `rtf` feature implements a built-in, byte-oriented RTF 1.x parser. It accepts
the `application/rtf` and `text/rtf` media types and `.rtf` files. Detection uses
the `{\rtf` signature and does not depend on an extension.

The authoritative payload retains every group, control word/control symbol,
literal byte sequence, binary run, source spelling, and unknown construct. Group
state is scoped through arbitrary nesting. Missing closing braces are recovered
at end of input, unmatched braces and truncated escapes/binary runs are retained,
and each repair makes the envelope `partial` with an exact byte locator.

The semantic view includes font/color/style tables, list definitions and
overrides, formatted paragraphs and runs, table rows/cells (including nesting
depth), fields and hyperlink targets, pictures, OLE/object data, tracked
insertions/deletions, annotations, and original/accepted/rejected text views.
ANSI code pages are decoded with `encoding_rs`; `\uN` and `\ucN` fallback rules
include UTF-16 surrogate pairs. Unsupported or invalid encodings produce explicit
partial diagnostics while the original bytes remain in the token tree.

Picture and object payloads accept hexadecimal and `\binN` forms. They are
inventoried as content-addressed `EmbeddedArtifact` records by default, or held
inline only when explicitly requested. Safety classification and quarantine are
shared with other container parsers. Nothing is executed, activated, fetched, or
materialized to a caller-visible path.

Every authoritative node carries a half-open raw byte range with one-based line
and column positions. The `DocumentGraph` projection emits paragraphs/runs,
lists, tables, fields, images, attachments, revisions, comments, explicit links,
and raw unknown controls. Standard graph rendering and deterministic segmentation
therefore retain citation locators. Public schemas are `grist/rtf/v1`,
`grist/envelope/v2`, and `grist/rtf-options/v1`.
