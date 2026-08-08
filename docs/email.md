# EML, RFC 5322, and MIME

The `email-message` feature enables the built-in `eml` parser for `.eml` and
`message/rfc822` inputs. Its authoritative payload schema is `grist/email/v1`.
The implementation is byte-oriented and does not use an email client, renderer,
network resolver, calendar agent, or executable MIME handler.

## Preserved structure

- Ordered, duplicate, folded, malformed, and unknown headers remain in source
  order with raw, unfolded, RFC 2047-decoded, and exact locator forms.
- Address fields remain textual facts while mailbox, display-name, comment, and
  group candidates are exposed. Date spellings and time zones remain verbatim.
- Message-ID, In-Reply-To, References, Received, Thread-Index, Thread-Topic,
  normalized-subject hints, DKIM, ARC, and authentication headers remain evidence.
- MIME types, RFC 2231 parameter continuations, dispositions, transfer encodings,
  preambles, epilogues, alternatives, text, inline resources, and attachments are typed.
- Each entity has a one-based MIME path plus an exact half-open byte/text range.
  Attachment identity is calculated over decoded attachment bytes.
- Supported attachment formats may be parsed recursively from supplied bytes
  under the shared input, child, nesting, memory, node, and time budgets.

## Options and limits

`EmailOptions` declares maximum header bytes, MIME depth, MIME entity count, and
decoded bytes per part. Limit hits are partial diagnostics. Attachment bytes are
inventory-only by default; callers may request inline capture. Recursive local
attachment parsing can be disabled.

## Security and loss behavior

No URI is dereferenced. HTTP(S) HTML attributes and remote `Content-Location`
facts are inventoried with `resolved: false`. HTML remains inert source text.
S/MIME encryption, MIME signatures, TNEF, scripts, executables, and active
attachments are data only. Encrypted bodies are explicit partial results unless
a separate authorized decryption operation supplies plaintext. Unsafe captured
bytes are quarantined by the shared embedded-artifact contract.

Stable diagnostics use the `email.header.*`, `email.limit.*`, `email.mime.*`,
and `email.nested.*` namespaces.

## MBOX mailboxes

The same feature also enables `grist::mbox` and `formats::mbox` for `.mbox` and
`application/mbox`. The authoritative `grist/mbox/v1` payload retains source
order, the complete `From ` separator spelling and line ending, raw and decoded
message hashes, mboxo/mboxrd quote-removal evidence, and mboxcl/mboxcl2
`Content-Length` boundary evidence. Each message embeds the EML payload with a
nested mailbox record locator.

`stream_mbox` emits one identity-bearing message event at a time and exactly one
terminal event. `parse_mbox` is a collector over those same events. Exact
duplicate messages receive occurrence suffixes while unrelated insertions do
not change existing message identities. Missing separators, invalid or
truncated lengths, MIME failures, cancellation, and message or shared resource
limits remain explicit partial/terminal conditions.

Mailbox thread evidence resolves Message-ID, In-Reply-To, and References facts
across messages without asserting semantic truth. Subject grouping remains a
separate textual hint. No separator, header, body, attachment, link, or remote
resource is executed or fetched.

## Outlook MSG

The `email-message` feature also enables the `msg` registry parser and the
`grist::outlook` / `formats::outlook` APIs. The authoritative payload schema is
`grist/outlook-msg/v1`. Detection requires both the Compound Binary File magic
and MSG property-directory evidence, so an arbitrary OLE compound document is
not classified as mail solely from its container signature.

The parser performs bounded MS-CFB v3/v4 traversal, including DIFAT, FAT,
mini-FAT, directory trees, mini streams, and cycle and truncation checks. It
then preserves message, recipient, and attachment MAPI property tables in
source order. Fixed and variable values, multi-values, GUIDs, FILETIME values,
Unicode and Windows code-page strings, named-property mappings, unknown property
types, property flags, stream paths, hashes, and optional raw bytes remain typed
evidence. Unknown storages and streams are retained as inert objects.

Recipients retain To/Cc/Bcc/originator roles and their complete property sets.
Sender facts, subject, message class, Internet Message-ID, In-Reply-To,
References, conversation topic/index, creation, modification, submit, delivery,
and client-submit dates are projected without treating them as verified claims.
Plain text, HTML, and RTF appear as distinct body alternatives. MS-OXRTFCP LZFu
and MELA data is decompressed under an output limit while declared sizes, CRCs,
and source bytes remain available for audit.

By-value attachments use the shared embedded-artifact inventory and quarantine
contract. Filenames, MIME types, content IDs/locations, rendering positions,
declared sizes, MAPI properties, and exact compound-object locators are kept.
Embedded-message attachments recurse through the same parsed compound file and
shared operation budget. The parser never follows by-reference paths, resolves
content locations, launches OLE objects, opens attachments, renders HTML, or
executes scripts and executables.

`OutlookMsgOptions` bounds properties per object, recipients, attachments,
embedded depth, compound-sector chains, and decompressed RTF output. Binary
property values, unknown streams, and attachment bytes are hash-and-length
inventory by default and can be captured inline explicitly. Embedded-message
parsing can be disabled. Invalid chains, truncated streams, malformed property
tables, decode loss, RTF size/CRC/decompression failures, and resource-limit
hits produce stable `outlook.msg.*` partial diagnostics. Encrypted S/MIME MSG
objects return an explicit partial payload with encrypted status evidence and no
decryption attempt; signed message classes remain inert evidence.

The MSG payload has deterministic DocumentGraph projection, segment projection,
checked-in payload/envelope/options schemas, registry aliases, content detection,
and CLI routing through `grist parse msg`.
