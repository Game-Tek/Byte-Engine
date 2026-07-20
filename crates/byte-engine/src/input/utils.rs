/// Registers the standard Byte Engine mouse device class.
///
/// # Triggers
///
/// - `Position`: Absolute window-relative position as a 2D vector from -1 to 1.
/// - `Movement`: Relative movement as a 2D vector normalized by the window size.
/// - `LeftButton`: State of the left mouse button as a Boolean value.
/// - `RightButton`: State of the right mouse button as a Boolean value.
/// - `Scroll`: Vertical scroll amount as a float from -1 to 1.
pub fn register_mouse_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
	let mouse_device_class_handle = input_manager.register_device_class("Mouse");

	input_manager.register_trigger(
		&mouse_device_class_handle,
		"Position",
		TriggerDescription::<Vector2>::default(),
	);
	input_manager.register_trigger(
		&mouse_device_class_handle,
		"Movement",
		TriggerDescription::<Vector2>::default(),
	);
	input_manager.register_trigger(
		&mouse_device_class_handle,
		"LeftButton",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&mouse_device_class_handle,
		"RightButton",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&mouse_device_class_handle,
		"Scroll",
		TriggerDescription::new(0f32, 0f32, -1f32, 1f32),
	);

	mouse_device_class_handle
}

/// Registers the standard Byte Engine keyboard device class.
///
/// # Triggers
///
/// The class exposes Boolean triggers for `W`, `S`, `A`, `D`, `Space`, the four
/// arrow keys, `Escape`, and `Backspace`. The `Character` trigger emits typed
/// text as a `char`.
pub fn register_keyboard_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
	let keyboard_device_class_handle = input_manager.register_device_class("Keyboard");

	input_manager.register_trigger(&keyboard_device_class_handle, "W", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "S", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "A", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "D", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "Space", TriggerDescription::<bool>::default());

	input_manager.register_trigger(&keyboard_device_class_handle, "Up", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "Down", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "Left", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&keyboard_device_class_handle, "Right", TriggerDescription::<bool>::default());

	input_manager.register_trigger(&keyboard_device_class_handle, "Escape", TriggerDescription::<bool>::default());
	input_manager.register_trigger(
		&keyboard_device_class_handle,
		"Backspace",
		TriggerDescription::<bool>::default(),
	);

	input_manager.register_trigger(
		&keyboard_device_class_handle,
		"Character",
		TriggerDescription::<char>::default(),
	);

	keyboard_device_class_handle
}

/// Registers the standard Byte Engine gamepad device class.
///
/// # Triggers
///
/// - `LeftStick` and `RightStick`: 2D vectors from -1 to 1.
/// - `LeftTrigger` and `RightTrigger`: floats from 0 to 1.
/// - Face, bumper, stick, menu, and directional-pad buttons: Boolean values.
pub fn register_gamepad_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
	let gamepad_device_class_handle = input_manager.register_device_class("Gamepad");

	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"LeftStick",
		TriggerDescription::<Vector2>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"RightStick",
		TriggerDescription::<Vector2>::default(),
	);

	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"LeftTrigger",
		TriggerDescription::<f32>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"RightTrigger",
		TriggerDescription::<f32>::default(),
	);

	input_manager.register_trigger(&gamepad_device_class_handle, "A", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "B", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "X", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "Y", TriggerDescription::<bool>::default());

	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"LeftBumper",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"RightBumper",
		TriggerDescription::<bool>::default(),
	);

	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"LeftStickButton",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"RightStickButton",
		TriggerDescription::<bool>::default(),
	);

	input_manager.register_trigger(&gamepad_device_class_handle, "Select", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "Start", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "Guide", TriggerDescription::<bool>::default());

	input_manager.register_trigger(&gamepad_device_class_handle, "DPadUp", TriggerDescription::<bool>::default());
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"DPadDown",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"DPadLeft",
		TriggerDescription::<bool>::default(),
	);
	input_manager.register_trigger(
		&gamepad_device_class_handle,
		"DPadRight",
		TriggerDescription::<bool>::default(),
	);

	gamepad_device_class_handle
}

use math::Vector2;

use super::{device_class::DeviceClassHandle, input_trigger::TriggerDescription, InputManager};
