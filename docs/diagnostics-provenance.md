# Diagnostics and provenance

Grist diagnostics use `grist/diagnostic/v1`. The wire-level `code` remains an
open string so patch releases and caller parsers can add codes, while the seven
shared condition families have fixed codes:

| Condition | Stable code |
| --- | --- |
| Malformed input | `grist.input.malformed` |
| Unsupported content | `grist.content.unsupported` |
| Lossy normalization | `grist.normalization.lossy` |
| Provider failure | `grist.provider.failed` |
| Parser defect | `grist.parser.defect` |
| Security rejection | `grist.security.rejected` |
| Resource-budget exhaustion | `grist.budget.exhausted` |

Each diagnostic records its emitting module and parser, root-cause message,
partial-output effect, optional source identity and nested `SourceLocator`,
structured cause chain, safe structured details, recovery guidance, affected
node or artifact IDs, and an explanation key or documentation URI. The legacy
`source`, `range`, and flat `cause` fields remain readable for v1 consumers.

`DiagnosticDetails` accepts JSON objects but rejects common credential fields
recursively and authorization-header-like values. Runtime credentials use
`SecretString`, which has neither serialization support nor an exposing debug
representation. Diagnostics should describe the credential condition, never
copy the credential. Legacy v1 scalar details remain readable, while new code
constructs structured object details through `DiagnosticDetails::from_value`.

## Parser and provider metadata

`ParserInfo` identifies the parser, Grist version, concrete implementation and
version, enabled feature, grammar or specification version, and optional build
identity. `ProviderInvocation` records only attributed metadata: provider and
implementation names, model version, configuration digest, input/output
identities, caller-supplied timing, confidence model, determinism claim, and
diagnostics. Provider configuration values and credentials are not part of the
serialized contract.

## Provenance chains

Every new `Envelope` starts with one operation provenance step. When source and
payload identities are available, serialization links the raw or aggregate
input digest to the canonical payload or aggregate output digest. Changing an
envelope's operation or options digest updates this root step.

Additional normalization, repair, reconciliation, provider, transform, render,
or segmentation work appends a `ProvenanceStep` with
`ProvenanceStep::new`. The checked constructor requires input/output identities,
an implementation, an options digest, and an explicit `DeclaredLoss`. Lossless
steps omit `loss_class` on the wire; lossy steps name a stable or caller-defined
class. Timestamps are included only when supplied by the caller.

The absence of `loss_class` is the compatibility representation of an explicit
lossless declaration. It never means that loss was not considered.
