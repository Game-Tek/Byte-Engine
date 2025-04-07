use crate::Vector2;

use super::{device_class::DeviceClassHandle, input_trigger::TriggerDescription, InputManager};

/// Registers a mouse device class with the input manager.
/// This is the standard Byte-Engine mouse device definition.
/// 
/// # Triggers
/// - `Position`: The position of the mouse. This is a 2D vector. In the range of -1 to 1, relative to the window.
/// - `LeftButton`: The state of the left mouse button. This is a boolean.
/// - `RightButton`: The state of the right mouse button. This is a boolean.
/// - `Scroll`: The scroll wheel of the mouse. This is a float. The value is the amount of scroll in the Y direction. The range is -1 to 1.
pub fn register_mouse_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
	let mouse_device_class_handle = input_manager.register_device_class("Mouse");

	input_manager.register_trigger(&mouse_device_class_handle, "Position", TriggerDescription::<Vector2>::default());
	input_manager.register_trigger(&mouse_device_class_handle, "LeftButton", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&mouse_device_class_handle, "RightButton", TriggerDescription::<bool>::default());
	input_manager.register_trigger(&mouse_device_class_handle, "Scroll", TriggerDescription::new(0f32, 0f32, -1f32, 1f32));

	mouse_device_class_handle
}

/// Registers a keyboard device class with the input manager.
/// This is the standard Byte-Engine keyboard device definition.
/// 
/// # Triggers
/// - `W`: The state of the W key. This is a boolean.
/// - `S`: The state of the S key. This is a boolean.
/// - `A`: The state of the A key. This is a boolean.
/// - `D`: The state of the D key. This is a boolean.
/// - `Space`: The state of the Space key. This is a boolean.
/// - `Up`: The state of the Up key. This is a boolean.
/// - `Down`: The state of the Down key. This is a boolean.
/// - `Left`: The state of the Left key. This is a boolean.
/// - `Right`: The state of the Right key. This is a boolean.
/// - `Character`: The output of the keyboard as text input. This is a char.
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

	input_manager.register_trigger(&keyboard_device_class_handle, "Character", TriggerDescription::new('\0', '\0', '\0', 'Z'));

	keyboard_device_class_handle
}

/// Registers a gamepad device class with the input manager.
/// This is the standard Byte-Engine gamepad device definition.
/// 
/// # Triggers
/// - `LeftStick`: The position of the left stick. This is a 2D vector. In the range of -1 to 1.
/// - `RightStick`: The position of the right stick. This is a 2D vector. In the range of -1 to 1.
/// - `LeftTrigger`: The state of the left trigger. This is a float. The range is 0 to 1.
/// - `RightTrigger`: The state of the right trigger. This is a float. The range is 0 to 1.
pub fn register_gamepad_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
	let gamepad_device_class_handle = input_manager.register_device_class("Gamepad");

	input_manager.register_trigger(&gamepad_device_class_handle, "LeftStick", TriggerDescription::<Vector2>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "RightStick", TriggerDescription::<Vector2>::default());

	input_manager.register_trigger(&gamepad_device_class_handle, "LeftTrigger", TriggerDescription::<f32>::default());
	input_manager.register_trigger(&gamepad_device_class_handle, "RightTrigger", TriggerDescription::<f32>::default());

	gamepad_device_class_handle
}