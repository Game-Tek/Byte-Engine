//! # Byte-Engine
//! Byte-Engine is a Rust powered game engine. It is designed to be efficient, fast and easy to use; with simple, composable patterns

#![feature(
	const_trait_impl,
	future_join,
	coerce_unsized,
	unsize,
	slice_pattern,
	trait_alias,
	iterator_try_collect,
	iter_collect_into,
	allocator_api
)]
#![feature(generic_const_exprs)] // https://github.com/rust-lang/rust/issues/133199
#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![deny(unsafe_code)]
#![deny(unused_must_use)]
#![deny(unused_features)]
#![deny(rustdoc::broken_intra_doc_links)]

// #![warn(missing_docs)] # Disable now because we are writing a lot of code
// #![warn(missing_doc_code_examples)] # Disable now because we are writing a lot of code

#[cfg(feature = "headed")]
extern crate ahi;
extern crate besl;
#[cfg(feature = "headed")]
extern crate ghi;
extern crate resource_management;
extern crate utils;

pub use math;

pub mod application;
#[cfg(feature = "headed")]
pub mod audio;
pub mod core;
pub mod input;
#[cfg(feature = "headed")]
pub mod ui;

pub mod constants;

pub mod gameplay;
pub mod network;
pub mod physics;
#[cfg(feature = "headed")]
pub mod rendering;
pub mod space;

pub mod inspector;

use serde::{Deserialize, Serialize};
