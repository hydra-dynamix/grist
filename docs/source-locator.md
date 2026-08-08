# Source locator contract

`SourceLocator` is Grist's versioned cross-format coordinate contract
(`grist/source-locator/v1`). It retains a containment chain instead of
flattening every source into a text range.

## Containment

`components` is non-empty and ordered from the outermost source to the
innermost location. Every component is interpreted relative to the component
immediately before it. For example:

```text
archive_member -> email_part -> ooxml_part -> text_range
```

This means the text range addresses decoded text in the OOXML part, the OOXML
part addresses an attachment MIME part, and the MIME part addresses a member of
the archive. Reordering or removing a parent changes the locator's meaning.

The tagged component set is:

- `byte_range`, `text_range`, `pdf_region`, `ooxml_part`, `slide_region`;
- `sheet_range`, `notebook_cell`, `email_part`, `archive_member`;
- `image_region`, `media_time`, `record_range`;
- `json_pointer` and `xml_path`.

## Ranges and index bases

Binary byte ranges are zero-based, half-open offsets relative to their parent
component (or the source when outermost). They do not imply a character
encoding. Text byte offsets are zero-based offsets into UTF-8 and form a half-open
`byte_start..byte_end` interval. Human line and column positions are one-based
and half-open. Columns count Unicode scalar values, so a multibyte scalar
advances the byte offset by its UTF-8 width but the column by one.

`IndexPosition`, `IndexRange`, and `CellAddress` always serialize their
`base` as `zero` or `one`. One-based zero is invalid. Public
human-facing page, slide, sheet-cell, and frame constructors use one-based
indexes; machine-native notebook, archive, token, track, and record indexes may
use zero-based indexes when that is the source format's convention.

`IndexRange`, text, media time, token, and record ranges are half-open.
`SheetRange` is an inclusive rectangle from `start_cell` through
`end_cell`; both cells must use the same base.

Bounding boxes declare points, pixels, or normalized units and a top-left or
bottom-left origin. Normalized rectangles must fit within the unit square.
PDF rotation is normalized modulo 360 and must be a multiple of 90 degrees.
JSON Pointer uses RFC 6901 string syntax; XML paths are absolute.

## Precision and derivation

Precision is a tagged contract, not an optional note:

- `exact` identifies source-native coordinates;
- `approximate` always includes a finite confidence in `[0, 1]`;
- `synthetic` always includes non-empty source node IDs and derivation steps.

Exact and approximate locators may also carry `derived_from` when a derived
representation retains a precise or confidence-bearing source map. Synthetic
locators cannot be deserialized without their source-node and derivation-step
references. The Rust constructors and deserializer validate the same
invariants.

`SourceRange` remains the parser-specific text compatibility type and converts
losslessly into a `text_range` component. Downstream versioned contracts use
`SourceLocator` when they migrate from text-only coordinates.
