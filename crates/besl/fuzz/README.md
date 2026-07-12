# BESL fuzzing

The fuzz workspace exercises the public BESL parsing and compilation pipelines without adding fuzz-only dependencies to the engine workspace.

Install `cargo-fuzz`, then run the targets from `crates/besl`:

```sh
cargo install cargo-fuzz --locked
cargo fuzz run parse -- -dict=fuzz/dictionaries/besl.dict -max_len=4096 -timeout=5
cargo fuzz run compile -- -dict=fuzz/dictionaries/besl.dict -max_len=4096 -timeout=5
```

Use a bounded run for a quick smoke test:

```sh
cargo fuzz run parse -- -runs=1000 -max_len=4096
cargo fuzz run compile -- -runs=1000 -max_len=4096
```

`parse` accepts arbitrary UTF-8 source and checks the parser's root invariant. `compile` continues successfully lexed programs through VM compilation. It does not execute arbitrary programs because mutated control flow may not terminate.

The human-readable `.besl` inputs under `corpus` are committed as starting points. Hash-named corpus entries and crash artifacts produced by libFuzzer remain ignored.
