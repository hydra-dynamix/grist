# PDF native layout and semantic structure contract

The `pdf` feature provides a built-in, byte-native structural parser for PDF 1.x and PDF 2.0 containers. It is deliberately inert: parsing never evaluates JavaScript, document actions, launch targets, form actions, external references, or network resources.

## Authoritative payload

`grist/pdf/v1` preserves:

- the header version, binary marker, EOF marker, leading/trailing byte facts, and raw content identity;
- direct and compressed indirect objects with deterministic `object-number:generation` identities, revision order, exact direct byte spans or object-stream-relative spans, typed PDF values, and raw hashes;
- classic xref tables, xref streams, hybrid/incremental `Prev` chains, free/in-use/compressed entries, trailer IDs, and validation against scanned objects;
- the catalog and its page-tree, labels, metadata, outline/name/form references;
- inherited `MediaBox`, `CropBox`, `Rotate`, and `UserUnit` values, displayed page dimensions, one-based page order, and exact `PdfRegion` locators;
- decoded native glyph codes and Unicode text, font resource/object identity, subtype, base font, encoding, embedding/style facts, writing direction, rendering mode, and page-user-space bounding boxes;
- deterministic whitespace/gap tokens, baseline-clustered lines, geometric blocks, multi-column reading order, and confidence-bearing evidence for every inferred grouping;
- immutable per-page native text representations plus separately retained OCR provider responses and identity-bearing reconciled text;
- explicit full-page OCR scopes for scanned/unusable pages and raster-region scopes for hybrid pages, with one-based `PdfRegion` locators included in each provider request digest;
- provider/model/configuration/input/output provenance and distinct recognition, reading-order, and layout confidence for every OCR attempt;
- deterministic native/OCR reconciliation with every suppressed duplicate retaining its OCR text, locator, matched native locator, text similarity, geometric overlap, diagnostic, and named `duplicate_suppression` loss;
- confidence-bearing semantic columns, headings, paragraphs, list items and list groups, repeated headers/footers/page numbers, and deterministic semantic reading items;
- aligned or ruled tables with row/cell geometry, row and column spans supported by missing-interior-ruling evidence, header candidates, captions, locators, and extraction confidence;
- painted vector paths, invoked raster image/Form XObjects, bounded inline-image facts, transformed geometry, pixel dimensions when declared, figure candidates, captions, and explicit relationships;
- a bounded structure-tree health summary; malformed, cyclic, missing, or over-deep tagged structure is retained as a partial diagnostic instead of being trusted;
- page rotation on every native region and one-based half-open token ranges on token, line, and block locators;
- Info dictionary fields and decoded UTF-8 XMP metadata;
- stream filter chains, encoded/decoded hashes and lengths, and explicit decode status;
- Standard/custom encryption dictionary facts without serializing secret material;
- every applied structural repair and an envelope provenance step;
- active-content references as `inventoried_not_executed` records.
- named and legacy destinations, outline/bookmark hierarchy, page links, annotations, comments and reply relationships, each retaining its object or page-region locator;
- inert action dictionaries on links, outlines, annotations, and fields, including URI/file/named targets and script hashes without evaluating, launching, submitting, printing, or fetching anything;
- AcroForm field hierarchy, values/defaults/flags, calculation order, signature flags, and signature dictionaries as metadata (signer, reason, location, time, byte ranges, filters, and a contents hash; no cryptographic validity claim);
- optional-content groups/layers with intent, usage dictionaries, and explicit default visibility;
- embedded-file name trees and file specifications as shared `EmbeddedArtifact` records, with immediate-parent content identities, media type, filename, relationship, extraction state, safety classification, and recursive nested-PDF attachment identities;
- explicit `unsupported`, `encrypted`, `malformed`, and `budget_limited` child states that retain inventory and identity even when no child payload can be interpreted.

The current core decoder handles `FlateDecode`, `ASCIIHexDecode`, `ASCII85Decode`, and `RunLengthDecode`. Image-native DCT, JPX, CCITT, and JBIG2 streams are inventoried without raster decoding. LZW, Crypt, unknown filters, and predictor transforms are explicit unsupported/partial conditions; they are never treated as empty streams.

Native text interpretation supports the PDF text-state and positioning operators (`BT`, `ET`, `Tf`, `Tm`, `Td`, `TD`, `T*`, `Tc`, `Tw`, `Tz`, `TL`, `Tr`, `Ts`, `Tj`, `TJ`, `'`, and `"`), graphics-state transforms (`q`, `Q`, and `cm`), literal/hex strings, text arrays, inherited resources, content arrays, simple-font widths, CID `W`/`DW` widths, standard single-byte decoding, Encoding Differences, and `bfchar`/`bfrange` ToUnicode CMaps. Semantic graphics interpretation covers line/rectangle path painting, raster/Form `Do` invocations, and bounded `BI`/`ID`/`EI` inline-image dictionaries; encoded pixels remain inert.

