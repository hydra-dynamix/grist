# Embedded artifacts and materialization

`grist::container::EmbeddedArtifact` is the versioned inventory record for a
child found inside an archive, package, message, or compound document. The wire
contract is `grist/embedded-artifact/v1`, exposed by `grist schema emit
embedded-artifact` when schema support is enabled.

## Identity and storage

Every extracted child has a `ContentIdentity` for its exact bytes. Its
`artifact_id` is a SHA-256 digest of canonical JSON containing the v1 identity
domain, the complete parent content identity, parent relationship, nested
`SourceLocator`, and child raw identity. Declared filenames, disposition,
extraction status, safety evidence, storage mode, and inline threshold are not
identity inputs. Moving the same child between inline and content-addressed
storage therefore does not change its identity.

`ArtifactCaptureOptions::inline_threshold_bytes` is explicit for every capture.
Bytes whose length is less than or equal to the threshold are retained inline.
Larger bytes require a caller-supplied `ContentAddressedArtifactSink`; the
serialized reference contains the `sha256` algorithm, digest, and byte length.
Both storage variants serialize the threshold that selected them, and contract
validation rejects a representation inconsistent with that threshold or with
the child identity. Grist defines the address and validation rules but does not
choose persistent storage policy.

Inventory-only traversal retains the exact raw child identity without retaining
its bytes. `capture_inline` and `capture_content_addressed` provide explicit
storage modes, including a verified content address for empty content. These
representations produce the same artifact ID.

Children that could not be extracted use `record_unavailable`. Their status is
one of inventory-only, skipped, encrypted, unsupported, rejected,
budget-limited, or failed, and always carries a non-empty machine status code.
An optional message and sorted diagnostic codes preserve the reason. Extracted
and quarantined statuses require content; terminal unavailable statuses reject
content.

When bytes were available but policy prevents their retention,
`record_known_unavailable` preserves their raw identity without content. This is
used for auditable budget-limited recursion and does not relax the rule that
extracted/quarantined states require retained content.

## Metadata and safety

The record preserves the original filename only as metadata. It also retains
the declared media type, immediate parent identity and relationship, complete
nested locator, source disposition, and deterministic safety evidence. Safety
classification considers parser hints, executable magic bytes, declared media
type, and filename extension. Executable, macro, script, active, encrypted,
suspicious, and unknown content is quarantined; an opaque media type or unknown
extension is not treated as passive merely because no executable signature was
recognized.

Classification inventories content and never executes it. No artifact API
performs network access. Callers remain responsible for supplying any external
content-addressed store or resolver.

## Opt-in materialization

`MaterializationRequest::default()` is disabled and performs no filesystem
operation. Materialization requires an explicit destination directory. The
original filename is never joined to that directory: Grist generates a name
from the verified SHA-256 digest and a media-type-derived extension, then uses
exclusive creation. Existing names receive bounded numeric suffixes, so
collisions do not overwrite files and traversal-shaped metadata cannot escape
the selected directory.

Non-passive content is blocked before resolving bytes or creating a directory.
It requires the separate `allowing_unsafe()` opt-in and is written with a
`.quarantine` extension. Inline and resolved external bytes are verified
against both the retained raw identity and content-addressed reference before
filesystem I/O. A supplied `BudgetTracker` checks temporary-storage allowance
before directory creation. The outcome reports disabled, blocked-by-safety, or
materialized with the generated path, length, digest, and quarantine flag.

The materialization directory itself must be non-empty, must be a directory,
and must not be a symbolic link. The caller explicitly controls that root and
its retention policy; Grist does not create caller-visible files anywhere else.
