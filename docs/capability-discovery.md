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

Unavailable formats remain registry entries with `feature_disabled`,
`not_implemented`, or `backend_unavailable` reasons. Provider kinds without a
registered implementation say that caller registration and explicit selection
are required. Every known format has a reconstruction record; a positive claim
requires format/media/profile/implementation scope and verified fixture hashes.
Normalized rendering is separately and explicitly reported as not being a byte
round trip.

The `unsupported_capabilities` list is intended for simple compatibility
checks. It includes unavailable formats, provider kinds, reconstruction paths,
implicit network fetching, active-content execution, and normalized-rendering
round-trip claims. Consumers needing details should use the corresponding
format, provider, reconstruction, or security record.
