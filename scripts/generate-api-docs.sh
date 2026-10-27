#!/usr/bin/env bash

set -euo pipefail

# rustdoc JSON is unstable. This toolchain must produce the format version that
# RUSTDOC_FORMAT_VERSION in docs-site/src/lib/rustdoc/format.ts names; change both together.
readonly TOOLCHAIN="nightly-2026-08-31"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly JSON_TARGET="$REPOSITORY_ROOT/target/api-docs-json"
readonly OUTPUT="$REPOSITORY_ROOT/docs-site/generated/rust-api.json"

if ! rustup run "$TOOLCHAIN" rustc --version >/dev/null 2>&1; then
	printf 'Missing Rust toolchain %s. Install it with `rustup toolchain install %s`.\n' \
		"$TOOLCHAIN" "$TOOLCHAIN" >&2
	exit 1
fi

cd "$REPOSITORY_ROOT"

# The previous cargo-docs-md generator wrote MDX pages here. The docs site loads everything under docs/, so stale
# pages would sit next to the generated API pages; remove them once on checkouts that ran the old generator.
rm -rf "$REPOSITORY_ROOT/docs/api/latest"

# Generate only the public library API. Cargo still builds dependencies needed to
# analyze the crate, but rustdoc writes one JSON artifact for byte-engine.
# Hidden items are included because many public types are defined in
# `#[doc(hidden)]` modules and re-exported; the model builder drops the rest.
RUSTC_WRAPPER= CARGO_TARGET_DIR="$JSON_TARGET" \
	cargo +"$TOOLCHAIN" rustdoc \
		-p byte-engine \
		--lib \
		--all-features \
		-Z unstable-options \
		--output-format json \
		-- -Z unstable-options --document-hidden-items

bun "$REPOSITORY_ROOT/docs-site/scripts/build-rust-api-model.ts" \
	"$JSON_TARGET/doc/byte_engine.json" \
	"$OUTPUT"
