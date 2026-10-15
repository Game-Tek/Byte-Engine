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
/// The class exposes a Boolean trigger for every key in [`KEYBOARD_KEYS`]: letters (`A`), digits (`Num0`), numpad
/// keys (`NumPad0`, `NumPadAdd`), modifiers (`LeftShift`, `RightControl`, `LeftAlt`), arrows (`Up`), function keys
/// (`F1`), and named keys such as `Space`, `Escape`, and `Backspace`. Bind them as `Keyboard.<name>`. The
/// `Character` trigger emits typed text as a `char`, one impulse per character.
pub fn register_keyboard_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let keyboard_device_class_handle = registry.register_device_class("Keyboard");

	for name in KEYBOARD_KEYS {
		registry.register_trigger(&keyboard_device_class_handle, name, TriggerDescription::<bool>::default());
	}

	registry.register_trigger(
		&keyboard_device_class_handle,
		"Character",
		TriggerDescription::<char>::default().transient(),
	);

	keyboard_device_class_handle
}

/// Names every key the standard keyboard device class exposes as a Boolean trigger.
///
/// Window input drives each of these; bind one in an action as `Keyboard.<name>`.
pub const KEYBOARD_KEYS: &[&str] = &[
	"A",
	"B",
	"C",
	"D",
	"E",
	"F",
	"G",
	"H",
	"I",
	"J",
	"K",
	"L",
	"M",
	"N",
	"O",
	"P",
	"Q",
	"R",
	"S",
	"T",
	"U",
	"V",
	"W",
	"X",
	"Y",
	"Z",
	"Num0",
	"Num1",
	"Num2",
	"Num3",
	"Num4",
	"Num5",
	"Num6",
	"Num7",
	"Num8",
	"Num9",
	"NumPad0",
	"NumPad1",
	"NumPad2",
	"NumPad3",
	"NumPad4",
	"NumPad5",
	"NumPad6",
	"NumPad7",
	"NumPad8",
	"NumPad9",
	"NumPadAdd",
	"NumPadSubtract",
	"NumPadMultiply",
	"NumPadDivide",
	"NumPadDecimal",
	"NumPadEnter",
	"Backspace",
	"Tab",
	"Enter",
	"LeftShift",
	"RightShift",
	"LeftControl",
	"RightControl",
	"LeftAlt",
	"RightAlt",
	"Menu",
	"Space",
	"Insert",
	"Delete",
	"Home",
	"End",
	"PageUp",
	"PageDown",
	"Up",
	"Down",
	"Left",
	"Right",
	"Escape",
	"F1",
	"F2",
	"F3",
	"F4",
	"F5",
	"F6",
	"F7",
	"F8",
	"F9",
	"F10",
	"F11",
	"F12",
	"NumLock",
	"ScrollLock",
	"CapsLock",
	"PrintScreen",
];

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
