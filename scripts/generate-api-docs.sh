#!/usr/bin/env bash

set -euo pipefail

# rustdoc JSON and cargo-docs-md must agree on the unstable schema version.
readonly TOOLCHAIN="nightly-2026-05-31"
readonly CARGO_DOCS_MD_VERSION="0.2.4"
readonly SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
readonly REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
readonly JSON_TARGET="$REPOSITORY_ROOT/target/api-docs-json"
readonly TOOL_ROOT="$REPOSITORY_ROOT/target/api-docs-tools"
readonly GENERATOR="$TOOL_ROOT/bin/cargo-docs-md"
readonly OUTPUT="$REPOSITORY_ROOT/docs/api/latest"

if ! rustup run "$TOOLCHAIN" rustc --version >/dev/null 2>&1; then
	printf 'Missing Rust toolchain %s. Install it with `rustup toolchain install %s`.\n' \
		"$TOOLCHAIN" "$TOOLCHAIN" >&2
	exit 1
fi

if [[ ! -x "$GENERATOR" ]] || \
	[[ "$($GENERATOR docs-md --version 2>/dev/null)" != "cargo-docs-md $CARGO_DOCS_MD_VERSION" ]]; then
	RUSTC_WRAPPER= cargo +"$TOOLCHAIN" install cargo-docs-md \
		--version "$CARGO_DOCS_MD_VERSION" \
		--locked \
		--root "$TOOL_ROOT"
fi

cd "$REPOSITORY_ROOT"

# Generate only the public library API. Cargo still builds dependencies needed to
# analyze the crate, but rustdoc writes one JSON artifact for byte-engine.
RUSTC_WRAPPER= CARGO_TARGET_DIR="$JSON_TARGET" \
	cargo +"$TOOLCHAIN" rustdoc \
		-p byte-engine \
		--lib \
		--all-features \
		-Z unstable-options \
		--output-format json

readonly STAGING="$(mktemp -d "$JSON_TARGET/markdown.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT

"$GENERATOR" docs-md \
	--dir "$JSON_TARGET/doc" \
	--output "$STAGING" \
	--primary-crate byte_engine \
	--exclude-private \
	--full-method-docs \
	--source-locations \
	--no-mdbook \
	--no-search-index

bun "$SCRIPT_DIR/prepare-api-docs.mjs" "$STAGING"

mkdir -p "$OUTPUT"
# The output is generated and ignored. Clear it completely so removed pages and
# metadata cannot survive locally and hide clean-checkout failures.
find "$OUTPUT" -mindepth 1 -delete
cp -R "$STAGING/." "$OUTPUT/"

printf 'Generated API documentation in %s.\n' "$OUTPUT"
