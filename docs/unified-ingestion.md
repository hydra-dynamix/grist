# Unified ingestion

`grist::ingest::Ingestor` is the common boundary for in-memory bytes, declared
UTF-8 strings, seekable readers, one-shot streams (including stdin), local
paths, and virtual compound members. Every adapter enters through
`ParseRequest`, resolves the exact original bytes once, runs ranked detection,
uses the selected registry parser, decodes text through `grist::decode`, and
returns the registry-owned envelope with `operation: ingest`.

The envelope identity retains the raw byte hash, decoded identity when text was
decoded, ranked detection candidates, selected format, and canonical payload
hash. Equivalent bytes and source hints therefore produce the same identity and
payload regardless of their I/O adapter. Detection ambiguity, recognized but
unavailable formats, I/O failures, decoding failures, parser failures,
cancellation, and exhausted budgets remain typed envelopes; callers do not
need to recover diagnostics from an opaque ingestion error.

## Single request

```rust
use grist::core::{
    BudgetProfile, BudgetSelection, Input, ParseRequest, ProviderSet, RequestId,
    SourceInfo,
};
use grist::ingest::Ingestor;

let request = ParseRequest::new(
    RequestId::new("upload/7")?,
    Input::bytes(b"# title\n"),
    SourceInfo::new("README.md"),
    BudgetSelection::Profile(BudgetProfile::UntrustedServiceV1),
    ProviderSet::none(),
);
let envelope = Ingestor::builtin()?.ingest(request)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

`Input::path` reads only the named local path. URI values in `SourceInfo` remain
labels and are never fetched. Seekable readers are rewound to byte zero; streams
are consumed once in bounded chunks.

## Streams and batches

`Ingestor::stream` takes an ordered request sequence plus one explicit
`BudgetSelection` and `CancellationToken`. The batch policy applies to every
item, so input, decoding, parsing, providers, and output all charge one shared
tracker. Each `StreamItem` has a contiguous sequence, the caller's `RequestId`,
the content identity, and the complete ingest envelope. Duplicate request IDs
are rejected before emission.

Exactly one terminal event closes the iterator. A budget hit is emitted as the
correlated item envelope and then closes the stream as `partial`; cancellation
is emitted as the correlated cancelled envelope and then closes it as
`cancelled`. Earlier identities are never rewritten. `Ingestor::batch` is only
`BatchResult::collect` over this stream, so batch and streaming behavior cannot
drift and input ordering remains stable.

The older `FileIngestReport` and `RepoIngestReport` types remain available for
the existing inventory-oriented CLI and repository adapter. New byte/file/
stream parsing should use `Ingestor` so it receives the complete parser,
decoder, registry, budget, and cancellation contract.

Repository traversal also exposes deterministic entry dispositions, explicit
symlink/submodule policies, and stable aggregate hashes; see
[`repository-ingestion.md`](repository-ingestion.md).
