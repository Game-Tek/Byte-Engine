//! # Byte-Engine
//! Byte-Engine is a Rust powered game engine. It is designed to be efficient, fast and easy to use; with simple, composable patterns

#![feature(int_roundings)]
#![feature(ptr_sub_ptr)]
#![feature(iter_advance_by)]
#![warn(missing_docs)]
#![warn(missing_doc_code_examples)]

pub mod application;
pub mod orchestrator;
pub mod window_system;
pub mod render_system;
pub mod render_backend;
pub mod vulkan_render_backend;
pub mod render_debugger;
pub mod resource_manager;
pub mod shader_generator;
pub mod input_manager;
pub mod file_tracker;
pub mod beshader_compiler;
pub mod executor;
//pub mod gdeflate;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Extent {
	pub width: u32,
	pub height: u32,
	pub depth: u32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector2 {
	pub x: f32,
	pub y: f32,
}

impl Vector2 {
	pub fn new(x: f32, y: f32) -> Vector2 {
		Vector2 {
			x: x,
			y: y,
		}
	}

	pub fn zero() -> Vector2 {
		Vector2 {
			x: 0.0,
			y: 0.0,
		}
	}

	pub fn one() -> Vector2 {
		Vector2 {
			x: 1.0,
			y: 1.0,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vector3 {
	pub x: f32,
	pub y: f32,
	pub z: f32,
}

impl Vector3 {
	pub fn new(x: f32, y: f32, z: f32) -> Vector3 {
		Vector3 {
			x: x,
			y: y,
			z: z,
		}
	}

	pub fn min() -> Vector3 {
		Vector3 {
			x: f32::MIN,
			y: f32::MIN,
			z: f32::MIN,
		}
	}

	pub fn max() -> Vector3 {
		Vector3 {
			x: f32::MAX,
			y: f32::MAX,
			z: f32::MAX,
		}
	}

	pub fn zero() -> Vector3 {
		Vector3 {
			x: 0.0,
			y: 0.0,
			z: 0.0,
		}
	}

	pub fn one() -> Vector3 {
		Vector3 {
			x: 1.0,
			y: 1.0,
			z: 1.0,
		}
	}

	pub fn x_axis() -> Vector3 {
		Vector3 {
			x: 1.0,
			y: 0.0,
			z: 0.0,
		}
	}

	pub fn y_axis() -> Vector3 {
		Vector3 {
			x: 0.0,
			y: 1.0,
			z: 0.0,
		}
	}

	pub fn z_axis() -> Vector3 {
		Vector3 {
			x: 0.0,
			y: 0.0,
			z: 1.0,
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quaternion {
	pub x: f32,
	pub y: f32,
	pub z: f32,
	pub w: f32,
}

impl Quaternion {
	pub fn new(x: f32, y: f32, z: f32, w: f32) -> Quaternion {
		Quaternion {
			x: x,
			y: y,
			z: z,
			w: w,
		}
	}

	pub fn identity() -> Quaternion {
		Quaternion {
			x: 0.0,
			y: 0.0,
			z: 0.0,
			w: 1.0,
		}
	}

	pub fn min() -> Quaternion {
		Quaternion {
			x: f32::MIN,
			y: f32::MIN,
			z: f32::MIN,
			w: f32::MIN,
		}
	}

	pub fn max() -> Quaternion {
		Quaternion {
			x: f32::MAX,
			y: f32::MAX,
			z: f32::MAX,
			w: f32::MAX,
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