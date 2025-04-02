/// A device class represents a type of device. Such as a keyboard, mouse, or gamepad.
/// It can have associated input sources, such as the UP key on a keyboard or the left trigger on a gamepad.
pub(super) struct DeviceClass {
	/// The name of the device class.
	pub(super) name: String,
}

#[derive(Copy, Clone, PartialEq, Eq)]
/// Handle to an input device class.
pub struct DeviceClassHandle(pub(super) u32);