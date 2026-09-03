# Capability discovery

`grist::capabilities::discover()` and `grist capabilities` return the same
`grist/capability-manifest/v1` value. The manifest is deterministic and is
derived from the parser and provider registries compiled into the active
build. Callers with custom registries can use
`CapabilityManifest::from_registries` and supply their fixture-backed format
reconstruction claims.

The manifest reports every Cargo feature gate and whether it is enabled; CLI
and library operations; available and unavailable parsers and formats; payload,
options, envelope, and checked schema versions; parser and backend versions;
allowed and required providers; all named budget profiles and budget axes; the
closed safe security modes; and faithful reconstruction support. Collections
use canonical identifier order.

Unavailable compiled formats remain registry entries with `feature_disabled`
reasons. Dropped legacy Office and Outlook-store formats are not advertised as
placeholder parsers. Provider kinds without a registered implementation say
that caller registration and explicit selection are required. Every retained
format has a reconstruction record; a positive claim requires
format/media/profile/implementation scope and verified fixture hashes.
Normalized rendering is separately and explicitly reported as not being a byte
round trip. See [cross-format-integration.md](cross-format-integration.md) for
the retained selector matrix and its two explicit payload-only projections.

Each selector points to a payload and options schema name/version that resolves
in the embedded schema catalog when that format is available. Feature-disabled
descriptors retain schema identity metadata, but their generated schema bodies
are gated with the parser implementation. Format variants such as `docx`,
`json`, and `zip` share their authoritative family contracts rather than
advertising nonexistent selector-specific schemas. The
`document_graph_projection` capability is generated from the same closed
artifact-kind predicate used by the CLI integration test.

The `unsupported_capabilities` list is intended for simple compatibility
checks. It includes unavailable formats, provider kinds, reconstruction paths,
implicit network fetching, active-content execution, and normalized-rendering
round-trip claims. Consumers needing details should use the corresponding
format, provider, reconstruction, or security record.
