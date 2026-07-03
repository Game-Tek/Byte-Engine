//! Concrete input devices created from registered device classes.
//!
//! A device carries independent trigger state, which allows multiple gamepads
//! using one class definition. Create devices through
//! [`crate::input::InputManager`] after registering their class.

use utils::hash::HashMap;

use super::{device_class::DeviceClassHandle, TriggerHandle};

/// The [`Device`] struct stores one concrete instance of a registered device
/// class.
pub(super) struct Device {
	pub(super) device_class_handle: DeviceClassHandle,
	pub(super) index: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The [`DeviceHandle`] struct identifies the device whose trigger or action state
/// is being read or updated.
pub struct DeviceHandle(pub(super) u32);
