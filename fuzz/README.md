# Grist fuzz surface

`universal_bytes` sends arbitrary bytes through ranked detection, loss-aware
text decoding, CBOR and MessagePack decoding, and (for UTF-8 inputs) the
model-output repair boundary. The target is intentionally bounded by each
public parser's normal resource limits and must never panic.

Run the checked seed regressions in normal CI:

```text
cargo test --all-features --test universal_fuzz_regressions
```

Run mutation fuzzing when `cargo-fuzz` is installed:

```text
cargo fuzz run universal_bytes fuzz/corpus/universal_bytes
```

New downstream parser regressions should first land as a minimal licensed or
synthetic seed, then remain in the normal deterministic regression test.
