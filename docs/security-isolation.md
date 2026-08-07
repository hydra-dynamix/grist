# Hostile-input security and backend isolation

Grist applies one `security::SecurityPolicy` through shared parse options. The
policy is deliberately closed: active content can only be preserved as inert
data, core parsing has no network mode, and input metadata remains local unless
the caller explicitly supplies it to a selected provider or persistence API.
Format-specific options cannot weaken these invariants.

## Active content and XML

Parsing never invokes code, scripts, formulas, notebook cells, document
actions, macros, or embedded executables. `ArtifactSafety` combines parser
hints, normalized media types, portable filename extensions, shebangs, and
executable magic. Any non-passive child is recorded with quarantined extraction
status and requires the separate unsafe-materialization opt-in; generated
quarantine names never reuse an attacker-controlled filename.

`security::inspect_xml` is the pre-parse XML guard. It performs no expansion,
resolution, decoding, filesystem access, or network access. The strict policy
inventories byte ranges for document types, entity declarations, XInclude, and
remote schema locations. Findings omit the referenced URI or entity value so a
diagnostic cannot accidentally persist sensitive input metadata. XML-family
parsers must run this guard and configure their underlying parser with entity,
XInclude, and schema resolution disabled.

Normalized Markdown and LaTeX rendering escapes text for the active-capable
target. Executable URI schemes become a stable inert fragment target. Raw
fallbacks use dynamically sized Markdown code fences or escaped LaTeX text;
they are never emitted as live HTML, JavaScript, or TeX commands. This is an
activation boundary, not a claim that normalized output is byte-identical.

## Archive and compound-source enforcement

Every archive/package decoder reports a relative member locator and an
`ArchiveEntryKind`. The shared container controller validates those reports
before recursive parsing or materialization. It rejects and inventories:

- absolute, drive-prefixed, UNC-shaped, parent-traversing, control-character,
  overlong, or over-deep paths;
- collisions after cross-platform separator, case, trailing-dot, and
  trailing-space normalization;
- symbolic links, hard links, block/character devices, FIFOs, and sockets;
- unsafe link targets.

Rejected members retain their content identity, parent-relative locator,
source order, extraction status, stable security diagnostic, and recovery
guidance. Their expanded bytes are not charged as accepted expansion. The
existing shared recursion budget independently limits member count, nesting,
child artifacts, cumulative expansion ratio, memory estimate, parse/provider
time, temporary storage, and output.

## Secrets and providers

`SecretString` and `SecretBytes` remain in-memory, zero-on-drop, non-cloneable,
non-serializable values with redacted `Debug`. Provider manifests and request
digests contain only input identity and public-configuration digests. Provider
execution now checks metadata, successful results, and failure diagnostics
against every request secret before building a serializable response. A leak is
replaced by a generic security diagnostic; the secret is never hashed into the
response. Provider panics are caught at the public execution boundary and
become parser-defect diagnostics.

No provider is built in or selected implicitly. A provider runs only after the
caller registers it, selects its capability for the request, and supplies a
network decision that exactly matches the binding. URI-valued source metadata
is an identifier and is never fetched by core parsing.

## Native backend sandbox contract

Native and external parsers are available only through the explicitly allowed
`IsolatedParserBackend` provider contract. `IsolationPolicy` carries positive,
non-zero wall-time, memory, temporary-storage, and process limits plus these
mandatory filesystem and execution constraints:

- read-only input;
- private temporary filesystem;
- no host filesystem access;
- no inherited environment;
- no active-content execution;
- an explicit network decision matching the provider binding.

The caller-supplied backend boundary is responsible for applying its OS or
container controls before invoking native code. Grist passes the complete
policy in the typed, digested request and rejects an invalid or weakened
policy. `create_private_temporary_storage` supplies the standard writable
filesystem: a fresh generated directory, generated child names only,
create-new writes, a hard cumulative byte limit, and the declared retention
policy.

`DeleteOnDrop` is the default and removes only a directory bearing Grist's
ownership marker. `RetainUntilExplicitCleanup` is available only as an explicit
caller choice and remains private until `cleanup` is called. On Unix the
directory mode is set to `0700`; on Windows the fresh directory inherits the
current user's ACL without broadening it. Initialization failures remove the
partially created directory.

## Stable rejection diagnostics

Archive controls use `grist.security.archive.*`; XML controls use
`grist.security.xml.*`; generic activation rejections use
`grist.security.rejected`. Unsafe content remains visible and attributable,
but no rejection path invokes it, follows it, fetches it, or writes it to a
caller-visible location.
