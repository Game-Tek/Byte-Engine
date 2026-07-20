# Byte Engine network fuzzing

Use this independent fuzz workspace to test the engine-owned raw BETP boundary.
The harness excludes sockets and channels while it covers framing, endpoint
routing, session transitions, at-most-once application delivery, and canonical
outbound encoding.

## Run the fuzz target

Install `cargo-fuzz`. Then run the target from `crates/byte-engine`:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo fuzz run raw_datagram -- -max_len=2048 -timeout=5
```

Before you submit protocol or network-adapter changes, run this bounded,
sanitizer-backed smoke test:

```sh
cargo fuzz run raw_datagram -- -runs=10000 -max_len=2048 -timeout=5
```

## Understand the checks

The target establishes real client and server pipelines. It passes arbitrary or
structured canonical datagrams through `raw bytes -> decode -> route/session ->
accepted payload -> encode` and checks that rejected traffic remains peer-local.
It also retries one valid packet after rejected traffic and verifies that matching
duplicate data is never delivered twice.
