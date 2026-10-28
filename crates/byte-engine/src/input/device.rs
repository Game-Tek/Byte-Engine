//! Input device layouts and the concrete devices created from them.
//!
//! A class describes the trigger layout one category of hardware shares, and a
//! device is one instance of that class with its own control values. Register
//! classes with [`InputCollector::register_device_class`](crate::input::InputCollector::register_device_class), using the
//! predefined layouts in [`utils`](crate::input::utils) where they fit, then
//! create devices with
//! [`InputCollector::create_device`](crate::input::InputCollector::create_device).

/// The [`DeviceClass`] struct groups the trigger layout shared by one category of
/// input devices.
pub(super) struct DeviceClass<A: std::alloc::Allocator> {
	/// The name of the device class.
	pub(super) name: Box<str, A>,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// The [`DeviceClassHandle`] struct identifies a registered layout when adding
/// triggers or creating concrete devices.
#[repr(transparent)]
pub struct DeviceClassHandle(pub(super) std::num::NonZeroU32);

impl DeviceClassHandle {
	/// Encodes a bounded registration index as a nonzero identity.
	pub(super) fn from_index(index: u32) -> Self {
		assert!(
			index < super::HANDLE_TAG.get(),
			"Input handle capacity exceeded. A local index exhausted its 31-bit space. Reuse existing input declarations."
		);
		Self(super::HANDLE_TAG | index)
	}

	pub(super) const fn index(self) -> u32 {
		self.0.get() ^ super::HANDLE_TAG.get()
	}
}

const _: () = assert!(size_of::<Option<DeviceClassHandle>>() == size_of::<u32>());

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
#[repr(transparent)]
pub struct DeviceHandle(pub(super) std::num::NonZeroU32);

impl DeviceHandle {
	/// Encodes a bounded registration index as a nonzero identity.
	pub(super) fn from_index(index: u32) -> Self {
		assert!(
			index < super::HANDLE_TAG.get(),
			"Input handle capacity exceeded. A local index exhausted its 31-bit space. Reuse existing input declarations."
		);
		Self(super::HANDLE_TAG | index)
	}

	pub(super) const fn index(self) -> u32 {
		self.0.get() ^ super::HANDLE_TAG.get()
	}
}

const _: () = assert!(size_of::<Option<DeviceHandle>>() == size_of::<u32>());
