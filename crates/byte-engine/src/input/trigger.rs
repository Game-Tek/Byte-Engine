/// The `TriggerRegistry` trait declares the controls an input source provides.
///
/// [`InputCollector`](super::InputCollector) implements it, so the helpers in
/// [`utils`](super::utils) register the standard classes on any collector.
/// Next, create devices of a registered class and record their values.
pub trait TriggerRegistry {
	/// Registers a named device class, such as `Keyboard`.
	///
	/// Use PascalCase for `name` so trigger paths stay consistent.
	fn register_device_class(&mut self, name: &str) -> DeviceClassHandle;

	/// Registers a named trigger on a device class.
	///
	/// `description` defines the trigger's initial value, its valid Rust type,
	/// and whether records are impulses. Use the returned [`TriggerHandle`] to
	/// bind actions or record values.
	fn register_trigger<T: InputValue + Into<Value>>(
		&mut self,
		device_class: &DeviceClassHandle,
		name: &str,
		description: TriggerDescription<T>,
	) -> TriggerHandle;
}

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

#[derive(Copy, Clone)]
pub struct TriggerDescription<T: InputValue> {
	/// The value used until the first input record arrives.
	pub(super) default: T,
	/// Marks a control whose records are individual impulses instead of a hold.
	pub(super) transient: bool,
	/// The value used when the control is released.
	rest: T,
	/// The minimum valid value.
	min: T,
	/// The maximum valid value.
	max: T,
}

impl<T: InputValue> TriggerDescription<T> {
	/// Describes a persistent control, such as a key or a stick axis.
	///
	/// Next, register it with
	/// [`TriggerRegistry::register_trigger`](crate::input::TriggerRegistry::register_trigger).
	pub fn new(default: T, rest: T, min: T, max: T) -> Self {
		TriggerDescription {
			default,
			transient: false,
			rest,
			min,
			max,
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

impl Default for TriggerDescription<Axis2> {
	fn default() -> Self {
		TriggerDescription::new(
			Axis2::new(0f32, 0f32),
			Axis2::new(0f32, 0f32),
			Axis2::new(-1f32, -1f32),
			Axis2::new(1f32, 1f32),
		)
	}
}

impl Default for TriggerDescription<Axis3> {
	fn default() -> Self {
		TriggerDescription::new(
			Axis3::new(0f32, 0f32, 0f32),
			Axis3::new(0f32, 0f32, 0f32),
			Axis3::new(-1f32, -1f32, -1f32),
			Axis3::new(1f32, 1f32, 1f32),
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

#[derive(Copy, Clone, PartialEq, Eq, Hash, Debug, facet::Facet)]
/// The `TriggerHandle` struct identifies a trigger registered with an
/// [`InputCollector`](crate::input::InputCollector).
pub struct TriggerHandle(pub(super) u32);

use math::Quaternion;
use utils::RGBA;

use super::{Axis2, Axis3, Types, Value, action::InputValue, device::DeviceClassHandle};
