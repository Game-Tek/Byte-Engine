//! Compile the engine sources in this harness to benchmark private CPU stages.
//! This keeps benchmark access out of the public UI API.

#![feature(allocator_api, const_trait_impl, coerce_unsized, trait_alias, unsize)]
#![feature(clone_from_ref, generic_const_exprs)]
#![allow(incomplete_features, unused_attributes)]

extern crate utils as engine_utils;

#[path = "../src/lib.rs"]
mod library;
pub use library::*;

fn main() {
	divan::main();
}
