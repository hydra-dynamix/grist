# Provider contracts

Version: grist/provider-response/v1

Grist does not perform OCR, speech recognition, decryption, or native conversion by itself. These operations cross a caller-supplied provider boundary. Enabling a format feature neither selects a provider nor grants network or backend permission.

## Request boundary

Every capability has a typed request and result:

- OcrRequest / OcrResult, including an optional digest-bearing compound-source locator, attributed regions, layout coordinates, language, reading order, and distinct normalized recognition, reading-order, and layout confidence;
- TranscriptionRequest / TranscriptionResult, including stable timed segments, speaker labels, language, and normalized confidence;
- DecryptionRequest / DecryptionResult, requiring at least one runtime secret and verifying the returned byte identity;
- IsolatedBackendRequest / IsolatedBackendResult, requiring BackendPermission::ExplicitlyAllowed and a validated isolation policy.

All requests contain ProviderRequestContext. It computes the raw input identity and canonical SHA-256 digest of public configuration. It also carries an explicit NetworkAccess decision and optional timing supplied by the caller. Grist never reads a clock to populate provider metadata.

ProviderRequestManifest is the only serializable request view. It contains the provider kind, input identity, network decision, configuration digest, and capability-parameter digest. It contains no input bytes and no secret names, values, count, or presence.

## Secrets

SecretString and SecretBytes are runtime-only values. They do not implement serialization, hashing, equality, or cloning; debug output is always redacted; and their owned allocations are overwritten on drop. ProviderSecrets stores only borrowed references and also has redacted debug output.

Secrets are deliberately excluded from request and configuration digests. A caller must not place a secret inside the public configuration or typed options passed for digesting.

## Metadata and determinism

Every typed provider supplies ProviderMetadata with:

- provider name;
- implementation and implementation version;
- optional model version;
- guaranteed, guaranteed_with_recording, or not_guaranteed determinism;
- an optional named confidence model.

Each ProviderResponse records the exact request/configuration/input/output identities, explicit network decision, whether replay was recorded, caller timing, and diagnostics. Its envelope-compatible ProviderInvocation retains the same attribution.

Confidence values are finite values from zero through one. A result cannot emit confidence unless metadata names the confidence model. Local execution is not assumed deterministic. RecordedProvider replays a result by request digest, rejects duplicate/mismatched recordings, cannot replay decryption credentials, performs no network access, and marks determinism as guaranteed_with_recording.

Checked recordings use `grist/provider-recording-catalog/v1`. Each entry retains
the secret-free `ProviderRequestManifest`, its canonical request digest, the
typed result, and its canonical output digest. Loading recomputes both digests,
rejects duplicate entries, requires network-denied requests and
`guaranteed_with_recording` metadata, and rejects empty or decryption catalogs
before constructing `RecordedProvider`. The deterministic OCR and transcription
contract recordings live under `fixtures/generated/provider/`; their generator
recipe contains only synthetic bytes and public configuration.

## Native, provider, and reconciled facts

RepresentationSet has three distinct fields:

1. immutable native extraction;
2. append-only provider attempts, including failed attempts;
3. an optional reconciled representation.

Provider output never replaces the native value. A failed provider response records diagnostics with no output identity and leaves native facts intact. Reconciliation requires a named algorithm and version, a configuration digest, the exact native input identity, and identities of successful provider outputs already present in the set.

Examples include PDF native text plus OCR, media-native subtitle cues plus transcription, or native metadata plus an isolated legacy backend. Consumers can choose any representation without mistaking provider-derived or reconciled content for source-native facts.

## Isolation and network safety

A ProviderSet is empty by default. A caller selects one implementation for one provider kind and one network permission. Invocation fails if no provider was selected, the typed capability differs, or request permission differs from the selected binding.

Isolated backends additionally require an explicit opt-in token and a policy with a private temporary filesystem, read-only input, nonzero process/time/memory/temporary-storage limits, matching network permission, and active-content execution disabled. Registry metadata remains an inspectable capability declaration; the per-request isolation policy is the enforceable invocation contract.

The checked JSON Schemas are `schemas/grist.provider-response.v1.schema.json`
and `schemas/grist.provider-recording-catalog.v1.schema.json`, exposed as
`provider-response` and `provider-recording-catalog` by the schema registry.
