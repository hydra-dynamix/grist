# iCalendar and vCard

The email-message feature provides inert parsers for iCalendar (icalendar,
ics) and vCard (vcard, vcf). Both use the shared raw-byte envelope, resource
budget, registry, detection, graph, segment, schema, and CLI contracts.

## iCalendar

The payload preserves unfolded content lines and their original folded source
spans, ordered parameters, raw and decoded values, the complete component tree,
and typed projections for events, tasks, journals, free/busy data, alarms, and
time zones. Events retain UIDs, recurrence IDs, UTC/floating/TZID-qualified
times, recurrence and exception sets, attendees and participation parameters,
organizers, relationships, categories, URLs, and URI or base64 attachments.
Time zones retain observances, offsets, names, recurrence, and source URLs.
Unknown and extension properties remain in the authoritative property list.

Calendar MIME entities are inventoried as attachments and parsed recursively
from bytes already present in the message. MIME parameters and METHOD are
evidence only.

## vCard

The payload accepts 2.1, 3.0, and 4.0-style records, including multiple cards.
It retains property groups, named and legacy bare parameters, RFC 6868 escapes,
line folding, vCard 2.1 quoted-printable continuation, structured names and
addresses, organizations, communications, preferences and types, dates,
related contacts, categories, notes, identifiers, URLs, and URI or base64
photo/logo/sound/key values. Unknown properties remain ordered in every card.

## Provenance and limits

Every content line carries its exact half-open decoded UTF-8 source range and a
one-based record_range. Components and cards carry exact enclosing ranges and
stable record ordinals. Graph nodes and derived segments reuse these locators.
Options and shared budgets bound input, decoded text, line size, records,
properties, components, nesting, child artifacts, and attachment bytes.
Limit hits and malformed structure return partial payloads with diagnostics.

## Security and loss behavior

Parsing has no calendar, contact-store, filesystem-materialization, provider,
or network operation. Scheduling methods, attendee RSVP state, alarms, URLs,
time-zone URLs, related contacts, and URI attachments are stored only.
External references always have resolved: false. Binary values are decoded
only for bounded identity evidence and are never opened or executed. Invalid
base64, recurrence, structure, temporal values, unresolved time zones, and
lossy decoding are explicit partial diagnostics; raw properties survive.

Graph projection emits events, time zones, attendees, recurrence rules,
contacts, communications, references, and attachment nodes. AttachmentOf and
explicit reference relations preserve source locators. The authoritative
payload remains in graph attributes.
