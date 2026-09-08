//! Input device layouts and the concrete devices created from them.
//!
//! A class describes the trigger layout one category of hardware shares, and a
//! device is one instance of that class with its own control values. Register
//! classes through [`TriggerRegistry`](crate::input::TriggerRegistry), using the
//! predefined layouts in [`utils`](crate::input::utils) where they fit, then
//! create devices with
//! [`InputEvents::create_device`](crate::input::InputEvents::create_device).

/// The [`DeviceClass`] struct groups the trigger layout shared by one category of
/// input devices.
pub(super) struct DeviceClass<A: std::alloc::Allocator> {
	/// The name of the device class.
	pub(super) name: Box<str, A>,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// The [`DeviceClassHandle`] struct identifies a registered layout when adding
/// triggers or creating concrete devices.
pub struct DeviceClassHandle(pub(super) u32);

/// The [`Device`] struct stores one concrete instance of a registered device
/// class.
///
/// A device carries its own control values, which is what lets several gamepads
/// share one class definition.
pub(super) struct Device {
	pub(super) device_class_handle: DeviceClassHandle,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, facet::Facet)]
/// The [`DeviceHandle`] struct identifies the device whose trigger or action state
/// is being read or updated.
pub struct DeviceHandle(pub(super) u32);
