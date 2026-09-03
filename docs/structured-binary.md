# Structured binary data

The structured-binary feature implements inert, typed decoding for CBOR,
MessagePack, and Protocol Buffers. Its authoritative payload schema is
grist/structured-binary/v1; JSON is a deterministic projection rather than the
source model.

## Formats and detection

- CBOR follows RFC 8949, including definite and indefinite byte strings, text,
  arrays, maps, simple values, half/single/double floats, arbitrary map keys,
  duplicate keys, tags, and concatenated CBOR sequences. The self-described
  tag has decisive magic-byte evidence.
- MessagePack follows the v5 format, including all integer and float widths,
  strings, binary values, arrays, maps, extension/fixed-extension values,
  timestamp extension -1, duplicate/non-string map keys, and concatenated
  streams.
- Protocol Buffers accepts binary messages only with caller-supplied serialized
  google.protobuf.FileDescriptorSet bytes and a fully-qualified message name.
  Descriptor files, packages, nested messages/enums, scalar fields, packed and
  unpacked repetition, maps, oneofs, proto2 required fields, and declared
  extensions are interpreted. Unknown wire fields remain raw typed records.

.cbor, .msgpack/.mpk, and .pb/.protobuf extensions and registered media types
contribute ranked evidence. Extensionless CBOR and MessagePack are selected
only when exactly one complete single-object structural probe succeeds. A
complete valid Protobuf wire probe can identify extensionless data, but decoding
still requires the descriptor. Overlapping scalar encodings remain ambiguous
rather than being guessed.

## Typed payload and identity

Every top-level object is a one-based BinaryRecord. Every value and field has:

- a stable structural ID and path;
- an exact zero-based half-open byte span;
- a nested RecordRange plus record-relative ByteRange locator;
- a typed scalar that preserves integer spelling, float width/raw bits and
  non-finite values, or byte content as lowercase hexadecimal;
- ordered map/message entries and array items;
- format metadata for CBOR tags, MessagePack extensions, or Protobuf fields.

CBOR and MessagePack payloads identify their specification. Protobuf payloads
record the descriptor SHA-256, byte size, descriptor file order, selected message,
and syntax. Canonical payload identity therefore changes when the descriptor
changes even if the message bytes do not.

The JSON projection uses ordinary JSON when lossless. Bytes, undefined/simple
values, non-finite floats, out-of-range integers, tagged values, extensions,
unknown fields, non-string keys, and duplicate map keys use explicit tagged
objects. Protobuf repeated fields become arrays. Authoritative ordered entries
are never replaced by this projection.

## Options and limits

StructuredBinaryOptions declares nesting, value-count, collection-length, and
blob-size ceilings, sequence behavior, malformed recovery, and optional
Protobuf descriptor options. Registry parsing additionally consumes the shared
record, node, nesting, memory, input, output, time, and cancellation budgets.
A limit or cancellation is diagnostic and cannot appear as complete output.

Strict malformed recovery returns a failed envelope without a fabricated
payload. Preserve-raw recovery retains the complete input as an exact raw value
and returns partial. Descriptor absence, malformed descriptor sets, and unknown
message names always fail because raw bytes cannot be typed without schema
identity.

## Diagnostics

Stable code families include:

- binary.nesting_limit, binary.value_limit, binary.collection_limit, and
  binary.blob_limit;
- cbor.truncated_*, cbor.invalid_utf8, cbor.invalid_indefinite_chunk,
  cbor.map_missing_value, cbor.reserved_additional_info, and
  cbor.unexpected_break;
- messagepack.truncated_*, messagepack.invalid_utf8,
  messagepack.invalid_timestamp, and messagepack.reserved_marker;
- protobuf.descriptor_*, protobuf.message_not_found,
  protobuf.wire_type_mismatch, protobuf.required_field_missing,
  protobuf.singular_field_repeated, protobuf.oneof_multiple_members,
  protobuf.unknown_fields_preserved, and protobuf.unknown_field_dropped.

Wrong descriptor wire types are retained as unknown bytes with a partial
diagnostic. Descriptor-unknown fields are losslessly retained without making
otherwise valid decoding partial.

## Graph, schemas, and CLI

ToDocumentGraph projects documents, records, fields, structured values, and raw
unknown nodes with exact locators and a namespaced grist.structured_binary
extension. Stable IDs use source scope, payload schema, structural path, native
IDs, and locators. The normal segment engine therefore produces citation-ready
segments without copying unlocated text.

Checked contracts are:

- grist.structured-binary.v1.schema.json;
- grist.structured-binary-envelope.v2.schema.json;
- grist.structured-binary-options.v1.schema.json.

CLI examples:

    grist parse cbor value.cbor
    grist parse messagepack events.msgpack
    grist parse protobuf person.pb --descriptor api.desc --message example.Person
    grist parse auto value.bin --kind cbor
    grist transform value.cbor --to graph

## Security and fidelity

The decoders contain no execution, resolver, extension callback, dynamic
library, filesystem traversal, or network surface. CBOR tags and MessagePack
extensions are inert data. Protobuf descriptors are parsed as bounded wire
data; custom options are skipped inertly. Deprecated Protobuf groups are
preserved as raw fields with a partial diagnostic rather than executed or
silently discarded. The public registry catches panics at the parser boundary.

Focused universal contracts cover valid, concatenated, malformed, deep,
oversized, descriptor-missing, wrong-descriptor, required-field, extension,
unknown-field, detection, identity, graph, segment, schema, budget, and CLI
behavior.
