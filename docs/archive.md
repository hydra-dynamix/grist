# ZIP, ZIP64, and TAR archives

The archives feature enables the zip and tar registry formats and the
authoritative grist::formats::archive namespace. Parsing is inert: member
names and header fields are inventory data and no decoder writes files, follows
links, creates devices, executes content, or accesses the network.

Every member retains source order, an exact archive-member locator, raw SHA-256
identity when bytes are available, compressed and expanded sizes, compression
method, entry kind, CRC-32 where defined, ZIP64 use, and TAR mode/owner/time/link
facts. ZIP encryption and unsupported compression methods are explicit terminal
member outcomes. TAR symbolic/hard links, devices, FIFOs, and unknown special
types remain visible but are never materialized.

## Shared traversal policy

archive::builtin_decoder_registry() supplies ZIP and TAR decoders to
ContainerRecursor. The controller applies one archive security policy and one
resource budget across all descendants. It rejects absolute or parent-traversal
paths, Windows device/alternate-stream names, links and devices, and
case/trailing-dot cross-platform collisions. Declared member count, expanded
size, memory, and compression ratio are preflighted before decompression.
Nested ZIP and TAR members recurse through the same depth, member, expansion,
child, memory, time, and output limits.

The caller explicitly selects inventory_only, inline_payload, or
content_addressed storage. Inventory still hashes available bytes and recurses
into nested archives but retains no payload bytes. Inline mode retains bytes in
the result. Content-addressed mode requires a caller-supplied verified sink.
Artifact IDs and archive metadata are identical across all three modes because
storage representation and parse status are excluded from identity.

The typed parser emits grist/archive/v1 inside the common v2 envelope and
projects every member and descendant to DocumentGraph archive-member nodes.
The archive, archive-envelope, and archive-options schemas are registered
and checked in. grist ingest archive uses the same built-in decoder registry.
