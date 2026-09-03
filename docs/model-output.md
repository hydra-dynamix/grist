# Model-output parsing contract

The `model-output` feature extracts candidate JSON values and tool calls from
mixed model responses while retaining the original response and every
candidate. It supports JSON variants, OpenAI-style calls, MCP JSON-RPC,
opt-in Python-call syntax and fenced YAML, TOML or XML tool blocks.

Repairs are bounded and recorded as typed evidence. They include missing
commas/quotes, trailing commas, Python literals, missing closing containers and
a premature object close when the following key clearly belongs to that
object. Ambiguity, incompleteness, validation failures and the source error
position remain explicit diagnostics; `--json-value` does not reduce a
malformed selected candidate to “no selected value.” Schema validation and
tool aliases are opt-in deterministic transforms.

Model-output is deliberately payload-only: inventing document structure would
weaken candidate fidelity, so it advertises no `DocumentGraph` projection.
Byte-facing repair paths are included in the universal fuzz corpus.
