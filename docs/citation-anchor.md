# Citation anchors and source-version verification

The core citation contract is versioned as "grist/citation-anchor/v1". An anchor is
an immutable claim about where extracted content appeared in one source version. It does
not claim that the content is true, relevant, or sufficient evidence.

## Anchor identity

Every anchor records:

- a node or segment target ID and the ordered contributing graph node IDs;
- caller-supplied "SourceInfo" and the complete "ContentIdentity";
- a source-content hash selected in the order raw bytes, compound aggregate, then decoded
  UTF-8 text; canonical payload hashes are deliberately not treated as source hashes;
- one or more exact, approximate, or synthetic "SourceLocator" values;
- a normalized-text hash, deterministic label, and optional bounded excerpt.

Node anchors contain one node ID and locator. Segment anchors retain their stable segment
ID, ordered source node IDs, and every disjoint locator. This makes the citation boundary
usable by the segmentation engine without making segmentation part of the core contract.

"grist/citation-text/v1" collapses every Unicode whitespace run to one ASCII
space and trims both ends. It performs no case folding or compatibility normalization.
The SHA-256 digest covers exactly the normalized UTF-8 bytes.

Excerpts use the same normalized text. Callers may request zero through 512 Unicode
scalar values; zero omits the excerpt and longer text ends in one ellipsis within that
limit. Excerpts are display aids, never matching material.

Labels are derived from structural context where available ("§ Methods") and
otherwise from the innermost locator ("page 14", "slide 7",
"Sheet1!B4:D9", or a line/range label). Approximate coordinates are visibly
marked approximate.

"DocumentGraph::citation_anchor_for_node" rejects nodes without a locator.
"DocumentGraph::citation_anchors" emits an anchor for every locator-addressable
node in graph order, while "citation_source_version" creates the corresponding
verification index.

## Verification

Verification consumes an anchor and a caller-supplied "CitationSourceVersion";
it never fetches a URI and never mutates the anchor. Results use
"grist/citation-verification/v1" and always preserve the original locators.
When a current target is identified, its current locators and target ID are returned
separately.

The outcomes are:

- "exact": the entire source hash matches, or stable identity/location and
  normalized text still match;
- "relocated": matching content has moved, proven by stable target identity or
  one unique bounded content match;
- "changed": a stable target or the original locator remains, but its normalized
  content hash differs;
- "missing": no stable identity, original locator, or content match exists;
- "unverifiable": source identity is unavailable, the candidate bound was
  exceeded, or locator/content matching is ambiguous.

After a changed source hash, verification checks stable target/node identity first,
then the original locator, then normalized content. Content relocation requires exactly
one match. Duplicate content is "unverifiable", not arbitrarily retargeted.
Candidate traversal is bounded by "CitationVerificationOptions::max_candidates"
(10,000 by default); exceeding the bound is also "unverifiable", not
"missing".

Checked schemas are available as "citation-anchor",
"citation-source-version", and "citation-verification".
