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

#![feature(const_trait_impl, coerce_unsized, unsize, iterator_try_collect, iter_collect_into)]
#![cfg_attr(feature = "headed", feature(allocator_api, future_join, slice_pattern, trait_alias))]
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
#[cfg(feature = "headed")]
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
#[cfg(feature = "network")]
pub mod network;
pub mod physics;
#[cfg(feature = "headed")]
pub mod rendering;
pub mod space;

pub mod inspector;

use serde::{Deserialize, Serialize};
