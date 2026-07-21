//! # Byte-Engine
//!
//! Byte-Engine is a composable Rust game engine for applications that need
//! graphics, input, audio, physics, and retained UI in one runtime.
//!
//! Headed applications usually start with `GraphicsApplication` and `default_setup`.
//! Lower-level users can compose [`application::BaseApplication`], factories,
//! channels, render passes, and UI layout pieces directly.
//!
//! ```no_run
//! use byte_engine::application::{Application, Parameter};
//! use byte_engine::application::graphics::{default_setup, GraphicsApplication};
//!
//! let mut application = GraphicsApplication::new("example", &[] as &[Parameter]);
//! default_setup(&mut application);
//! ```

#![feature(allocator_api, const_trait_impl, coerce_unsized, unsize)]
#![cfg_attr(feature = "headed", feature(future_join, slice_pattern, trait_alias))]
#![feature(generic_const_exprs)] // https://github.com/rust-lang/rust/issues/133199
#![allow(dead_code)]
#![allow(incomplete_features)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![deny(unsafe_code)]
#![deny(unused_must_use)]
#![deny(unused_features)]
#![deny(rustdoc::broken_intra_doc_links)]
// Existing engine APIs intentionally use domain-sized constructors and acronym-heavy graphics names; keep Clippy quiet until those APIs are redesigned.
#![allow(
	clippy::items_after_test_module,
	clippy::large_enum_variant,
	clippy::legacy_numeric_constants,
	clippy::doc_lazy_continuation,
	clippy::empty_line_after_doc_comments,
	clippy::extra_unused_lifetimes,
	clippy::manual_clamp,
	clippy::match_like_matches_macro,
	clippy::module_inception,
	clippy::needless_range_loop,
	clippy::needless_maybe_sized,
	clippy::new_ret_no_self,
	clippy::new_without_default,
	clippy::non_canonical_partial_ord_impl,
	clippy::only_used_in_recursion,
	clippy::ptr_arg,
	clippy::redundant_locals,
	clippy::redundant_pattern_matching,
	clippy::result_unit_err,
	clippy::tabs_in_doc_comments,
	clippy::too_many_arguments,
	clippy::type_complexity,
	clippy::upper_case_acronyms
)]

// #![warn(missing_docs)] # Disable now because we are writing a lot of code
// #![warn(missing_doc_code_examples)] # Disable now because we are writing a lot of code

#[cfg(feature = "headed")]
extern crate ahi;
#[cfg(feature = "headed")]
extern crate besl;
#[cfg(feature = "headed")]
extern crate ghi;
extern crate resource_management;
extern crate utils as engine_utils;

pub use math;
pub use time::MediaTime;

const ONLINE_DOCS_BASE_URL: &str = match option_env!("BYTE_ENGINE_DOCS_BASE_URL") {
	Some(url) => url,
	None => "https://byte-engine.0x44491229.dev/docs",
};

/// Builds a link to one online documentation page.
fn online_docs_url(path: &str) -> String {
	format!(
		"{}/{}",
		ONLINE_DOCS_BASE_URL.trim_end_matches('/'),
		path.trim_start_matches('/')
	)
}

/// The `utils` module provides engine utility types through the main `byte_engine` crate API.
pub mod utils {
	use std::{
		alloc::{GlobalAlloc, Layout, System},
		sync::atomic::{AtomicUsize, Ordering},
	};

	pub use crate::engine_utils::*;

	static ALLOCATION_COUNT: AtomicUsize = AtomicUsize::new(0);

	/// Use this allocator to track memory allocations made.
	pub struct CountingAllocator;

	#[allow(unsafe_code)]
	unsafe impl GlobalAlloc for CountingAllocator {
		unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
			ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
			unsafe { System.alloc(layout) }
		}

		unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
			unsafe { System.dealloc(ptr, layout) };
		}

		unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
			ALLOCATION_COUNT.fetch_add(1, Ordering::Relaxed);
			unsafe { System.realloc(ptr, layout, new_size) }
		}
	}

	/// Call this function to get the current allocation count of the global counting allocator.
	pub fn allocation_count() -> usize {
		ALLOCATION_COUNT.load(Ordering::Relaxed)
	}
}

pub mod application;
#[cfg(feature = "headed")]
pub mod audio;
pub mod core;
pub mod input;
#[cfg(feature = "headed")]
pub mod ui;

pub mod constants;

pub mod gameplay;
#[cfg(feature = "network")]
pub mod network;
pub mod physics;
#[cfg(feature = "headed")]
pub mod rendering;
pub mod space;
pub mod time;

pub mod inspector;

use serde::{Deserialize, Serialize};
