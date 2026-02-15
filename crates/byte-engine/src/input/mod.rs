use crate::core::factory::Handle;

use super::utils::RGBA;

pub mod input_manager;
mod gamepad;

pub mod device_class;
pub mod device;
pub mod input_trigger;
pub mod action;
pub mod utils;

pub use action::Action;
pub use action::ActionBindingDescription;

pub use input_manager::InputManager;
pub use input_trigger::TriggerHandle;
pub use device::DeviceHandle;
pub use action::ActionHandle;
use math::Quaternion;
use math::Vector2;
use math::Vector3;

use self::action::InputValue;

/// Enumerates the different types of types of values the input manager can handle.
#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Types {
	/// A boolean value.
	Boolean,
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

impl Into<Value> for bool {
	fn into(self) -> Value {
		Value::Bool(self)
	}
}

impl Into<Value> for char {
	fn into(self) -> Value {
		Value::Unicode(self)
	}
}

impl Into<Value> for f32 {
	fn into(self) -> Value {
		Value::Float(self)
	}
}

impl Into<Value> for i32 {
	fn into(self) -> Value {
		Value::Int(self)
	}
}

impl Into<Value> for RGBA {
	fn into(self) -> Value {
		Value::Rgba(self)
	}
}

impl Into<Value> for Vector2 {
	fn into(self) -> Value {
		Value::Vector2(self)
	}
}

impl Into<Value> for Vector3 {
	fn into(self) -> Value {
		Value::Vector3(self)
	}
}

impl Into<Value> for Quaternion {
	fn into(self) -> Value {
		Value::Quaternion(self)
	}
}

impl Value {
	/// Returns `true` if this value is equal to the default value for its type.
	///
	/// Used by `TickPolicy::WhileActive` to determine whether to emit events.
	pub fn is_default(&self) -> bool {
		match self {
			Value::Bool(v) => !v,
			Value::Unicode(v) => *v == '\0',
			Value::Float(v) => *v == 0.0,
			Value::Int(v) => *v == 0,
			Value::Rgba(v) => v.r == 0.0 && v.g == 0.0 && v.b == 0.0 && v.a == 0.0,
			Value::Vector2(v) => v.x == 0.0 && v.y == 0.0,
			Value::Vector3(v) => v.x == 0.0 && v.y == 0.0 && v.z == 0.0,
			Value::Quaternion(v) => *v == Quaternion::identity(),
		}
	}
}

impl Into<Types> for Value {
	fn into(self) -> Types {
		match self {
			Value::Bool(_) => Types::Boolean,
			Value::Unicode(_) => Types::Unicode,
			Value::Float(_) => Types::Float,
			Value::Int(_) => Types::Int,
			Value::Rgba(_) => Types::Rgba,
			Value::Vector2(_) => Types::Vector2,
			Value::Vector3(_) => Types::Vector3,
			Value::Quaternion(_) => Types::Quaternion,
		}
	}
}

impl Types {
	pub fn default_value(&self) -> Value {
		match self {
			Types::Boolean => Value::Bool(false),
			Types::Unicode => Value::Unicode('\0'),
			Types::Float => Value::Float(0.0),
			Types::Int => Value::Int(0),
			Types::Rgba => Value::Rgba(RGBA::new(0.0, 0.0, 0.0, 1.0)),
			Types::Vector2 => Value::Vector2(Vector2::new(0.0, 0.0)),
			Types::Vector3 => Value::Vector3(Vector3::new(0.0, 0.0, 0.0)),
			Types::Quaternion => Value::Quaternion(Quaternion::identity()),
		}
	}
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

/// The `TickPolicy` enum controls how frequently an action emits events through the event channel.
///
/// This allows applications to choose between event-driven and poll-driven input handling
/// on a per-action basis.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum TickPolicy {
	/// Emit events only when a trigger value actually changes. This is the default behavior.
	#[default]
	OnChange,
	/// Emit events every frame while the action's resolved value is non-default
	/// (e.g. while a key is held, while a stick is displaced from center).
	WhileActive,
	/// Emit events every frame unconditionally, regardless of the action's current value.
	Always,
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

/// The `ValueMapping` struct represents a how an input event value is mapped.
/// It allows for the transformation of input values using various functions.
///
/// Blanket implementations for `Into<ValueMapping>` exist for all types that implement `Into<Value>`. This implementations create a mapping with no transformation of the value.
#[derive(Copy, Clone, Debug)]
pub struct ValueMapping {
	pub(crate) function: Function,
	pub(crate) value: Value,
}

impl ValueMapping {
	pub fn new<V: Into<Value>>(function: Function, value: V) -> Self {
		Self { function, value: value.into() }
	}

	pub fn function(mut self, function: Function) -> Self {
		self.function = function;
		self
	}

	pub fn value(mut self, value: Value) -> Self {
		self.value = value;
		self
	}
}

impl Into<ValueMapping> for bool {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

impl Into<ValueMapping> for Vector2 {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

impl Into<ValueMapping> for Vector3 {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

impl Into<ValueMapping> for Quaternion {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

impl Into<ValueMapping> for f32 {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

impl Into<ValueMapping> for Value {
	fn into(self) -> ValueMapping {
		ValueMapping::new(Function::Linear, self)
	}
}

#[derive(Clone, Debug)]
pub struct ActionEvent {
	/// The handle of the action that triggered the event.
	handle: Handle,
	/// The value of the action that triggered the event.
	value: Value,
}

impl ActionEvent {
	pub fn new(handle: Handle, value: Value) -> Self {
		Self { handle, value }
	}

	pub fn handle(&self) -> Handle {
		self.handle.clone()
	}

	pub fn value(&self) -> Value {
		self.value
	}
}
