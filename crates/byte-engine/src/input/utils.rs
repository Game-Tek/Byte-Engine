/// Registers the standard Byte Engine pointer device class: the system cursor, as the platform moves it with
/// pointer acceleration. Bind it for interfaces and for anything the cursor points at; bind
/// [`register_mouse_device_class`]'s `Mouse` for camera control instead.
///
/// # Triggers
///
/// - `Position`: Absolute window-relative position as a 2D vector from -1 to 1.
/// - `Movement`: The cursor's movement after pointer acceleration, as a 2D vector with `y` growing upward. Each
///   record is one impulse. See [`ghi::window::PointingDevice::Pointer`] for the units.
/// - `LeftButton`, `RightButton`, `MiddleButton`: Whether the button is held, as a Boolean value.
/// - `Scroll`: How far the wheel turned, as a float. Each record is one impulse.
pub fn register_pointer_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let pointer_device_class_handle = registry.register_device_class("Pointer");
	registry.register_trigger(
		&pointer_device_class_handle,
		"Position",
		TriggerDescription::<Axis2>::default(),
	);
	register_mouse_triggers(registry, &pointer_device_class_handle);
	pointer_device_class_handle
}

/// Registers the standard Byte Engine mouse device class: the physical mouse before pointer acceleration, so a
/// movement always turns a camera by the same amount. Bind it for camera control and aiming; bind
/// [`register_pointer_device_class`]'s `Pointer` for interfaces instead.
///
/// # Triggers
///
/// - `Movement`: The mouse's movement before pointer acceleration, as a 2D vector with `y` growing upward. Each record
///   is one impulse. See [`ghi::window::PointingDevice::Mouse`] for the units.
/// - `LeftButton`, `RightButton`, `MiddleButton`: Whether the button is held, as a Boolean value.
/// - `Scroll`: How far the wheel turned, as a float. Each record is one impulse.
pub fn register_mouse_device_class<A: Allocator + Clone>(registry: &mut InputCollector<A>) -> DeviceClassHandle {
	let mouse_device_class_handle = registry.register_device_class("Mouse");
	register_mouse_triggers(registry, &mouse_device_class_handle);
	mouse_device_class_handle
}

/// Registers the movement, button, and wheel triggers the pointer and mouse classes share.
fn register_mouse_triggers<A: Allocator + Clone>(registry: &mut InputCollector<A>, class: &DeviceClassHandle) {
	registry.register_trigger(class, "Movement", TriggerDescription::<Axis2>::default().transient());
	for button in ["LeftButton", "RightButton", "MiddleButton"] {
		registry.register_trigger(class, button, TriggerDescription::<bool>::default());
	}
	registry.register_trigger(class, "Scroll", TriggerDescription::<f32>::default().transient());
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
