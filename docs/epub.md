# EPUB 2 and EPUB 3 contract

The `epub` feature exposes the authoritative `grist/epub/v1` payload for EPUB
2 and EPUB 3 packages. The registry names are `epub` and `application/epub+zip`;
the parser accepts bytes and never extracts archive members to the filesystem.

## Package and content model

The parser validates the required stored `mimetype` member, reads
`META-INF/container.xml`, selects the declared OPF rootfile, and retains package
version, unique-identifier binding, ordered metadata, manifest properties,
fallback/media-overlay labels, spine order and linearity, and exact nested
archive-member plus XML-path locators.

EPUB 3 navigation documents retain toc, landmarks, page-list, and other nav
groups. EPUB 2 NCX navigation retains nav-point order, labels, play order, and
destinations. XHTML and HTML manifest resources become ordered chapters through
the existing HTML/XHTML parser. Footnotes and noterefs, image usages and alt
text, CSS links and `url()` references, font/style resources, and other embedded
manifest resources remain typed. References are normalized only inside the
package; fragments are retained independently.

Every manifest member has an embedded-artifact record with its own raw-byte
content identity, a parent link to the package identity, media type, role, and
archive-member locator. `inline_resource_bytes` controls byte retention without
changing inventory or identity. `retain_non_spine_documents` controls whether
non-spine HTML resources are parsed as chapters.

## Status, security, and malformed input

ZIP central-directory entries are preflighted before decompression. Absolute
paths, traversal, unsafe names, duplicate normalized names, links and special
files are rejected by the shared archive policy. Member count, child artifact,
expanded byte, memory, and compression-ratio limits use the request resource
budget. No script, stylesheet, font, image, link, external entity, network
resource, or executable is run or fetched.

An encrypted spine member returns `encrypted`, with an `epub.encrypted`
diagnostic distinct from malformed, unsupported, security-rejected, and
budget-exceeded input. Recoverable missing resources, broken package references,
malformed XML/XHTML, and rejected optional members retain partial payloads and
specific nested diagnostics. Invalid XML bytes are retained with replacement
text and an explicit partial diagnostic.

## Graph, segmentation, schemas, and evidence

`ToDocumentGraph` maps package metadata, manifest resources, chapters, XHTML
structure, navigation links, spine precedence, footnote relations, and image
embedding to deterministic nodes and typed edges. Parser-native details remain
under `grist.epub`; unknown chapter markup carries raw node content. Shared
segmentation preserves nested archive/member locators and graph identities.

Checked schemas are `grist.epub.v1.schema.json`,
`grist.epub-envelope.v2.schema.json`, and
`grist.epub-options.v1.schema.json`. Focused EPUB 2/3, adversarial archive,
budget, encryption, malformed input, graph, segmentation, schema, determinism,
and CLI coverage lives in `tests/epub_universal_contract.rs`.
