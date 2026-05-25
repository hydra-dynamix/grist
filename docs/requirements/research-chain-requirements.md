Here’s a practical requirements spec for an external utility dependency that centralizes **document/code ingestion** and **model-output parsing** for the evidence viewer / research-chain ecosystem.

**Purpose**
The dependency should provide one trusted ingestion and normalization layer for arbitrary research inputs, source files, generated artifacts, and model responses, so the app does not duplicate fragile parsing logic across dashboard, workers, validators, and viewers.

**Core Capabilities**

1. **Document ingestion**
   - Ingest plain text, Markdown, JSON, JSONL, CSV, TSV, YAML, TOML, HTML, PDF, DOCX, notebooks, and common source files.
   - Preserve original bytes, decoded text, content type, file size, path/source URI, SHA-256 hash, timestamps, and parsing warnings.
   - Produce normalized text blocks with stable block ids.
   - Extract structural sections: headings, tables, code blocks, frontmatter, links, references, cells, metadata.
   - Never silently drop unreadable sections; emit structured warnings/errors.

2. **Code ingestion**
   - Detect language by extension and/or content.
   - Preserve raw source exactly.
   - Extract imports, exports, symbols, functions/classes, comments/docstrings, tests, config files, and dependency manifests where feasible.
   - Support repo-level ingestion with ignore rules.
   - Produce file-level and symbol-level hashes for provenance.
   - Avoid executing code during ingestion.

3. **Notebook ingestion**
   - Parse `.ipynb` files into cells.
   - Preserve source, outputs, execution counts, errors, metadata, and attachments.
   - Distinguish authored code, generated output, and rendered result.
   - Normalize notebook cells into evidence blocks.

4. **Model output parsing**
   - Parse raw model responses into:
     - plain text;
     - JSON objects;
     - fenced JSON/code blocks;
     - tool calls;
     - markdown sections;
     - citations/evidence refs;
     - declared claims;
     - proposed mutations;
     - validation reports.
   - Support malformed or partial model output recovery, but mark recovered output as non-canonical until validated.
   - Preserve raw model output and parsing diagnostics.
   - Never trust parsed model output as valid just because parsing succeeded.

5. **Schema normalization**
   - Emit stable normalized records, for example:
     ```json
     {
       "schema_version": "ingested-artifact/v1",
       "artifact_id": "...",
       "source": "...",
       "kind": "markdown|json|code|notebook|model_output|pdf|...",
       "sha256": "...",
       "blocks": [],
       "claims": [],
       "tables": [],
       "code_units": [],
       "warnings": [],
       "errors": []
     }
     ```
   - Version every schema.
   - Validate output against JSON Schema or equivalent.
   - Include migration strategy for schema changes.

6. **Evidence/provenance discipline**
   - Every extracted block must point back to byte/file/source provenance.
   - Derived summaries must reference primary block ids.
   - Hash raw input and normalized output separately.
   - Record parser version, dependency versions, runtime platform, and options used.
   - Support reproducible re-ingestion.

7. **Security**
   - No arbitrary code execution.
   - No network access unless explicitly enabled.
   - Bounded file size and archive expansion limits.
   - Defend against zip bombs, path traversal, malicious PDFs/DOCX, embedded scripts, and oversized model responses.
   - Treat all ingested content as untrusted.
   - Package updates must respect the 48-hour age gate.
   - Prefer `pnpm` for JS package workflows and `uv` for Python workflows.

8. **API shape**
   - Library API and CLI.
   - CLI examples:
     ```bash
     ingestctl ingest path/to/file --out artifact.json
     ingestctl ingest-repo ./repo --out corpus.json
     ingestctl parse-model-output response.txt --schema proposal-response/v1 --out parsed.json
     ingestctl validate artifact.json
     ```
   - Programmatic API should support streaming/large files.
   - Must return structured errors, not only exceptions/stdout text.

9. **Integration with research-chain**
   - Should be usable from:
     - dashboard;
     - sandboxed workers;
     - validators;
     - standalone evidence viewer bundle generation;
     - CLI tools.
   - Must not mutate canonical state directly.
   - Should output candidate artifacts only; research-chain validators/policy decide acceptance.
   - Should support local-first operation.

10. **Testing requirements**
   - Fixture matrix for valid, malformed, adversarial, and oversized inputs.
   - Golden tests for stable normalized output.
   - Fuzz/property tests for model-output parsing.
   - Regression tests for:
     - malformed JSON;
     - markdown with nested fences;
     - PDFs with weird encodings;
     - notebooks with error outputs;
     - code files with syntax errors;
     - prompt-injection-like content.
   - Snapshot tests must be limited to parser output, not huge end-to-end golden paths.

**Non-goals**
- It should not decide scientific truth.
- It should not approve canonical mutations.
- It should not execute arbitrary notebooks/code.
- It should not replace validators or policy.
- It should not hide malformed input behind “best effort” success.

**Acceptance Criteria**
- Given raw files/artifacts, it emits normalized, schema-valid records with hashes and provenance.
- Given malformed model output, it preserves raw text and emits parsing diagnostics.
- Given research-chain demo artifacts, it can generate the evidence bundle manifest without custom parsing code in the dashboard.
- It runs locally, deterministically, and without network access by default.