# BESL fuzzing

Use this independent fuzz workspace to test the public BESL parsing and
compilation pipelines. Its dependencies don't affect the engine workspace.

## Run the fuzz targets

Install `cargo-fuzz`. Then run the targets from `crates/besl`:

```sh
cargo install cargo-fuzz --locked
cargo fuzz run parse -- -dict=fuzz/dictionaries/besl.dict -max_len=4096 -timeout=5
cargo fuzz run compile -- -dict=fuzz/dictionaries/besl.dict -max_len=4096 -timeout=5
cargo fuzz run grammar_compile -- -max_len=512 -timeout=5
```

For a quick smoke test, limit the number of runs:

```sh
cargo fuzz run parse -- -runs=1000 -max_len=4096
cargo fuzz run compile -- -runs=1000 -max_len=4096
cargo fuzz run grammar_compile -- -runs=1000 -max_len=512
```

## Weekly CI run

The `Fuzz BESL` workflow runs every target for 20 minutes each Sunday and keeps
the grown corpus in the Actions cache. To start it early, open the workflow in
the Actions tab and select **Run workflow**. If a target crashes, download the
`besl-fuzz-<target>-artifacts` artifact and replay the input with
`cargo fuzz run <target> <artifact>`.

## Choose a target

- `parse` passes arbitrary UTF-8 source to the parser and checks its root invariant.
- `compile` sends successfully lexed programs through VM compilation.
- `grammar_compile` converts arbitrary bytes into bounded grammar choices. It
  emits a type-correct BESL program, then requires parsing, lexing, and VM
  compilation to succeed.

The targets don't execute arbitrary programs because mutated control flow might
not terminate.

## Manage the corpus

Keep the human-readable `.besl` inputs and structured-generator `.seed` inputs
under `corpus` as committed starting points. Leave hash-named corpus entries and
libFuzzer crash artifacts ignored.
