/// Registers the standard Byte Engine mouse device class.
///
/// # Triggers
///
/// - `Position`: Absolute window-relative position as a 2D vector from -1 to 1.
/// - `Movement`: Relative movement as a 2D vector normalized by the window size. Each record is one impulse.
/// - `LeftButton`: State of the left mouse button as a Boolean value.
/// - `RightButton`: State of the right mouse button as a Boolean value.
/// - `Scroll`: Vertical scroll amount as a float from -1 to 1. Each record is one impulse.
pub fn register_mouse_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let mouse_device_class_handle = registry.register_device_class("Mouse");

	registry.register_trigger(&mouse_device_class_handle, "Position", TriggerDescription::<Axis2>::default());
	registry.register_trigger(
		&mouse_device_class_handle,
		"Movement",
		TriggerDescription::<Axis2>::default().transient(),
	);
	registry.register_trigger(
		&mouse_device_class_handle,
		"LeftButton",
		TriggerDescription::<bool>::default(),
	);
	registry.register_trigger(
		&mouse_device_class_handle,
		"RightButton",
		TriggerDescription::<bool>::default(),
	);
	registry.register_trigger(
		&mouse_device_class_handle,
		"Scroll",
		TriggerDescription::<f32>::default().transient(),
	);

	mouse_device_class_handle
}

/// Registers the standard Byte Engine keyboard device class.
///
/// # Triggers
///
/// The class exposes Boolean triggers for `W`, `S`, `A`, `D`, `Space`, the four
/// arrow keys, `Escape`, and `Backspace`. The `Character` trigger emits typed
/// text as a `char`, one impulse per character.
pub fn register_keyboard_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let keyboard_device_class_handle = registry.register_device_class("Keyboard");

	for name in [
		"W",
		"S",
		"A",
		"D",
		"Space",
		"Up",
		"Down",
		"Left",
		"Right",
		"Escape",
		"Backspace",
	] {
		registry.register_trigger(&keyboard_device_class_handle, name, TriggerDescription::<bool>::default());
	}

	registry.register_trigger(
		&keyboard_device_class_handle,
		"Character",
		TriggerDescription::<char>::default().transient(),
	);

	keyboard_device_class_handle
}

/// Registers the standard Byte Engine gamepad device class.
///
/// # Triggers
///
/// - `LeftStick` and `RightStick`: 2D vectors from -1 to 1. Positive X is right and positive Y is up.
/// - `LeftTrigger` and `RightTrigger`: floats from 0 to 1.
/// - Face, bumper, stick, menu, and directional-pad buttons: Boolean values.
pub fn register_gamepad_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let gamepad_device_class_handle = registry.register_device_class("Gamepad");

	registry.register_trigger(
		&gamepad_device_class_handle,
		"LeftStick",
		TriggerDescription::<Axis2>::default(),
	);
	registry.register_trigger(
		&gamepad_device_class_handle,
		"RightStick",
		TriggerDescription::<Axis2>::default(),
	);

	registry.register_trigger(
		&gamepad_device_class_handle,
		"LeftTrigger",
		TriggerDescription::<f32>::default(),
	);
	registry.register_trigger(
		&gamepad_device_class_handle,
		"RightTrigger",
		TriggerDescription::<f32>::default(),
	);

	for name in [
		"A",
		"B",
		"X",
		"Y",
		"LeftBumper",
		"RightBumper",
		"LeftStickButton",
		"RightStickButton",
		"Select",
		"Start",
		"Guide",
		"DPadUp",
		"DPadDown",
		"DPadLeft",
		"DPadRight",
	] {
		registry.register_trigger(&gamepad_device_class_handle, name, TriggerDescription::<bool>::default());
	}

	gamepad_device_class_handle
}

use std::alloc::Allocator;

use super::{Axis2, InputCollector, device::DeviceClassHandle, trigger::TriggerDescription};
