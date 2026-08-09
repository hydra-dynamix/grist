# Generic graph-input contract fixtures

These conformance fixtures pin the implemented non-normative
`grist/graph-document/v1` input dialect. They are not `DocumentGraph` fixtures
and are not LDGR scheduler inputs. Encoding/closed-shape failures terminate
parsing; duplicate IDs and endpoint failures are decoded first and then
reported by the explicit semantic validator.

## Outcome table

| Fixture | Expected outcome | Boundary proved |
| --- | --- | --- |
| `valid/v1-property-multigraph.json` | valid | JSON spelling of the v1 directed property multigraph. |
| `valid/v1-property-multigraph.yaml` | valid | YAML spelling semantically equivalent to the JSON fixture after default expansion. |
| `invalid/unsupported-version.json` | invalid: `grist.graph.schema_version.unsupported` | Versions are explicit and are not guessed or silently migrated. |
| `invalid/document-graph-projection.json` | invalid: `grist.graph.schema_version.unsupported` | A normalized `DocumentGraph` is not accepted as a `GraphDocument` input payload. |
| `invalid/duplicate-node-id.yaml` | invalid: `grist.graph.node.id.duplicate` | Node IDs are exact strings unique within the node namespace. |
| `invalid/dangling-endpoint.json` | invalid: `grist.graph.edge.endpoint.unknown` | Every edge endpoint names a declared node. |
| `invalid/unknown-structural-field.yaml` | invalid: `grist.graph.field.unknown` | V1 structural mappings are closed; extensions belong in `attrs`. |
| `invalid/yaml-alias.yaml` | invalid: `grist.graph.yaml.feature.unsupported` | YAML aliases/anchors are outside the deterministic JSON-compatible subset. |

The two valid fixtures have identical semantic values after these specified
defaults are materialized:

- missing document/node/edge `attrs` becomes `{}`;
- missing node `labels` becomes `[]`;
- missing edge `directed` becomes the document-level `directed` value.

Their raw content identities differ because their bytes differ. Their parsed
canonical payload identities may match because the normalized GraphDocument
values match. Node, edge, and label arrays remain in source order.

The invalid files are intentionally parseable JSON/YAML wherever possible so
they isolate contract validation rather than encoding syntax. No invalid input
is accepted by repairing, dropping, or auto-creating declarations.
