# Publish Byte-Engine crates

Publish the internal support crates before you publish the public `byte-engine`
crate. This order lets crates.io resolve each dependency during `cargo publish`.

## Follow the publish order

1. `byte-engine-utils`
2. `byte-engine-math`
3. `byte-engine-ahi`
4. `byte-engine-besl-derive`
5. `byte-engine-besl`
6. `byte-engine-resource-management`
7. `byte-engine-ghi`
8. `byte-engine-betp`
9. `byte-engine`

Don't publish `beld`. It is a workspace tool and has `publish = false`.

## Verify the workspace

Run these checks before publishing:

```sh
cargo fmt --check
cargo check -q --workspace
cargo test -q --workspace
cargo clippy -q --workspace
cargo doc -q -p byte-engine --no-deps
cargo doc -q -p byte-engine --no-default-features --no-deps
cargo rustdoc -q -p byte-engine -- -D missing_docs
cargo rustc -q -p byte-engine -- -W missing_debug_implementations
```

You can verify the leaf packages before you publish any internal crate:

```sh
cargo package -p byte-engine-utils
cargo package -p byte-engine-math
cargo package -p byte-engine-ahi
cargo package -p byte-engine-besl-derive
```

The remaining packages require their earlier internal dependencies on crates.io.
After you publish each crate, verify the next package in the list before you
publish it. Otherwise, `cargo package` and `cargo publish --dry-run` can't finish.
