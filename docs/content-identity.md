# Content identity and canonical JSON

Grist distinguishes the bytes it received from text produced by decoding, typed payloads produced by parsing, and manifests produced for compound inputs. These identities are related but never interchangeable.

## Digest notation

Every digest uses SHA-256 and is serialized as `sha256:` followed by 64 lowercase hexadecimal characters.

- `RawContentIdentity.sha256` hashes exactly the original input byte slice. It has no domain prefix, filename, length field, media type, diagnostic, or decoding metadata.
- `DecodedContentIdentity.sha256` hashes exactly the UTF-8 encoding of the decoded Rust `str`. If decoding was lossy, replacement characters are included in those UTF-8 bytes. The encoding label and `lossy` flag are not hashed. Adding or changing a decode diagnostic cannot change the raw digest.
- `CanonicalPayloadIdentity.sha256` hashes exactly the bytes returned by the canonical JSON version named in `canonicalization`. The payload schema version is retained beside the digest but is not prepended to those bytes.
- `AggregateContentIdentity.sha256` hashes the canonical JSON bytes of the aggregate manifest described below. Summary fields and the digest itself are excluded from that manifest.

Byte lengths always describe the byte sequence associated with that identity, not Unicode scalar values or UTF-16 code units.

## Grist canonical JSON v1

The compatibility identifier is `grist/canonical-json/v1`. The one implementation is `grist::core::canonical_json_bytes`.

Version 1 first materializes the input through `serde_json::to_value`, then emits:

- UTF-8 JSON with no insignificant whitespace;
- object keys sorted lexicographically by their UTF-8 bytes at every nesting level;
- arrays in their semantic input order;
- compact `serde_json` encodings for strings, numbers, booleans, and null.

Map insertion order therefore cannot alter canonical bytes. Arbitrary arrays are not sorted because array order is data. Collection types whose order is operational rather than semantic must normalize that order before canonical encoding.

The version is a closed Rust/Serde enum. A changed algorithm requires a new variant and compatibility work; the behavior of v1 must never be changed in place.

## Detection identity

`FormatIdentity` retains the detected format name and media type. `DetectionCandidate` retains a one-based rank, confidence, and typed evidence records. Identity construction sorts candidate records by rank and deterministic tie breakers but does not discard alternatives. Parser selection and ambiguity policy remain detection concerns.

## Compound inputs

An aggregate member records its compound member path, optional source-order index, byte length, available raw/decoded/canonical/child-aggregate digests, and optional format identity.

Before hashing, members are sorted by:

1. presence and value of the member index;
2. UTF-8 member path bytes;
3. byte length, then raw, decoded, canonical payload, and child aggregate digests;
4. format identity.

The hashed manifest has exactly these logical fields:

```json
{
  "schema_version": "grist/aggregate-identity/v1",
  "canonicalization": "grist/canonical-json/v1",
  "members": []
}
```

Canonical JSON key ordering determines the physical byte order. `member_count`, `total_byte_length`, `manifest_byte_length`, and `sha256` are derived summaries and are not hashed. Supplying stable member indexes preserves archive/package source order; path-only collections such as repositories remain deterministic across parallel completion order.
