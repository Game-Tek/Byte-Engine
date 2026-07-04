# Publishing

Byte-Engine publishes the public engine crate plus several internal support crates.
Publish internal crates first so crates.io can resolve `byte-engine` dependencies
during `cargo publish`.

## Publish Order

1. `byte-engine-utils`
2. `byte-engine-math`
3. `byte-engine-ahi`
4. `byte-engine-besl-derive`
5. `byte-engine-besl`
6. `byte-engine-resource-management`
7. `byte-engine-ghi`
8. `byte-engine-betp`
9. `byte-engine`

`beld` is a workspace tool and is marked `publish = false`.

## Verification

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

Leaf packages can be verified before any internal crate is published:

```sh
cargo package -p byte-engine-utils
cargo package -p byte-engine-math
cargo package -p byte-engine-ahi
cargo package -p byte-engine-besl-derive
```

The remaining packages require their earlier internal dependencies to exist on
crates.io before `cargo package` or `cargo publish --dry-run` can complete.
After publishing each earlier crate, verify the next package in the order above
before publishing it.
