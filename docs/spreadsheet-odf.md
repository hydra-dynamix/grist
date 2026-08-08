# OpenDocument spreadsheet contract

The `spreadsheet-odf` feature provides inert, bounded parsing for ODS workbooks and OTS spreadsheet templates. Both registry selectors emit the authoritative `grist/spreadsheet-odf/v1` payload in the common envelope v2. Formula source, stored results, hyperlinks, drawings, and embedded objects are data only: Grist does not calculate, execute, fetch, or materialize them.

## Package and security behavior

Detection combines ZIP structure with the required OpenDocument `mimetype` member and distinguishes the workbook and template media types even without an extension. A selector that contradicts the package media type fails explicitly. ZIP traversal uses the shared archive path, collision, expansion, member, memory, node, child-artifact, cell, nesting, and cancellation controls. XML declarations, entities, DTDs, and other secured constructs are inspected before parsing and never trigger external access.

Encrypted `content.xml` or `mimetype` yields `encrypted` without a fabricated payload. Other encrypted members remain inventoried and make usable output `partial`. Rejected members retain their rejection code and message. Malformed core XML, invalid ZIP data, absent core members, and resource exhaustion remain explicit diagnostics and statuses.

## Authoritative payload

The payload preserves package and manifest order, identities, sizes, compression, encryption, and metadata; workbook calculation settings with `formulas_calculated_by_grist: false`; active table and stored split/freeze settings; named ranges and expressions; named, automatic, and default styles with source properties; and source-ordered sheets with visibility, protection, print ranges, columns, rows, and exact one-based ranges.

Cells retain compressed repeated-row and repeated-column counts, covered-cell state, source value type, source-native stored and displayed values, formula text and namespace prefix, explicitly labeled package-stored formula caches, styles, validations, spans, and merged ranges. Comments retain creator, date, and text. Links retain their inert target and external classification. Sheet-anchored and cell-contained charts, images, and embedded objects retain anchors, media types, content identities, chart source ranges, and stored chart caches. Unsupported workbook and sheet extensions remain as raw XML with package, byte, and XML-path locators.

## Projection and public surfaces

The `DocumentGraph` projection emits workbook, sheet, row, cell, comment, link, chart, image, attachment, named-range, metadata, and raw nodes. `FormulaDependsOn` is a syntax-only relation between source cells already present in the package and never represents evaluation. Sheet and cell facts use exact one-based `SheetRange` locators; metadata and raw XML use nested archive-member, byte-range, and XML-path locators.

The same parser is available through registry dispatch, `parse ods`, `parse ots`, automatic detection, graph transforms, deterministic segmentation, capabilities, and checked-in payload/options/envelope schemas. Focused fixtures cover workbooks and templates, repetitions, formulas and caches, styles, comments, links, merges, drawings, charts, hidden state, panes, metadata, raw extensions, malformed packages, hostile members, encryption declarations, large sheets, and budget failures.
