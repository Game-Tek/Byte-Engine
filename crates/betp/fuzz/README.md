# BETP fuzzing

This independent fuzz workspace exercises BETP's sans-I/O wire decoder and the client and server session state machines. Malformed packets and protocol errors are expected outcomes; panics, arithmetic overflow, excessive per-packet work, and unbounded state growth are not.

Install `cargo-fuzz`, then run the targets from `crates/betp`:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo fuzz run decode_packet -- -dict=fuzz/dictionaries/betp.dict -max_len=2048 -timeout=5
cargo fuzz run client_session -- -max_len=8192 -timeout=5
cargo fuzz run server_session -- -max_len=8192 -timeout=5
```

Use bounded runs for quick sanitizer-backed smoke tests:

```sh
cargo fuzz run decode_packet -- -runs=10000 -dict=fuzz/dictionaries/betp.dict -max_len=2048 -timeout=5
cargo fuzz run client_session -- -runs=10000 -max_len=8192 -timeout=5
cargo fuzz run server_session -- -runs=10000 -max_len=8192 -timeout=5
```

`decode_packet` passes arbitrary datagrams through the production decoder and checks that every accepted packet has a canonical wire round trip. `client_session` exercises every client lifecycle state with raw and structured hostile packets. `server_session` exercises the accepted and post-accept server lifecycle; server handshake ownership currently lives above BETP, so this target begins at `Session::accept` instead of duplicating transport logic in the harness.

The named `.betp` files under `corpus/decode_packet` seed every canonical packet type, the reserved type, an unknown type, and truncated packet boundaries. Keep these curated seeds in source control; hash-named corpus entries are local discoveries and remain ignored.

Regenerate the decoder seeds from the fuzz workspace with:

```sh
rustc scripts/generate_decode_corpus.rs -o target/generate_decode_corpus
target/generate_decode_corpus
```

The session targets cap each input at 64 operations, 8 packets per batch, and 512 update calls. These bounds keep fuzzing focused on protocol work rather than allowing a generated test case itself to consume unbounded time. The fuzz release profile retains overflow checks so integer wrap remains a crash instead of becoming invisible in optimized builds.
