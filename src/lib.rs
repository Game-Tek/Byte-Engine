//! # Byte-Engine
//! Byte-Engine is a Rust powered game engine. It is designed to be efficient, fast and easy to use; with simple, composable patterns

#![feature(downcast_unchecked)]
#![feature(const_mut_refs)]
#![feature(is_sorted)]
#![feature(iter_map_windows)]
// #![warn(missing_docs)] # Disable now because we are writing a lot of code
// #![warn(missing_doc_code_examples)] # Disable now because we are writing a lot of code

pub mod application;
pub mod orchestrator;
pub mod window_system;
pub mod render_debugger;
pub mod resource_manager;
pub mod shader_generator;
pub mod input_manager;
pub mod file_tracker;
pub mod executor;
pub mod camera;
pub mod render_domain;

pub mod math;
pub mod rendering;
pub mod gameplay;
pub mod jspd;
//pub mod gdeflate;

pub use maths_rs::{Vec2f, Vec3f, Quatf, prelude::Base};
use serde::{Serialize, Deserialize};
pub type Vector2 = Vec2f;
pub type Vector3 = Vec3f;
pub type Quaternion = Quatf;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Extent {
	pub width: u32,
	pub height: u32,
	pub depth: u32,
}

impl Extent {
	pub fn new(width: u32, height: u32, depth: u32) -> Self {
		Self {
			width,
			height,
			depth,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RGBA {
	pub r: f32,
	pub g: f32,
	pub b: f32,
	pub a: f32,
}

fn insert_return_length<T>(collection: &mut Vec<T>, value: T) -> usize {
	let length = collection.len();
	collection.push(value);
	return length;
}