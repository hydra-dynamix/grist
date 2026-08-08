# Native image parser

The `media` feature provides one bounded, inert image payload for PNG, JPEG, TIFF,
WebP, GIF, BMP, HEIF/HEIC, and SVG. Parsing identifies container structure,
dimensions, animation frames, metadata, unknown payloads, diagnostics, and source
locators. It does not decode raster pixels, render SVG, execute scripts or event
handlers, resolve links, or fetch external resources.

## Formats and retained structure

- PNG validates the signature, chunk boundaries, CRCs, IHDR methods, critical
  chunks, text metadata, and APNG frame control.
- JPEG retains marker order, dimensions, comments, application metadata, and a
  single bounded scan while rejecting malformed marker and termination structure.
- TIFF walks bounded IFD chains and retains EXIF, GPS, XMP, IPTC, and unknown tag
  source ranges without duplicating the source bytes.
- WebP validates RIFF and nested animation chunk boundaries and retains frame
  controls and image subchunks.
- GIF retains logical-screen data, extensions, frame rectangles, disposal, delay,
  transparency, and bounded image-data blocks.
- BMP validates its declared file, DIB, palette/pixel offsets, dimensions, planes,
  bit depth, compression mode, and uncompressed pixel extent.
- HEIF/HEIC validates ISO BMFF box extents and associates the `pitm` primary
  item through `iinf`/`infe`, `iloc`, `iref`, and `iprp`/`ipma`. Only properties
  associated with the primary item supply dimensions or orientation, and only
  associated Exif/XMP item extents inside `mdat` become metadata.
- SVG retains ordered XML elements, attributes, text, exact byte ranges, and
  sibling-indexed XML paths.

## Metadata and active content

EXIF/TIFF values, GPS coordinates, XMP, IPTC, textual metadata, and unknown chunks
are retained under independent byte and item limits. GPS latitude and longitude are
derived only when the required reference and rational components are valid.

SVG script elements, event-handler attributes, JavaScript URIs, CSS `url()` and
`@import` references, `href`/`src`/`poster` resources, foreign objects, embedded
media, and SMIL animation or timing attributes are typed inventory records. They
remain source data; the parser has no execution, rendering, or network path.

## Public surfaces and limits

`ImageOptions` controls dimensions, frames, chunks, metadata, unknown retained
bytes, SVG element count, nesting depth, and locator-path size. Shared operation
budgets independently control input and output bytes and parser work.
The registry routes each descriptor only to its matching detected container, and
controlled parsing observes cancellation/parse-time checkpoints while charging
nodes, records, nesting, and decoded characters incrementally. PNG requires IDAT
(and data for every declared APNG frame), JPEG requires an SOS scan, and HEIF
requires primary item/property/data association before reporting completion.
`ImageDocument` projects deterministically to `DocumentGraph` with source locators
for the root and retained children.

The payload, envelope, and options schemas are registered as `image-v1`,
`image-envelope-v1`, and `image-options-v1`. The CLI accepts the registered image
format IDs when built with `media` and `schemas`.

Focused verification uses:

```text
cargo test --locked --no-default-features --features media,schemas,document-graph --test image_universal_contract
```
