# Deterministic repository ingestion

`grist::ingest::ingest_repo` traverses one caller-supplied filesystem root. The
root is canonicalized once, traversal never follows directory symlinks, and a
file symlink is followed only when `SymlinkPolicy::FollowFilesWithinRoot` is
selected and its canonical target remains below that root. The default
`SymlinkPolicy::Skip` inventories links without reading their targets.

Nested Git working trees and submodules are detected by a nested `.git` entry.
`SubmodulePolicy::Skip` is the safe default. `SubmodulePolicy::Traverse` walks
their working-tree files but still inventories and skips repository metadata.
Neither policy invokes Git or resolves remote submodule content.

## Ignore and selection policy

Repository-local `.gitignore`, `.ignore`, and `.git/info/exclude` rules are
honored by default. Parent and global ignore files are deliberately excluded so
the result depends only on the explicit root and options. A raw root-confined
pass inventories paths omitted by the ignore-aware pass. `include_ignored`
disables ignore filtering; include and exclude globs remain independent and all
applicable reasons are retained in `RepositoryEntry.skip_reasons`.

`.git`, `target`, and `.bathysphere` trees are pruned with named reasons. An
external artifact directory located inside the repository is also pruned so an
ingestion cannot consume its own output.

## Inventory and parsing

Every encountered file-like entry has a deterministic root-relative path.
Non-Unicode path bytes are percent-escaped where the platform exposes them.
Entries distinguish parsed, binary, unsupported, ignored, policy-skipped,
budget-limited, and failed outcomes. Ignored and over-budget files are not read;
their known metadata and every applicable skip reason remain in the inventory.
Known unsupported formats and opaque binary files retain exact byte length and
raw SHA-256.

File reads stop after `max_file_bytes + 1`, repository file count and traversal
depth are enforced independently, and entries beyond either limit retain a
named budget disposition. Canonical checks on every traversed directory and
read file also reject reparse-point or mount-style escapes, not only symlinks.

Eligible files use the same ranked detector, decoder, parser registry, typed
payloads, diagnostics, and canonical identities as `Ingestor`. Special-name
detection covers common Cargo, npm/pnpm/yarn, Python, Maven/Gradle, Go,
Docker/Compose, CI, and Git metadata manifests and lockfiles. Supported JSON,
YAML, TOML, Markdown, text, and source-code manifests are parsed; recognized
formats without an enabled parser remain explicitly unsupported.

## Stable hashes and ordering

All public collections are sorted by normalized repository-relative path before
serialization. Three SHA-256 aggregates use `grist/canonical-json/v1`:

- `inventory_sha256` hashes the ordered `RepositoryEntry` inventory, including
  dispositions and skip reasons;
- `content_sha256` is the shared compound identity of every file whose bytes
  were read, including unsupported and binary files;
- `parsed_artifacts_sha256` hashes path, artifact kind, schema, raw content hash,
  and canonical payload hash without depending on inline versus external
  storage.

The aggregate hashes omit the absolute root and external artifact location, so
equivalent repositories have equal aggregates regardless of creation order or
checkout location. Ignored bytes do not affect the content aggregate because
the ignore contract intentionally prevents reading them; their paths,
dispositions, and visible metadata remain covered by the inventory aggregate.

Any child result that is partial, failed, ambiguous, encrypted, cancelled, or
security-rejected makes the repository envelope partial. Policy-compliant
ignore, glob, symlink, and submodule skips remain normal inventory outcomes.
