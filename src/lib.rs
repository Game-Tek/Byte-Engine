//! # Byte-Engine
//! Byte-Engine is a Rust powered game engine. It is designed to be efficient, fast and easy to use; with simple, composable patterns

#![feature(downcast_unchecked)]
#![feature(const_mut_refs)]
#![feature(is_sorted)]
#![feature(iter_map_windows)]
#![feature(fn_ptr_trait)]
#![feature(new_uninit)]
#![feature(trivial_bounds)]
#![feature(async_closure)]
#![feature(closure_lifetime_binder)]
#![feature(ptr_metadata)]
#![feature(buf_read_has_data_left)]
#![feature(generic_const_exprs)]
#![feature(unchecked_shifts)]
#![feature(duration_millis_float)]
#![feature(const_trait_impl, future_join)]
// #![feature(effects)]
#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![deny(unused_must_use)]
#![deny(unused_features)]
// #![warn(missing_docs)] # Disable now because we are writing a lot of code
// #![warn(missing_doc_code_examples)] # Disable now because we are writing a lot of code

extern crate ahi;
extern crate besl;
pub extern crate core;
extern crate ghi;
extern crate resource_management;
extern crate utils;

pub mod application;
pub mod audio;
pub mod camera;
pub mod input;
pub mod networking;
pub mod ui;
pub mod window_system;

pub mod gameplay;
pub mod math;
pub mod physics;
pub mod rendering;

pub use maths_rs::{prelude::Base, Quatf, Vec2f, Vec3f};
use serde::{Deserialize, Serialize};
pub type Vector2 = Vec2f;
pub type Vector3 = Vec3f;
pub type Quaternion = Quatf;