# Byte Engine network fuzzing

This independent fuzz workspace exercises the engine-owned raw BETP boundary. It keeps sockets and channels out of the harness while covering framing, endpoint routing, session transitions, at-most-once application delivery, and canonical outbound encoding.

Install `cargo-fuzz`, then run the target from `crates/byte-engine`:

```sh
cargo install cargo-fuzz --version 0.13.2 --locked
cargo fuzz run raw_datagram -- -max_len=2048 -timeout=5
```

Use a bounded sanitizer-backed smoke run before submitting protocol or network-adapter changes:

```sh
cargo fuzz run raw_datagram -- -runs=10000 -max_len=2048 -timeout=5
```

The target establishes real client and server pipelines, passes arbitrary or structured canonical datagrams through `raw bytes -> decode -> route/session -> accepted payload -> encode`, and checks that rejected traffic remains peer-local. It also retries one valid packet after rejected traffic and verifies that duplicate matching data is never delivered twice.
