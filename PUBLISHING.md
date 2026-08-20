# Publish Byte-Engine crates

Publish the internal support crates before you publish the public `byte-engine`
crate. This order lets crates.io resolve each dependency during `cargo publish`.

## Resolve release blockers

Do not publish any crate while `byte-engine-ghi` depends on the Git revision of `ash`. The Vulkan backend uses the unreleased `VK_EXT_descriptor_heap` bindings from that revision, and crates.io packages cannot depend on Git repositories. Wait for a compatible `ash` release or publish an owned bindings crate, then replace the Git dependency and verify GHI on Linux.

## Follow the publish order

Publish crates in dependency waves. You can publish crates within the same wave in any order, but wait for crates.io to index every crate before you continue to the next wave.

1. `byte-engine-utils`, `byte-engine-math`, `byte-engine-ahi`, and `byte-engine-besl-derive`
2. `byte-engine-besl`, `byte-engine-ghi`, and `byte-engine-betp`
3. `byte-engine-resource-management`
4. `byte-engine`

Don't publish `beld`. It is a workspace tool and has `publish = false`.

## Verify the workspace

Run these checks before publishing:

```sh
cargo fmt --check
cargo check -q --workspace
cargo nextest run --workspace
cargo test -q --doc --workspace
cargo clippy -q --workspace
cargo doc -q -p byte-engine --no-deps
cargo doc -q -p byte-engine --no-default-features --no-deps
cargo rustdoc -q -p byte-engine -- -D missing_docs
cargo rustc -q -p byte-engine -- -W missing_debug_implementations
```

Verify the first wave before you publish any internal crate:

```sh
cargo publish --dry-run -p byte-engine-utils
cargo publish --dry-run -p byte-engine-math
cargo publish --dry-run -p byte-engine-ahi
cargo publish --dry-run -p byte-engine-besl-derive
```

The remaining packages require their earlier internal dependencies on crates.io. After crates.io indexes each wave, run `cargo publish --dry-run -p <package>` for every package in the next wave. Inspect the packaged file list and archive size in the command output before you publish.
