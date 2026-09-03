# Text decoding contract

Grist decodes byte inputs through `grist::decode::decode_text`. The returned
`DecodedText` owns both the exact original bytes and the decoded Rust string;
its `DecodeReport` is the serializable evidence record defined by
`grist/text-decode/v1`.

## Supported encodings and selection

The dependency-free decoder supports UTF-8, UTF-16LE/BE, UTF-32LE/BE, their
byte-order marks, and Windows-1252. The `extended-encodings` feature enables
the additional labels and decoding algorithms in the WHATWG Encoding Standard
through `encoding_rs`; `full` enables this feature. If such a declaration is
selected without the feature, decoding returns an explicit
`decode.encoding.unsupported` error instead of guessing.

Selection is deterministic. A BOM has highest precedence, followed by a
caller-supplied transport `charset`, an XML byte signature, an in-document
HTML `meta` or XML declaration, and finally a context-sensitive default or
heuristic. HTML declarations use WHATWG label aliases, including the mapping
of ISO-8859-1 labels to Windows-1252. XML UTF-16/32 declarations inherit the
endianness established by a BOM or XML signature. Contradictory lower-priority
evidence is retained and emits `decode.encoding.conflict`.

Without explicit evidence, valid UTF-8 is selected. Invalid text with at least
one valid UTF-8 multibyte scalar is recovered as mixed UTF-8 so the valid
scalars survive and each malformed byte sequence receives its own replacement
range. Otherwise text-like legacy bytes select Windows-1252. Detection uses
this same decoder after its binary-control-byte guard.

## Fidelity and diagnostics

Decoding never normalizes line endings. `NewlineFidelity.sequences` records
every LF, CRLF, and CR in order with half-open original-byte and decoded-UTF-8
byte ranges, counts each form, and records whether the final line terminates.

Each undecodable sequence, partial UTF code unit, unsupported ignored
declaration, and encoding conflict is a `DecodeIssue`. Ranges in
`raw_range` address the exact original bytes. When decoded coordinates exist,
`decoded_range` addresses the resulting UTF-8 string and the corresponding
diagnostic carries an exact `SourceLocator`; conflict issues also retain the
winning evidence range. Replacement and conflict diagnostics mark parser
envelopes `partial` rather than presenting recovered output as lossless.

The report hashes the original byte sequence and the exact decoded string
separately. `raw_identity` is reproducible from `DecodedText::raw_bytes()`, and
`decoded_identity` hashes the UTF-8 bytes of `DecodedText::text`, including any
U+FFFD replacements. Registry parsers consume this shared decoded text and
publish the decoded identity and diagnostics on their outer envelope.
