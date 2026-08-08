# Subtitle parsing

The `media` feature provides native, inert parsers for SubRip (`srt`), WebVTT
(`webvtt`), and TTML (`ttml`). Parsers retain the decoded source and original
bytes, cues in source order, a separate stable time-ordered transcript,
standalone subtitle tracks, presentation regions, styles and settings,
speaker-aware text runs, and diagnostics. Cue text, TTML structure, and media
time use separate typed locators so rendering provenance is not replaced by a
derived time coordinate.

SRT and WebVTT clocks are validated with their format-specific separators and
field widths. TTML clock, offset, frame, and tick expressions use checked
integer/rational arithmetic, including frame-rate multipliers and declared
subframe rates. Invalid TTML timing parameters and unsupported drop-frame clock
expressions are retained with partial diagnostics instead of being silently
interpreted. The `media` time base is supported; `clock`, `smpte`, and invalid
time bases fail closed. TTML element and attribute semantics use expanded XML
namespaces, so foreign elements, their descendant text/CDATA, and foreign
timing-like attributes remain raw source facts rather than native cues, cue
text, or timing. TTML element timing inherits
through `par` and `seq` contexts, and the parser requires a recognized
namespaced `tt` root with balanced XML end tags. DTDs, unknown WebVTT markup,
CSS, and external-looking values remain inert; the parser never resolves
entities, fetches resources, or executes style content.

Shared input, decoded-character, record, node, nesting, memory, time, and
cancellation controls are checked during parsing. Retained memory is accounted
cumulatively and before retained pushes/clones across raw and decoded evidence,
TTML state, cues and runs, diagnostics, and temporary plus joined transcript
allocations. Format options additionally cap cues,
styles, tracks, regions, and TTML nesting. Hitting a format cap emits an
explicit partial diagnostic; hitting a shared operation budget or caller
cancellation preserves its terminal status and diagnostic.
