# Media format contract

The `media` feature provides bounded, inert parsing for native images, timed
media containers and subtitles. Image selectors are BMP, GIF, HEIF/HEIC, JPEG,
PNG, SVG, TIFF and WebP. Timed-media selectors are FLAC, Matroska, MP3, MP4,
QuickTime and WAV. Subtitle selectors are SRT, TTML and WebVTT.

Parsers retain container structure, dimensions/duration, tracks, chapters,
metadata, embedded subtitle evidence, unknown ranges and exact locators. SVG
active content and external references remain inert; audio/video codecs are not
executed and remote resources are never fetched. Provider-derived OCR or
transcription is accepted only through the explicit provider contract and is
kept separate from native evidence.

All media payloads have generated schemas and registry-backed detection/CLI
routing. Their normalized graph projections preserve source locators,
provenance and deterministic ordering; structural segmentation consumes only
text-bearing nodes. See `docs/image.md` and the cross-format integration matrix
for selector-level details.
