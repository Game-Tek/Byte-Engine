use utils::RGBA;
use crate::{Quaternion, Vector2, Vector3};

pub mod input_manager;
pub mod action;

pub use action::Action;
pub use action::ActionBindingDescription;

pub use input_manager::InputManager;
pub use input_manager::DeviceHandle;

use self::action::InputValue;

/// Enumerates the different types of types of values the input manager can handle.
#[derive(Copy, Clone)]
pub enum Types {
	/// A boolean value.
	Bool,
	/// A unicode character.
	Unicode,
	/// A floating point value.
	Float,
	/// An integer value.
	Int,
	/// A 2D point value.
	Vector2,
	/// A 3D point value.
	Vector3,
	/// A quaternion.
	Quaternion,
	/// An RGBA color value.
	Rgba,
}

#[derive(Debug, Copy, Clone, PartialEq)]
/// A simple "typeless" container for several underlying types.
/// Can be used to store any of these types, but will be usually used to traffic record and input event values.
pub enum Value {
	/// A boolean value.
	Bool(bool),
	/// A unicode character.
	Unicode(char),
	/// A floating point value.
	Float(f32),
	/// An integer value.
	Int(i32),
	/// An RGBA color value.
	Rgba(RGBA),
	/// A 2D point value.
	Vector2(Vector2),
	/// A 3D point value.
	Vector3(Vector3),
	/// A quaternion.
	Quaternion(Quaternion),
}

#[derive(Copy, Clone, Debug)]
/// Enumerates the different functions that can be applied to an input event.
pub enum Function {
	Boolean,
	Threshold,
	Linear,
	/// Maps a 2D point to a 3D point on a sphere.
	Sphere,
}

pub trait Extract<T: InputValue> {
	fn extract(&self) -> T;
}

impl Extract<bool> for Value {
	fn extract(&self) -> bool {
		match self {
			Value::Bool(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}

impl Extract<Vector2> for Value {
	fn extract(&self) -> Vector2 {
		match self {
			Value::Vector2(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}

impl Extract<Vector3> for Value {
	fn extract(&self) -> Vector3 {
		match self {
			Value::Vector3(value) => *value,
			_ => panic!("Wrong type")
		}
	}
}