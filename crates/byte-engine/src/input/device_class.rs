//! Input device layouts shared by concrete devices.
//!
//! Register classes and their triggers before creating devices. Most
//! applications use the predefined layouts in [`crate::input::utils`]; custom
//! hardware integrations can register a class directly through
//! [`crate::input::InputManager`].

/// The [`DeviceClass`] struct groups the trigger layout shared by one category of
/// input devices.
pub(super) struct DeviceClass {
	/// The name of the device class.
	pub(super) name: String,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// The [`DeviceClassHandle`] struct identifies a registered layout when adding
/// triggers or creating concrete devices.
pub struct DeviceClassHandle(pub(super) u32);
