# Ranked format detection

Grist detection is an evidence-ranking operation, not an extension lookup.
The public detect_with_registry and detect_source entry points combine bounded
probes for:

- byte and container signatures;
- caller-declared media types and format hints;
- extensions and special manifest/lockfile names;
- ZIP central-directory package structure and stored mimetype manifests;
- UTF-8, UTF-16, and UTF-32 BOM/charset evidence;
- lightweight JSON, JSONL, HTML, XML, CSV, LaTeX, YAML, TOML, and Markdown
  structure;
- Python, JavaScript, and shell shebangs; and
- enabled tree-sitter Rust, Python, and TypeScript grammar probes.

Every candidate retains typed evidence, confidence, one-based rank, canonical
format/media identity, active-registry parser availability, and selected
parser ID when available. Ordering is confidence descending, then canonical
format identity and evidence ordering. Confidence is rounded to four decimal
places so repeated runs and serialization are stable.

Magic and package-manifest evidence is decisive. When present, candidates
based only on extensions, caller labels, or text heuristics are capped below
the decisive candidate. Contradictory evidence remains visible and produces
detect.contradictory_evidence; it is never discarded or allowed to override
the signature.

## Ambiguity policy

DetectionOptions declares a minimum selectable confidence and the confidence
margin that forms an ambiguity set. The caller chooses one policy:

- return_ambiguous (default) returns no selected format or parser;
- prefer_available selects only when exactly one candidate in the ambiguity
  set has an available parser in the supplied registry; or
- select_highest_ranked selects the deterministic leader.

Invalid thresholds or a zero probe budget return DetectionOptionsError.
Recognized formats with no active parser return unsupported; insufficient
evidence returns unknown. The compatibility detect_path entry point uses the
built-in registry and default policy.

ZIP inspection is bounded to 16,384 central-directory entries and the standard
65,557-byte end-record search window. It does not decompress or execute
package content. OOXML packages are refined from canonical part names, while
EPUB and OpenDocument packages use stored mimetype data or standard package
paths.
