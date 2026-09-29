/// The `TriggerReference` enum lets callers select a trigger by handle or name.
#[derive(Copy, Clone, Debug)]
pub enum TriggerReference {
	/// Selects a trigger by its registered handle.
	Handle(TriggerHandle),
	/// Selects a trigger by its `DeviceClass.Trigger` name.
	Name(&'static str),
}

/// The `Trigger` struct stores one input source defined by a device class.
///
/// A trigger can represent a keyboard key, a gamepad control, or another named
/// source that produces an input [`Value`].
pub(super) struct Trigger<A: std::alloc::Allocator> {
	/// The device class that defines this trigger.
	pub(super) device_class_handle: DeviceClassHandle,
	/// The `DeviceClass.Trigger` name records and bindings resolve against.
	pub(super) name: Box<str, A>,
	/// The value type produced by the trigger.
	pub(super) r#type: Types,
	/// The value used until the first input record arrives.
	pub(super) default: Value,
	/// Marks a control whose records are individual impulses instead of a hold.
	pub(super) transient: bool,
}

/// The `TriggerDescription` struct describes a control when it is registered with
/// [`InputCollector::register_trigger`](crate::input::InputCollector::register_trigger).
///
/// Use [`Default`] for the standard initial value of a type, or [`Self::new`] for a custom one.
#[derive(Copy, Clone)]
pub struct TriggerDescription<T: InputValue> {
	/// The value used until the first input record arrives.
	pub(super) default: T,
	/// Marks a control whose records are individual impulses instead of a hold.
	pub(super) transient: bool,
}

impl<T: InputValue> TriggerDescription<T> {
	/// Describes a persistent control, such as a key or a stick axis, that reads `default` until its first record.
	///
	/// Next, register it with
	/// [`InputCollector::register_trigger`](crate::input::InputCollector::register_trigger).
	pub fn new(default: T) -> Self {
		TriggerDescription {
			default,
			transient: false,
		}
	}

	/// Treats each record as a one-time impulse instead of a control being held.
	///
	/// Use it for wheel steps, relative motion, and text: an impulse never holds
	/// a sink's capture of the control and never repeats through a tick policy.
	pub fn transient(mut self) -> Self {
		self.transient = true;
		self
	}
}

impl Default for TriggerDescription<bool> {
	fn default() -> Self {
		Self::new(false)
	}
}

impl Default for TriggerDescription<char> {
	fn default() -> Self {
		Self::new('\0')
	}
}

impl Default for TriggerDescription<f32> {
	fn default() -> Self {
		Self::new(0.0)
	}
}

impl Default for TriggerDescription<i32> {
	fn default() -> Self {
		Self::new(0)
	}
}

impl Default for TriggerDescription<RGBA> {
	fn default() -> Self {
		Self::new(RGBA::new(0f32, 0f32, 0f32, 1f32))
	}
}

impl Default for TriggerDescription<Axis2> {
	fn default() -> Self {
		Self::new(Axis2::new(0f32, 0f32))
	}
}

impl Default for TriggerDescription<Axis3> {
	fn default() -> Self {
		Self::new(Axis3::new(0f32, 0f32, 0f32))
	}
}

impl Default for TriggerDescription<Quaternion> {
	fn default() -> Self {
		Self::new(Quaternion::identity())
	}
}

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, facet::Facet)]
/// The `TriggerHandle` struct identifies a trigger registered with an
/// [`InputCollector`](crate::input::InputCollector).
pub struct TriggerHandle(pub(super) u32);

use math::Quaternion;
use utils::RGBA;

use super::{Axis2, Axis3, Types, Value, action::InputValue, device::DeviceClassHandle};
