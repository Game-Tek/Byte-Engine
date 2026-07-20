# BETP fuzzing

Use this independent fuzz workspace to test BETP's sans-I/O wire decoder and its
client and server session state machines. Treat malformed packets and protocol
errors as expected outcomes. Treat panics, arithmetic overflow, excessive
per-packet work, and unbounded state growth as failures.

## Run the fuzz targets

Install `cargo-fuzz`. Then run the targets from `crates/betp`:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo fuzz run decode_packet -- -dict=fuzz/dictionaries/betp.dict -max_len=2048 -timeout=5
cargo fuzz run client_session -- -max_len=8192 -timeout=5
cargo fuzz run server_session -- -max_len=8192 -timeout=5
```

For quick sanitizer-backed smoke tests, use bounded runs:

```sh
cargo fuzz run decode_packet -- -runs=10000 -dict=fuzz/dictionaries/betp.dict -max_len=2048 -timeout=5
cargo fuzz run client_session -- -runs=10000 -max_len=8192 -timeout=5
cargo fuzz run server_session -- -runs=10000 -max_len=8192 -timeout=5
```

## Choose a target

- `decode_packet` passes arbitrary datagrams through the production decoder. It
  checks that every accepted packet has a canonical wire round trip.
- `client_session` exercises every client lifecycle state with raw and structured
  hostile packets.
- `server_session` exercises the accepted and post-accept server lifecycle.
  Server handshake ownership lives above BETP, so this target starts at
  `Session::accept` instead of duplicating transport logic in the harness.

The session targets generate matching-session data and disconnects,
guaranteed-other-session traffic, and duplicate data. They don't rely on
independently generated IDs becoming equal by chance. Their semantic checks cover
matching-ID transitions, invalid-then-valid liveness, the 16-packet output bound,
output connection identity, and eventual retry quiescence.

## Manage the corpus

Keep the named `.betp` files under `corpus` in source control. They seed every
packet type and session lifecycle, including correlated data, disconnect, retry,
and timeout paths. Leave hash-named local discoveries ignored.

To regenerate the decoder seeds, run these commands from the fuzz workspace:

```sh
rustc scripts/generate_decode_corpus.rs -o target/generate_decode_corpus
target/generate_decode_corpus

rustc scripts/generate_session_corpus.rs -o target/generate_session_corpus
target/generate_session_corpus
```

The session targets cap each input at 64 operations, 8 packets per batch, and 512
update calls, including the final retry-quiescence checks. Keep these bounds so
generated cases remain focused on protocol work. The fuzz release profile keeps
overflow checks enabled, so integer wrap remains visible as a crash in optimized
builds.

## Mutation testing

After you change framing, connection-ID checks, acknowledgements, retry policy,
timeouts, or sequence wraparound, run the scoped BETP mutation suite from the
workspace root:

```sh
cargo install --locked cargo-mutants
cargo mutants --package byte-engine-betp
```

The checked-in `.cargo/mutants.toml` limits the default campaign to the six
protocol files that enforce these hostile-input contracts. If a mutant survives,
strengthen the deterministic tests with a semantic assertion before you consider
the protocol change hardened.
