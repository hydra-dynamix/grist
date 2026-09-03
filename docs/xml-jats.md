# XML and JATS core parsing

The `xml` feature exposes `grist::xml` and the `xml`, `jats`, and `nxml` registry selectors. `text-publishing`, `scholarly`, `full`, and `cli` enable it. The authoritative payload schema is `grist/xml/v1` in the v2 envelope.

Parsing uses quick-xml 0.37.5 as an inert pull parser plus an exact lexical and namespace stack. It retains original bytes, decoded text and byte mapping, declarations, document order, element/attribute qualified names, namespace bindings, text, CDATA, comments, processing instructions, doctypes, entity references, and raw source for every node. Every node and attribute has a UTF-8 text range nested with an absolute indexed XML path.

Auto dialect detection identifies an `article` root or JATS namespace as JATS. The JATS core projection extracts article metadata, sections and titles, links and `rid` references, tables with spans, figures/media references, bibliography records, footnotes, and common inline structure. Elements outside the known JATS vocabulary remain exact raw nodes.

For JATS payloads, `scholarly_links` is the authoritative `grist/jats-scholarly-links/v1` semantic view. Every unqualified `id` or `xml:id` target is retained in source order and typed as a bibliography entry, figure, table, supplement, section, footnote, metadata, label, or other target. Labels retain their owning node and source locator. Each `rid` value is split according to JATS IDREFS rules while preserving the complete raw attribute; every token becomes a typed relationship with body/front/back scope and citation, figure, table, supplement, section, footnote, metadata, or other semantics. Resolution is explicitly `resolved`, `unresolved`, or `ambiguous`; duplicate target IDs and empty or missing targets remain in the payload with exact attribute locators and make the parse partial rather than disappearing.

`XmlOptions` selects `auto`, `xml`, or `jats`, an optional declared encoding, and comment retention. Detection combines `.xml`, `.jats`, `.nxml`, XML/JATS media types, declarations, and lightweight root structure. Declared XML encodings use the shared decoder and retain raw-byte mappings.

Security is unconditional. External entity declarations, custom entity references, XInclude, and remote schema locations are inventoried and diagnosed but never resolved. Grist installs no filesystem resolver, schema loader, network callback, or XInclude processor. Predefined and numeric character references are decoded safely while their source entity records remain present.

Malformed input returns a partial payload. Stable diagnostics cover unbound prefixes, duplicate or malformed attributes, mismatched/unclosed elements, unresolved entities, multiple/missing roots, and text outside the root. Recoverable source remains in normal nodes or `raw_unknown` recovery nodes; parsing never fabricates complete status after structural loss.

The `DocumentGraph` projection preserves every authoritative node in source order under `grist.xml` extensions. JATS-known constructs map to normalized section, heading, paragraph, citation, reference, label, table, figure, image, attachment, bibliography, metadata, and footnote kinds; unknown JATS and recovery content use raw nodes. Resolved scholarly edges point to stable projected target nodes, while unresolved edges retain the original IDREF. Edge attributes preserve relationship type, raw `rid`, target XML ID, resolution, candidates, and exact locator. Labels use `defines` edges and captions use `caption_for` edges. Segmentation and normalized rendering therefore retain source identity and XML-path locators.

Fixtures live under `fixtures/generated/xml`. The focused gates are `tests/jats_scholarly_links.rs`, `tests/xml_universal_contract.rs`, and `tests/xml_promotion.rs`.
