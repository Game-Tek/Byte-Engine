use math::{Quaternion, Vector2, Vector3};
use utils::RGBA;

use super::{action::InputValue, device_class::DeviceClassHandle, Types, Value};

/// An input trigger is a source of input on a device class/type. Such as the UP key on a keyboard or the left trigger on a gamepad.
pub(super) struct Trigger {
	/// The device class the input source is associated with.
	pub(super) device_class_handle: DeviceClassHandle,
	/// The name of the input source.
	pub(super) name: String,
	/// The type of the input source.
	pub(super) r#type: Types,
	/// The default value the input source will have when it's first registered and no events have been recorded for it.
	pub(super) default: Value,
}

#[derive(Copy, Clone)]
pub struct TriggerDescription<T: InputValue> {
	/// The value the input source will have when it's first registered and no events have been recorded for it.
	pub(super) default: T,
	/// The value the input source will have when it's released.
	rest: T,
	/// The minimum value the input source can have.
	min: T,
	/// The maximum value the input source can have.
	max: T,
}

impl <T: InputValue> TriggerDescription<T> {
	pub fn new(default: T, rest: T, min: T, max: T) -> Self {
		TriggerDescription {
			default,
			rest,
			min,
			max,
		}
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
		TriggerDescription::new(RGBA::new(0f32, 0f32, 0f32, 1f32), RGBA::new(0f32, 0f32, 0f32, 1f32), RGBA::new(0f32, 0f32, 0f32, 1f32), RGBA::new(1f32, 1f32, 1f32, 1f32))
	}
}

impl Default for TriggerDescription<Vector2> {
	fn default() -> Self {
		TriggerDescription::new(Vector2::new(0f32, 0f32), Vector2::new(0f32, 0f32), Vector2::new(-1f32, -1f32), Vector2::new(1f32, 1f32))
	}
}

impl Default for TriggerDescription<Vector3> {
	fn default() -> Self {
		TriggerDescription::new(Vector3::new(0f32, 0f32, 0f32), Vector3::new(0f32, 0f32, 0f32), Vector3::new(-1f32, -1f32, -1f32), Vector3::new(1f32, 1f32, 1f32))
	}
}

impl Default for TriggerDescription<Quaternion> {
	fn default() -> Self {
		TriggerDescription::new(Quaternion::identity(), Quaternion::identity(), Quaternion::identity(), Quaternion::identity())
	}
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug)]
/// Handle to an input trigger.
pub struct TriggerHandle(pub(super) u32);
