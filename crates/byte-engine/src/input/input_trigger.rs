/// The `Trigger` struct stores one input source defined by a device class.
///
/// A trigger can represent a keyboard key, a gamepad control, or another named
/// source that produces an input [`Value`].
pub(super) struct Trigger {
	/// The device class that defines this trigger.
	pub(super) device_class_handle: DeviceClassHandle,
	/// The trigger name within its device class.
	pub(super) name: String,
	/// The value type produced by the trigger.
	pub(super) r#type: Types,
	/// The value used until the first input record arrives.
	pub(super) default: Value,
}

#[derive(Copy, Clone)]
pub struct TriggerDescription<T: InputValue> {
	/// The value used until the first input record arrives.
	pub(super) default: T,
	/// The value used when the control is released.
	rest: T,
	/// The minimum valid value.
	min: T,
	/// The maximum valid value.
	max: T,
}

impl<T: InputValue> TriggerDescription<T> {
	pub fn new(default: T, rest: T, min: T, max: T) -> Self {
		TriggerDescription { default, rest, min, max }
	}
}

impl Default for TriggerDescription<bool> {
	fn default() -> Self {
		TriggerDescription::new(false, false, false, true)
	}
}

impl Default for TriggerDescription<char> {
	fn default() -> Self {
		TriggerDescription::new('\0', '\0', '\0', '\u{10FFFF}')
	}
}

impl Default for TriggerDescription<f32> {
	fn default() -> Self {
		TriggerDescription::new(0f32, 0f32, 0f32, 1f32)
	}
}

impl Default for TriggerDescription<i32> {
	fn default() -> Self {
		TriggerDescription::new(0, 0, i32::MIN, i32::MAX)
	}
}

impl Default for TriggerDescription<RGBA> {
	fn default() -> Self {
		TriggerDescription::new(
			RGBA::new(0f32, 0f32, 0f32, 1f32),
			RGBA::new(0f32, 0f32, 0f32, 1f32),
			RGBA::new(0f32, 0f32, 0f32, 1f32),
			RGBA::new(1f32, 1f32, 1f32, 1f32),
		)
	}
}

impl Default for TriggerDescription<Vector2> {
	fn default() -> Self {
		TriggerDescription::new(
			Vector2::new(0f32, 0f32),
			Vector2::new(0f32, 0f32),
			Vector2::new(-1f32, -1f32),
			Vector2::new(1f32, 1f32),
		)
	}
}

impl Default for TriggerDescription<Vector3> {
	fn default() -> Self {
		TriggerDescription::new(
			Vector3::new(0f32, 0f32, 0f32),
			Vector3::new(0f32, 0f32, 0f32),
			Vector3::new(-1f32, -1f32, -1f32),
			Vector3::new(1f32, 1f32, 1f32),
		)
	}
}

impl Default for TriggerDescription<Quaternion> {
	fn default() -> Self {
		TriggerDescription::new(
			Quaternion::identity(),
			Quaternion::identity(),
			Quaternion::identity(),
			Quaternion::identity(),
		)
	}
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// The `TriggerHandle` struct identifies a trigger registered with an
/// [`crate::input::InputManager`].
pub struct TriggerHandle(pub(super) u32);

use math::{Quaternion, Vector2, Vector3};
use utils::RGBA;

use super::{action::InputValue, device_class::DeviceClassHandle, Types, Value};
