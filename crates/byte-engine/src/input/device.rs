use utils::hash::HashMap;

use super::{device_class::DeviceClassHandle, TriggerHandle};

/// A device represents a particular instance of a device class. Such as the current keyboard, or a specific gamepad.
/// This is useful for when you want to have multiple devices of the same type. Such as multiple gamepads(player 0, player 1, etc).
pub(super) struct Device {
	pub(super) device_class_handle: DeviceClassHandle,
	pub(super) index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// Handle to an device.
pub struct DeviceHandle(pub(super) u32);