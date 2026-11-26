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

input_handle! {
	#[derive(Copy, Clone, PartialEq, Eq)]
	/// The [`DeviceClassHandle`] struct identifies a registered layout when adding
	/// triggers or creating concrete devices.
	DeviceClassHandle
}

/// The [`Device`] struct stores one concrete instance of a registered device
/// class.
///
/// A device carries its own control values, which is what lets several gamepads
/// share one class definition.
pub(super) struct Device {
	pub(super) device_class_handle: DeviceClassHandle,
}

input_handle! {
	#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, facet::Facet)]
	/// The [`DeviceHandle`] struct identifies the device whose trigger or action state
	/// is being read or updated.
	DeviceHandle
}

/// The `SetDeviceOutput` struct sets a value an application sends to a device, such as a controller's rumble or
/// light color, so feedback reaches the player through the controller in their hands.
///
/// Send it on the [`GraphicsApplication::messages`](crate::application::graphics::GraphicsApplication::messages)
/// channel with a device from
/// [`InputCollector::devices_by_class_name`](crate::input::InputCollector::devices_by_class_name). The device keeps
/// the value until it's set again, and the last value set in a tick wins. Output names follow trigger names, such
/// as `Gamepad.LowFrequencyRumble`; see [device outputs](/docs/reference/input#device-outputs) for every name, its
/// value type, and the controllers that take it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SetDeviceOutput {
	device: DeviceHandle,
	output: &'static str,
	value: super::Value,
}

impl SetDeviceOutput {
	/// Gives the output named `output` of `device` the value `value`, such as `Value::Float(0.5)` for
	/// `Gamepad.LowFrequencyRumble`.
	pub fn new(device: DeviceHandle, output: &'static str, value: impl Into<super::Value>) -> Self {
		Self {
			device,
			output,
			value: value.into(),
		}
	}

	/// The device the output belongs to.
	pub fn device(&self) -> DeviceHandle {
		self.device
	}

	/// The output's name, such as `Gamepad.Light`.
	pub fn output(&self) -> &'static str {
		self.output
	}

	/// The value the output takes.
	pub fn value(&self) -> super::Value {
		self.value
	}
}