Font and content failures are explicit. Missing font resources, unusable or absent required ToUnicode maps, invalid text operators, missing/non-stream content objects, undecodable content streams, and text-operation/glyph budget limits produce stable diagnostics. Geometry or mappings based on fallback metrics use approximate locators with confidence rather than claiming exact coordinates or Unicode.

## OCR and reconciliation

OCR runs only when the caller explicitly selects an `Ocr` provider in the request. `PdfOcrMode::Auto` requests a whole page when native text is absent, unusable, or budget-limited, and requests each raster-image region on a hybrid page with usable native text. `AllPages` requests every page; `Disabled` makes no provider calls even when a provider is selected. `max_scopes` bounds calls deterministically in page/graphic order. Language hints and table-recognition intent are part of each request manifest, and the page/region locator is included in `OcrOptions::source_locator` so a provider can distinguish repeated calls over the same PDF bytes.

Each page retains `native`, zero or more `ocr_attempts`, and an optional `reconciled` representation. Native text is never mutated. Every OCR attempt stores its complete `ProviderResponse`, scope, region locators, provider/model versions, configuration and request digests, input/output identities, diagnostics, confidence model, network decision, and deterministic-recording flag. A failed response remains in `ocr_attempts`, makes the operation partial, and leaves the native representation usable; it never fabricates reconciled text.

`grist.pdf.native-ocr-reconcile@1` orders native and nonduplicate OCR regions geometrically, prefers native text only when normalized text similarity and geometric-overlap thresholds both pass, and records each suppression individually. OCR output remains available even when its duplicate is omitted from the reconciled projection. Reconciled identities bind the native identity, every successful provider-output identity, algorithm/version, and configuration digest. Reading-order and layout confidence plus named evidence remain separate; a provider that omits either requested confidence produces an explicit partial diagnostic rather than an invented value.

## Status and repair behavior

A structurally valid unencrypted PDF returns `complete`. A usable document recovered from missing or inconsistent xref data, object boundaries, lengths, page counts, EOF markers, unsupported filters, active-content inventory, or configured limits returns `partial` with stable diagnostics. A recognized encrypted PDF returns `encrypted` with no fabricated payload. A missing header, catalog, page root, or usable page produces `failed`.

`PdfOptions::set_password` and `with_password` accept `SecretString`. The password field is skipped by Serde and JSON Schema, redacted by `Debug`, zeroized on drop, and excluded from option digests, diagnostics, identities, and provenance. This structural backend inventories encryption but does not claim successful authentication or decryption.

## Resource and security limits

Object count, syntax depth, page count, page-tree depth, page-label number-tree nodes, decoded stream bytes, content operations, native glyphs, semantic graphic objects, interactive objects, embedded-file count, cumulative embedded bytes, nested embedded-PDF depth, and OCR scopes are bounded. Registered parsing charges every nested artifact to the shared child-artifact budget. Limit hits are diagnostic and cannot masquerade as EOF. Streams are decoded in memory under the declared cap; no file is extracted, no child path is materialized, and no subprocess is invoked. Providers run only through the explicitly selected, budgeted provider boundary with the caller's exact network policy.

## Projection and dependent scopes

The `DocumentGraph` projection emits a document node, ordered page nodes, semantic headings/paragraphs/headers/footers/list items, list groups, tables/rows/cells, figures/images/captions, separately labeled native/OCR/reconciled text nodes, native text-run lines, token spans, bookmarks, links, annotations, comments, forms/fields, signature/layer metadata, and recursively nested attachments with stable identities and locators. Explicit `ResolvesTo`, `ReplyTo`, `AnnotationFor`, `ParentOf`, `AttachmentOf`, and containment edges retain source evidence. Confidence-bearing `Precedes`, list-containment, `CaptionFor`, `AlternativeRepresentationOf`, and `ReconciledWith` edges name their inference rules. Text nodes expose `text_origin` and `segment_primary` attributes, allowing segmentation to select the reconciled projection without erasing alternative representations. Table and figure/caption graph groups participate in the shared atomic segmentation policy, while page/root extensions retain authoritative native, OCR, reconciled, semantic, and interactive payloads.

Every inferred semantic or reconciled object includes bounded confidence and named geometric, typographic, textual, repetition, ruling, XObject, caption, provider, or duplicate-matching evidence. Unsupported shading geometry produces an explicit loss diagnostic. Parsing never executes interactive content.

Focused synthetic fixtures cover classic xref, repaired xref, xref streams, object streams, page inheritance and labels, metadata, unsupported filters, active content, encryption, malformed bytes, missing EOF, resource limits, deterministic multi-column born-digital/hybrid text, directional ToUnicode text, repeated margins/page numbers, headings/lists, ruled tables with merged spans and headers, vector/raster figures and captions, malformed structure trees, destinations/outlines, link and comment annotations, form/signature metadata, optional-content layers, recursive and encrypted/unsupported/budget-limited attachments, scanned-page and hybrid-region OCR, provider failure, deterministic recording replay, duplicate suppression, graph/segment projection, native/OCR/reconciled source maps, schemas, registry/CLI routing, and secret redaction.
