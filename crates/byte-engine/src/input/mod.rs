//! Device-independent input actions and device registration.
//!
//! Typical headed applications call `setup_default_input`, translate window
//! events with `process_default_window_input`, and create application-level
//! [`Action`] values through the graphics application's action factory. Use
//! [`utils`] when registering the standard mouse, keyboard, or gamepad classes
//! in a custom application.
//!
//! [`InputManager`] owns device and action state. [`Value`] is the erased value
//! passed through that runtime; typed action declarations use
//! [`action::InputValue`] to constrain supported value types.

use super::utils::RGBA;
use crate::core::factory::Handle;

mod action_evaluator;
pub(crate) mod gamepad;
#[doc(hidden)]
pub mod input_manager;
mod records;

#[doc(hidden)]
pub mod action;
#[doc(hidden)]
pub mod device;
#[doc(hidden)]
pub mod device_class;
#[doc(hidden)]
pub mod input_trigger;
mod seat;
#[doc(hidden)]
pub mod utils;

pub use action::Action;
pub use action::ActionBindingDescription;
pub use action::ActionHandle;
pub use device::DeviceHandle;
pub use input_manager::InputManager;
pub use input_trigger::TriggerHandle;
use math::Quaternion;
use math::Vector2;
use math::Vector3;
pub use seat::SeatHandle;

use self::action::InputValue;

/// The [`Types`] enum identifies the value representation accepted by input
/// triggers and actions.
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
/// The [`Value`] enum carries device and action values through the non-generic
/// input runtime.
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

impl From<bool> for Value {
	fn from(val: bool) -> Self {
		Value::Bool(val)
	}
}

impl From<char> for Value {
	fn from(val: char) -> Self {
		Value::Unicode(val)
	}
}

impl From<f32> for Value {
	fn from(val: f32) -> Self {
		Value::Float(val)
	}
}

impl From<i32> for Value {
	fn from(val: i32) -> Self {
		Value::Int(val)
	}
}

impl From<RGBA> for Value {
	fn from(val: RGBA) -> Self {
		Value::Rgba(val)
	}
}

impl From<Vector2> for Value {
	fn from(val: Vector2) -> Self {
		Value::Vector2(val)
	}
}

impl From<Vector3> for Value {
	fn from(val: Vector3) -> Self {
		Value::Vector3(val)
	}
}

impl From<Quaternion> for Value {
	fn from(val: Quaternion) -> Self {
		Value::Quaternion(val)
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

impl From<Value> for Types {
	fn from(val: Value) -> Self {
		match val {
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
	/// Returns the neutral value used before a trigger or action produces input.
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

/// The `Function` enum identifies how an input value is transformed before it
/// reaches an action.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Function {
	/// Treats the mapped input as an on/off value.
	Boolean,
	/// Converts the mapped input to a boolean using a threshold.
	Threshold,
	/// Passes the mapped value through without curve-specific remapping.
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

/// The `Extract` trait exists to recover typed values from the input runtime's
/// erased [`Value`] representation.
pub trait Extract<T: InputValue> {
	/// Returns the stored value as the requested input type.
	fn extract(&self) -> T;
}

impl Extract<bool> for Value {
	fn extract(&self) -> bool {
		match self {
			Value::Bool(value) => *value,
			_ => panic!("Wrong type"),
		}
	}
}

impl Extract<Vector2> for Value {
	fn extract(&self) -> Vector2 {
		match self {
			Value::Vector2(value) => *value,
			_ => panic!("Wrong type"),
		}
	}
}

impl Extract<Vector3> for Value {
	fn extract(&self) -> Vector3 {
		match self {
			Value::Vector3(value) => *value,
			_ => panic!("Wrong type"),
		}
	}
}

/// The `ValueMapping` struct exists to bind a trigger value to the transform
/// used before action evaluation.
///
/// `From` implementations for values use [`Function::Linear`] as the default
/// transform, so callers can pass plain values when no custom mapping is needed.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ValueMapping {
	pub(crate) function: Function,
	pub(crate) value: Value,
}

impl ValueMapping {
	/// Creates a mapping from a transform function and a compatible input value.
	pub fn new<V: Into<Value>>(function: Function, value: V) -> Self {
		Self {
			function,
			value: value.into(),
		}
	}

	/// Replaces the transform function used by this mapping.
	pub fn function(mut self, function: Function) -> Self {
		self.function = function;
		self
	}

	/// Replaces the input value used by this mapping.
	pub fn value(mut self, value: Value) -> Self {
		self.value = value;
		self
	}
}

impl From<bool> for ValueMapping {
	fn from(val: bool) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

impl From<Vector2> for ValueMapping {
	fn from(val: Vector2) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

impl From<Vector3> for ValueMapping {
	fn from(val: Vector3) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

impl From<Quaternion> for ValueMapping {
	fn from(val: Quaternion) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

impl From<f32> for ValueMapping {
	fn from(val: f32) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

impl From<Value> for ValueMapping {
	fn from(val: Value) -> Self {
		ValueMapping::new(Function::Linear, val)
	}
}

#[derive(Clone, Debug)]
/// The `ActionEvent` struct carries resolved action input to application code.
pub struct ActionEvent {
	/// The seat that triggered the action event.
	seat_handle: SeatHandle,
	/// The handle of the action that triggered the event.
	handle: Handle,
	/// The value of the action that triggered the event.
	value: Value,
}

impl ActionEvent {
	/// Creates an action event for dispatch through the input event channel.
	pub fn new(seat_handle: SeatHandle, handle: Handle, value: Value) -> Self {
		Self {
			seat_handle,
			handle,
			value,
		}
	}

	/// Returns the seat that produced the action value.
	pub fn seat_handle(&self) -> SeatHandle {
		self.seat_handle
	}

	/// Returns the action handle associated with the event.
	pub fn handle(&self) -> Handle {
		self.handle
	}

	/// Returns the resolved action value.
	pub fn value(&self) -> Value {
		self.value
	}
}
