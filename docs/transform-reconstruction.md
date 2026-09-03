# Graph transforms and format reconstruction

The graph transform API is the normalized graph-operation boundary. It accepts
an ordered options pipeline and returns a versioned transform envelope. The
result contains the output graph, canonical input/output graph identities, and
exactly one source-map entry per output node.

Preserved nodes keep their IDs and original locators. Modified or derived nodes
name their input node IDs and derivation steps. Missing locators are represented
by an explicit unavailable reason. The built-in identity and conditional
obligation operations are lossless; future operations that remove or alter
content must populate the fidelity losses, and a silent node removal fails the
operation.

The CLI transform command with a graph target emits the same
graph-transform-envelope contract. Text transform manifests retain both the
renderer generated-range source map and the preceding graph-transform map.

Package reconstruction is not rendering. A format adapter implements the
format reconstructor trait and declares one reconstruction claim, scoped by
format, media type, package profile, implementation, and version. Claims are
invalid without checked fixture identities proving the advertised maximum
fidelity.

All adapters run through the checked reconstruction wrapper; adapters do not
construct the public envelope themselves. The wrapper verifies requested versus
achieved fidelity, exact raw hashes for byte-identical claims, declared
differences for lossy results, package source-map node IDs and ranges, canonical
identities, and provenance. Every successful result therefore contains a
mandatory fidelity report. Lossy reconstruction returns a partial envelope and
a lossy diagnostic.

Normalized Markdown, LaTeX, HTML, text, and JSON continue to report
normalized-not-byte-round-trip. That statement is independent of any
format-specific reconstruction capability.
