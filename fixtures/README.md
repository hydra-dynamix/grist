# Governed fixture corpus

`corpus.v1.json` is the authoritative registry. Directory names are storage,
not fixture semantics. `grist::fixtures::load_corpus` verifies the manifest,
path safety, licensing rules, byte identities, and canonical expected outputs.
The universal parser-promotion harness consumes the same contract.

## Layout

```text
fixtures/
  corpus.v1.json                 typed registry and Section 16.1 policy
  builders/recipes.v1.json       deterministic generator inputs
  generated/                     generated synthetic, hostile, and recordings
  expected/<fixture-id>/         canonical JSON goldens
  regressions/<format>/<id>/     approved redistributable downstream cases
  licenses/                      LicenseRef texts when SPDX is insufficient
```

`private/` and `intake-staging/` are ignored. Neither is a valid manifest path.
Sensitive or non-redistributable sources are registered as `external_only`
metadata with their original SHA-256 and byte length; their bytes stay in the
owning project's approved storage. Secrets, personal data, credentials, customer
documents, and provider request bytes must never be committed.

## Required classes and per-format registration

The policy enumerates every class in specification Section 16.1: minimal,
representative, maximum-complexity, empty, truncated, malformed, adversarial,
encrypted, oversized, deeply nested, mixed/invalid text, nested attachment and
container, unsupported, provider-recording, malicious-active-content, and
downstream-regression fixtures. A format registers cases under its stable format
key. One case may cover several classes, but the promotion harness decides
whether the format has sufficient independent evidence; the corpus validator
does not turn a declared class into a passing parser gate.

Every checked case records:

- a unique slug, exact classes, media type, byte length, and SHA-256;
- origin and source, SPDX expression or checked `LicenseRef`, copyright when
  applicable, and redistribution decision;
- public/check-in versus external-only storage and immutable inert/no-network/
  no-execution handling;
- generator argv, version, seed, and recipe for generated cases;
- each expected surface, schema version, raw-file identity, canonical identity,
  and explicit normalization rules.

Licensed bytes require an upstream URI and affirmative redistribution permission.
Review the license and attribution before intake; an accessible URL is not a
license. Synthetic fixtures in this repository are Apache-2.0. Malicious fixtures
are intentionally dangerous as data: do not open them in a browser or office
application, serve them, follow their links, enable macros, or execute embedded
content. Tests must ingest bytes through Grist with network and execution denied.

## Deterministic builders

Run:

```sh
python scripts/fixture_corpus.py build --check
python scripts/fixture_corpus.py build --update-manifest
python scripts/fixture_corpus.py validate
```

The maximum-complexity recipe is declarative and ordered. The nested-container
builder fixes ZIP member order, timestamps, permissions, flags, and compression,
then wraps the previous bytes at each depth. Provider recordings contain only a
secret-free `ProviderRequestManifest` and result. Their request and output
digests are recomputed by `ProviderRecordingCatalog` before replay; recordings
must be network-denied and cannot represent decryption credentials.

`build --check` regenerates in memory and reports byte drift. Change a recipe,
generator version, or seed whenever generation semantics change, regenerate,
review the binary/hash diff, and update parser/backend metadata when output
semantics changed.

## Downstream regression intake

For public redistributable bytes:

```sh
python scripts/fixture_corpus.py intake path/to/case.bin \
  --format pdf --id downstream-1234-xref-loop \
  --source-project hydra-example \
  --issue-uri https://example.invalid/issues/1234 \
  --license-expression Apache-2.0 \
  --media-type application/pdf \
  --fixture-class malformed --fixture-class adversarial \
  --redistribution-permitted
```

Use `--metadata-only --data-classification restricted` when bytes cannot be
redistributed. Intake preserves the original identity, requires downstream
project and issue provenance, refuses a checked-in sensitive/unlicensed case,
and never invents an expected output. A maintainer must add reviewed goldens and
then run both validators before the regression can promote a parser.

## Canonical expected outputs

Goldens are UTF-8 Grist canonical JSON v1 followed by exactly one LF. The
`canonical_sha256` hashes canonical JSON without that LF; `sha256` hashes the
checked file including it. Arrays retain semantic order and object keys are
UTF-8 byte sorted by canonicalization. No timestamp, path, ID, diagnostic, or
backend field is silently ignored. Use `[{"kind":"exact"}]` for exact output,
or list each permitted caller-timestamp removal or `$FIXTURE_ROOT` replacement.

Canonicalize an approved candidate with:

```sh
python scripts/fixture_corpus.py canonicalize candidate.json fixtures/expected/<id>/<surface>.json
```

Never hand-edit a hash to make validation pass. Regenerate or explain and review
the semantic golden change.
