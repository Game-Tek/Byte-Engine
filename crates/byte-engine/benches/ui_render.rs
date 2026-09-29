//! Compile the engine sources in this harness to benchmark private CPU stages.
//! This keeps benchmark access out of the public UI API.

#![feature(allocator_api, const_trait_impl, coerce_unsized, trait_alias, unsize)]
#![feature(clone_from_ref, context_ext, generic_const_exprs, local_waker)]
#![allow(incomplete_features, unused_attributes)]

extern crate utils as engine_utils;

// Checking this harness enables `cfg(test)` without a test harness, so the `#[test]` functions are stripped and the
// helpers and imports only they use look unused. The library's own targets report its dead code.
#[path = "../src/lib.rs"]
#[allow(dead_code, unused_imports, unused_macros)]
mod library;
pub use library::*;

fn main() {
	divan::main();
}
