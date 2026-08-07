# Safe LaTeX project parsing

The latex feature provides an inert LaTeX2e project parser. Its authoritative
payload schema is grist/latex/v1 and its implementation identity is
grist-safe-latex.

## Input and detection

Detection uses tex/latex extensions, standard LaTeX media types, document
class and document-environment commands, and combinations of structural
commands. Root discovery scans caller-supplied roots deterministically, does
not follow directory symlinks, and scores files containing documentclass and
the document environment while accounting for include references. Equal top
scores are reported as ambiguous.

Text, byte, and project entry points share one payload. Byte parsing retains
original bytes, decoded text, encoding diagnostics, raw-byte ranges, text
ranges, and exact locators.

## Supported constructs

The payload preserves:

- document class, packages, title, author, date, and other metadata;
- parts, chapters, sections, paragraphs, and comments;
- macro definitions and uses, arguments, original syntax, and bounded
  expansions;
- known and unknown environments, lists/items, tables/rows/cells,
  figures/images, captions, equations, and inline/display math;
- labels, references, and citation command variants;
- every unknown command as exact raw syntax;
- resolved input/include files as nested typed sources with their own bytes,
  hashes, decoding reports, nodes, macros, paths, ranges, and locators.

Malformed groups, unclosed environments/math, unavailable includes, decoding
loss, and expansion limits return partial output with stable diagnostics.

## Include and macro limits

Local input/include resolution requires at least one canonical allowed root.
Candidates resolve relative to the including file, are canonicalized, and are
accepted only below an allowed root. Remote references, missing files,
non-files, boundary and symlink escapes, cycles, excessive depth, and byte
limits remain inert and diagnosed.

Macro expansion interprets only source definitions. It never invokes TeX
primitives. Depth, expansion count, and output characters are independently
bounded; failures retain the original macro use and make the result partial.

## Security, graph, schema, and CLI

Grist never starts a TeX engine, shell, subprocess, package installer, or
network client. Shell-capable primitives are raw inert commands. Packages and
graphics remain metadata or references.

The DocumentGraph projection covers headings, paragraphs, lists, tables,
figures, captions, equations, labels, references, citations, metadata, and raw
fallback nodes with exact locators. Nested include payloads remain embedded in
raw include nodes. Payload, envelope, and options schemas are checked in.

CLI examples:

    grist parse latex paper.tex --project-root ./project
    grist parse latex paper.tex --no-resolve-includes
