# Parser and provider registries

`grist::registry` is the single runtime discovery and dispatch boundary for
built-in and caller-supplied parsers, providers, and isolated parser backends.
Its snapshots are deterministically ordered by registration ID and include
format aliases, media types, extensions, payload and option schema identities,
feature requirements, capabilities, provider requirements, origin, and
priority.

## Selection and conflicts

Parser IDs are unique across available and unavailable entries. Format,
media-type, extension, and alias selectors are normalized to lowercase;
format hyphens normalize to underscores, leading extension dots are ignored,
and MIME parameters are ignored. A selector cannot name two different
canonical formats. Multiple implementations may serve one format only at
different priorities, and the highest priority wins. Equal-priority
registrations fail instead of using insertion order. Built-ins use priority 0;
caller descriptors default to 100 and may choose another explicit value.

Provider IDs are unique. Providers of one kind may coexist only at different
priorities. Selection is inspectable, but a parse request must still explicitly
bind a provider and its network permission. There is no implicit provider or
network selection.

## Unsupported behavior

Feature-disabled and recognized-but-unimplemented built-ins remain in the
unavailable inventory. Selecting or dispatching one produces an
`unsupported` envelope with a `grist.content.unsupported` diagnostic. Unknown
formats use the same explicit status without claiming a parser exists.

## Extension boundary

Caller parsers receive `ParserContext`, not an unchecked envelope constructor
or the raw provider set. The context exposes resolved bytes, source metadata,
typed JSON options, cancellation checkpoints, budget charging, and a
provider-call wrapper. Provider calls are permitted only when declared by the
parser registration and explicitly selected in the request; elapsed provider
time is charged to the shared operation budget.

The registry catches parser panics, verifies status/payload consistency,
requires attributed `ProviderInvocation` metadata for every provider used,
constructs parser/source/options/provenance envelope fields, attaches raw
content identity, validates the envelope, and charges serialized output bytes.
An extension therefore cannot relabel a parser, erase provider attribution,
fabricate complete output without a payload, or bypass the shared output and
provider-time gates.

Isolated backends are provider registrations of kind
`isolated_parser_backend`. They must declare a private temporary filesystem, a
positive subprocess limit, and no active-content execution. Their network
policy is still enforced when the caller explicitly binds the backend.
