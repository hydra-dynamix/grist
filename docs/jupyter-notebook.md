# Jupyter Notebook parsing

The `notebooks` feature exposes `grist::notebook` and the canonical
`grist::formats::jupyter` namespace. The registry selector is `ipynb`;
`jupyter`, `jupyter_notebook`, and `notebook` are aliases. Detection uses
the `.ipynb` extension, `application/x-ipynb+json`, or an nbformat object
with a v4 `cells` array or v3 `worksheets` array.

The authoritative `grist/ipynb/v1` payload preserves:

- notebook version, metadata, widget state, and unknown top-level members;
- cell order, native IDs, deterministic stable IDs, type, exact source JSON,
  normalized source text, metadata, attachments, and execution counts;
- v3 worksheet ownership, `input`/`prompt_number`, and legacy output/MIME
  names alongside normalized v4-style output kinds;
- stream text, rich MIME bundles, errors and traceback lines, transient display
  IDs, widget views, unknown output members, and output-to-cell stable IDs;
- exact `NotebookCell` locators for cells and outputs, with nested JSON
  pointers for attachments.

MIME values remain JSON values. Base64 image or attachment data is retained as
text and is never decoded into an executable object. Parsing never starts a
kernel, evaluates source, renders HTML or JavaScript, instantiates widgets,
resolves references, or writes notebook-controlled paths.

The payload projects to `DocumentKind::Notebook` with `NotebookCell`,
`CellOutput`, attachment, and widget metadata nodes. Containment plus explicit
`DerivedFrom` and `AttachmentOf` edges retain output/attachment ownership;
the shared segmenter treats notebook cells as structural boundaries.

Public schemas are registered as `ipynb`, `ipynb-envelope`, and
`ipynb-options`. The generic CLI registry path supports explicit `ipynb`
parsing, automatic detection, graph projection, and downstream segmentation.
Malformed JSON, unsupported versions, nesting limits, cell/output/MIME limits,
operation cancellation, and resource budgets produce explicit failed or
partial envelopes rather than silent loss.
