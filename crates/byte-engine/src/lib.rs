//! # Byte-Engine
//! Byte-Engine is a Rust powered game engine. It is designed to be efficient, fast and easy to use; with simple, composable patterns

#![feature(const_trait_impl, future_join, coerce_unsized, unsize, once_cell_try, iter_map_windows, slice_pattern, trait_alias, associated_type_defaults)]
#![feature(generic_const_exprs)] // https://github.com/rust-lang/rust/issues/133199
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![deny(unused_must_use)]
#![deny(unused_features)]
// #![warn(missing_docs)] # Disable now because we are writing a lot of code
// #![warn(missing_doc_code_examples)] # Disable now because we are writing a lot of code

#[cfg(not(feature = "headless"))]
extern crate ahi;
#[cfg(not(feature = "headless"))]
extern crate ghi;
extern crate besl;
extern crate resource_management;
extern crate utils;

pub mod core;
pub mod application;
#[cfg(not(feature = "headless"))]
pub mod audio;
pub mod camera;
pub mod input;
#[cfg(not(feature = "headless"))]
pub mod ui;
#[cfg(not(feature = "headless"))]
pub mod window_system;

pub mod constants;

pub mod gameplay;
pub mod math;
pub mod physics;
#[cfg(not(feature = "headless"))]
pub mod rendering;

pub use maths_rs::{prelude::Base, Quatf, Vec2f, Vec3f};
use serde::{Deserialize, Serialize};
pub type Vector2 = Vec2f;
pub type Vector3 = Vec3f;
pub type Quaternion = Quatf;
