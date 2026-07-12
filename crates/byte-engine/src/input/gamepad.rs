const STICK_EPSILON: f32 = 0.001;
const TRIGGER_EPSILON: f32 = 0.001;

const BUTTON_A: u32 = 1 << 0;
const BUTTON_B: u32 = 1 << 1;
const BUTTON_X: u32 = 1 << 2;
const BUTTON_Y: u32 = 1 << 3;
const BUTTON_LEFT_BUMPER: u32 = 1 << 4;
const BUTTON_RIGHT_BUMPER: u32 = 1 << 5;
const BUTTON_SELECT: u32 = 1 << 6;
const BUTTON_START: u32 = 1 << 7;
const BUTTON_LEFT_STICK: u32 = 1 << 8;
const BUTTON_RIGHT_STICK: u32 = 1 << 9;
const BUTTON_GUIDE: u32 = 1 << 10;
const BUTTON_DPAD_UP: u32 = 1 << 11;
const BUTTON_DPAD_DOWN: u32 = 1 << 12;
const BUTTON_DPAD_LEFT: u32 = 1 << 13;
const BUTTON_DPAD_RIGHT: u32 = 1 << 14;

const BUTTON_TRIGGERS: &[(u32, &str)] = &[
	(BUTTON_A, "Gamepad.A"),
	(BUTTON_B, "Gamepad.B"),
	(BUTTON_X, "Gamepad.X"),
	(BUTTON_Y, "Gamepad.Y"),
	(BUTTON_LEFT_BUMPER, "Gamepad.LeftBumper"),
	(BUTTON_RIGHT_BUMPER, "Gamepad.RightBumper"),
	(BUTTON_SELECT, "Gamepad.Select"),
	(BUTTON_START, "Gamepad.Start"),
	(BUTTON_LEFT_STICK, "Gamepad.LeftStickButton"),
	(BUTTON_RIGHT_STICK, "Gamepad.RightStickButton"),
	(BUTTON_GUIDE, "Gamepad.Guide"),
	(BUTTON_DPAD_UP, "Gamepad.DPadUp"),
	(BUTTON_DPAD_DOWN, "Gamepad.DPadDown"),
	(BUTTON_DPAD_LEFT, "Gamepad.DPadLeft"),
	(BUTTON_DPAD_RIGHT, "Gamepad.DPadRight"),
];

#[derive(Clone, Copy, Debug)]
struct GamepadState {
	left_stick: Vector2,
	right_stick: Vector2,
	left_trigger: f32,
	right_trigger: f32,
	buttons: u32,
}

impl Default for GamepadState {
	fn default() -> Self {
		Self {
			left_stick: Vector2::new(0.0, 0.0),
			right_stick: Vector2::new(0.0, 0.0),
			left_trigger: 0.0,
			right_trigger: 0.0,
			buttons: 0,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GamepadKind {
	DualShock4,
	DualSense,
	GenericJoystick,
	Xbox,
}

pub(crate) struct GamepadSystem {
	api: HidApi,
	devices: HashMap<String, GamepadDevice>,
	last_refresh: Instant,
}

impl GamepadSystem {
	pub(crate) fn new() -> Result<Self, String> {
		let api = HidApi::new().map_err(|e| {
			format!(
				"Failed to initialize HID API. The most likely cause is that the system HID backend is unavailable: {}",
				e
			)
		})?;

		Ok(Self {
			api,
			devices: HashMap::new(),
			last_refresh: Instant::now() - Duration::from_secs(2),
		})
	}

	pub(crate) fn poll(&mut self) -> (Vec<(String, GamepadKind, HidDevice)>, Vec<GamepadEvent>) {
		let new_devices = self.refresh_devices();
		let mut events = Vec::new();

		for device in self.devices.values_mut() {
			events.extend(device.poll());
		}

		(new_devices, events)
	}

	pub(crate) fn add_device(&mut self, path: String, kind: GamepadKind, device: HidDevice, device_handle: DeviceHandle) {
		self.devices.insert(path, GamepadDevice::new(kind, device, device_handle));
	}

	fn refresh_devices(&mut self) -> Vec<(String, GamepadKind, HidDevice)> {
		if self.last_refresh.elapsed() < Duration::from_secs(1) {
			return Vec::new();
		}

		self.last_refresh = Instant::now();

		if let Err(error) = self.api.refresh_devices() {
			warn!(
				"Failed to refresh HID devices. The most likely cause is that the HID backend could not enumerate devices: {}",
				error
			);
			return Vec::new();
		}

		let mut present_paths = HashSet::new();
		let mut new_devices = Vec::new();

		for device_info in self.api.device_list() {
			let kind = match classify_gamepad(
				device_info.vendor_id(),
				device_info.product_id(),
				device_info.product_string(),
				device_info.usage_page(),
				device_info.usage(),
			) {
				Some(kind) => kind,
				None => continue,
			};

			let path = device_info.path().to_string_lossy().to_string();
			present_paths.insert(path.clone());

			if self.devices.contains_key(&path) {
				continue;
			}

			let device = match self.api.open_path(device_info.path()) {
				Ok(device) => device,
				Err(error) => {
					warn!(
						"Failed to open HID device. The most likely cause is insufficient permissions or the device being in use: {}",
						error
					);
					continue;
				}
			};

			if let Err(error) = device.set_blocking_mode(false) {
				warn!(
					"Failed to set HID device to non-blocking mode. The most likely cause is a platform HID backend limitation: {}",
					error
				);
			}

			debug!(
				target: "byte_engine::input::events",
				"Detected HID gamepad: path={}, kind={:?}, vendor={:#06x}, product={:#06x}, name={}",
				path,
				kind,
				device_info.vendor_id(),
				device_info.product_id(),
				device_info.product_string().unwrap_or("<unknown>")
			);

			new_devices.push((path, kind, device));
		}

		self.devices.retain(|path, _| present_paths.contains(path));
		new_devices
	}
}

struct GamepadDevice {
	kind: GamepadKind,
	device: HidDevice,
	device_handle: DeviceHandle,
	state: GamepadState,
	initialized: bool,
}

impl GamepadDevice {
	fn new(kind: GamepadKind, device: HidDevice, device_handle: DeviceHandle) -> Self {
		Self {
			kind,
			device,
			device_handle,
			state: GamepadState::default(),
			initialized: false,
		}
	}

	fn poll(&mut self) -> Vec<GamepadEvent> {
		let mut buffer = [0u8; 128];
		let mut events = Vec::new();

		loop {
			let size = match self.device.read_timeout(&mut buffer, 0) {
				Ok(0) => break,
				Ok(size) => size,
				Err(error) => {
					warn!(
						"Failed to read HID input report. The most likely cause is that the device disconnected unexpectedly: {}",
						error
					);
					break;
				}
			};

			let report = &buffer[..size];
			let state = match self.kind {
				GamepadKind::DualShock4 => parse_dualshock4(report),
				GamepadKind::DualSense => parse_dualsense(report),
				GamepadKind::GenericJoystick => parse_generic_joystick(report),
				GamepadKind::Xbox => parse_xbox(report),
			};

			if let Some(state) = state {
				events.extend(self.emit_changes(state));
			}
		}

		events
	}

	fn emit_changes(&mut self, state: GamepadState) -> Vec<GamepadEvent> {
		transition_gamepad_state(self.device_handle, &mut self.state, &mut self.initialized, state)
	}
}

fn transition_gamepad_state(
	device_handle: DeviceHandle,
	previous: &mut GamepadState,
	initialized: &mut bool,
	state: GamepadState,
) -> Vec<GamepadEvent> {
	let mut events = Vec::new();

	if !*initialized {
		// The first HID report is the physical device's current state. Treat it as
		// baseline so neutral axes or held buttons do not replay as startup input.
		*previous = state;
		*initialized = true;
		return events;
	}

	if (previous.left_stick.x - state.left_stick.x).abs() > STICK_EPSILON
		|| (previous.left_stick.y - state.left_stick.y).abs() > STICK_EPSILON
	{
		events.push(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.LeftStick"),
			Value::Vector2(state.left_stick),
		));
	}

	if (previous.right_stick.x - state.right_stick.x).abs() > STICK_EPSILON
		|| (previous.right_stick.y - state.right_stick.y).abs() > STICK_EPSILON
	{
		events.push(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.RightStick"),
			Value::Vector2(state.right_stick),
		));
	}

	if (previous.left_trigger - state.left_trigger).abs() > TRIGGER_EPSILON {
		events.push(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.LeftTrigger"),
			Value::Float(state.left_trigger),
		));
	}

	if (previous.right_trigger - state.right_trigger).abs() > TRIGGER_EPSILON {
		events.push(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.RightTrigger"),
			Value::Float(state.right_trigger),
		));
	}

	for (mask, name) in BUTTON_TRIGGERS {
		let was_pressed = (previous.buttons & mask) != 0;
		let current = (state.buttons & mask) != 0;
		if was_pressed != current {
			events.push(GamepadEvent::new(
				device_handle,
				TriggerReference::Name(name),
				Value::Bool(current),
			));
		}
	}

	*previous = state;
	events
}

pub(crate) struct GamepadEvent {
	device_handle: DeviceHandle,
	trigger: TriggerReference,
	value: Value,
}

impl GamepadEvent {
	fn new(device_handle: DeviceHandle, trigger: TriggerReference, value: Value) -> Self {
		Self {
			device_handle,
			trigger,
			value,
		}
	}

	pub(crate) fn device_handle(&self) -> DeviceHandle {
		self.device_handle
	}

	pub(crate) fn trigger(&self) -> TriggerReference {
		self.trigger
	}

	pub(crate) fn value(&self) -> Value {
		self.value
	}
}

fn classify_gamepad(
	vendor_id: u16,
	product_id: u16,
	product_string: Option<&str>,
	usage_page: u16,
	usage: u16,
) -> Option<GamepadKind> {
	match vendor_id {
		0x054C => match product_id {
			0x05C4 | 0x09CC | 0x0BA0 | 0x0E5F => Some(GamepadKind::DualShock4),
			0x0CE6 | 0x0DF2 => Some(GamepadKind::DualSense),
			_ => None,
		},
		0x045E => Some(GamepadKind::Xbox),
		_ => {
			let product = product_string.unwrap_or_default();
			if contains_ascii_case_insensitive(product, "xbox") {
				Some(GamepadKind::Xbox)
			} else if contains_ascii_case_insensitive(product, "joystick") || (usage_page == 0x01 && usage == 0x04) {
				Some(GamepadKind::GenericJoystick)
			} else {
				None
			}
		}
	}
}

fn contains_ascii_case_insensitive(haystack: &str, needle: &str) -> bool {
	haystack
		.as_bytes()
		.windows(needle.len())
		.any(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn parse_dualshock4(report: &[u8]) -> Option<GamepadState> {
	let report = match report {
		[0x01, rest @ ..] => rest,
		[0x11, _marker, rest @ ..] => rest,
		_ => report,
	};

	if report.len() < 9 {
		return None;
	}

	let left_stick = Vector2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = Vector2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]));

	let buttons = report[4];
	let buttons2 = report[5];
	let buttons3 = report[6];

	let left_trigger = normalize_trigger_u8(report[7]);
	let right_trigger = normalize_trigger_u8(report[8]);

	let mut mask = 0u32;

	let dpad = buttons & 0x0F;
	if dpad == 0 || dpad == 1 || dpad == 7 {
		mask |= BUTTON_DPAD_UP;
	}
	if dpad == 2 || dpad == 1 || dpad == 3 {
		mask |= BUTTON_DPAD_RIGHT;
	}
	if dpad == 4 || dpad == 3 || dpad == 5 {
		mask |= BUTTON_DPAD_DOWN;
	}
	if dpad == 6 || dpad == 5 || dpad == 7 {
		mask |= BUTTON_DPAD_LEFT;
	}

	if buttons & 0x10 != 0 {
		mask |= BUTTON_X;
	}
	if buttons & 0x20 != 0 {
		mask |= BUTTON_A;
	}
	if buttons & 0x40 != 0 {
		mask |= BUTTON_B;
	}
	if buttons & 0x80 != 0 {
		mask |= BUTTON_Y;
	}

	if buttons2 & 0x01 != 0 {
		mask |= BUTTON_LEFT_BUMPER;
	}
	if buttons2 & 0x02 != 0 {
		mask |= BUTTON_RIGHT_BUMPER;
	}
	if buttons2 & 0x10 != 0 {
		mask |= BUTTON_SELECT;
	}
	if buttons2 & 0x20 != 0 {
		mask |= BUTTON_START;
	}
	if buttons2 & 0x40 != 0 {
		mask |= BUTTON_LEFT_STICK;
	}
	if buttons2 & 0x80 != 0 {
		mask |= BUTTON_RIGHT_STICK;
	}

	if buttons3 & 0x01 != 0 {
		mask |= BUTTON_GUIDE;
	}

	Some(GamepadState {
		left_stick,
		right_stick,
		left_trigger,
		right_trigger,
		buttons: mask,
	})
}

fn parse_generic_joystick(report: &[u8]) -> Option<GamepadState> {
	let report = match report {
		[report_id @ 1..=15, rest @ ..] if rest.len() >= 6 => {
			debug!(target: "byte_engine::input::events", "Parsing generic joystick report id: {}", report_id);
			rest
		}
		_ => report,
	};

	if report.len() < 5 {
		debug!(
			target: "byte_engine::input::events",
			"Ignoring generic joystick report with unsupported size: {}",
			report.len()
		);
		return None;
	}

	let left_stick = Vector2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = if report.len() >= 7 {
		Vector2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]))
	} else {
		Vector2::new(0.0, 0.0)
	};

	let (hat, raw_buttons) = if report.len() >= 7 {
		let packed_hat_buttons = report[4];
		let buttons = u16::from_le_bytes([packed_hat_buttons, report[5]]);
		(Some(packed_hat_buttons & 0x0F), buttons)
	} else {
		let packed_hat_buttons = report[2];
		let buttons = u16::from_le_bytes([packed_hat_buttons, report.get(3).copied().unwrap_or_default()]);
		(Some(packed_hat_buttons & 0x0F), buttons)
	};
	let mut mask = 0u32;
	debug!(
		target: "byte_engine::input::events",
		"Generic joystick raw buttons={:#06x}, hat={:?}, report_size={}",
		raw_buttons,
		hat,
		report.len()
	);

	// Generic USB joysticks commonly keep non-button metadata in the low nibble.
	// Start mapping at bit 4 so neutral metadata does not look like held buttons.
	for (index, engine_mask) in [
		BUTTON_A,
		BUTTON_B,
		BUTTON_Y,
		BUTTON_LEFT_BUMPER,
		BUTTON_RIGHT_BUMPER,
		BUTTON_SELECT,
		BUTTON_START,
		BUTTON_LEFT_STICK,
		BUTTON_RIGHT_STICK,
		BUTTON_GUIDE,
	]
	.iter()
	.enumerate()
	{
		if raw_buttons & (1 << (index + 4)) != 0 {
			mask |= *engine_mask;
		}
	}

	// This AppleUserHIDDevice generic joystick reports X as an active-low bit.
	if raw_buttons & 0x4000 == 0 {
		mask |= BUTTON_X;
	}

	if let Some(hat) = hat {
		if hat == 0 || hat == 1 || hat == 7 {
			mask |= BUTTON_DPAD_UP;
		}
		if hat == 2 || hat == 1 || hat == 3 {
			mask |= BUTTON_DPAD_RIGHT;
		}
		if hat == 4 || hat == 3 || hat == 5 {
			mask |= BUTTON_DPAD_DOWN;
		}
		if hat == 6 || hat == 5 || hat == 7 {
			mask |= BUTTON_DPAD_LEFT;
		}
	}

	Some(GamepadState {
		left_stick,
		right_stick,
		left_trigger: 0.0,
		right_trigger: 0.0,
		buttons: mask,
	})
}

fn parse_dualsense(report: &[u8]) -> Option<GamepadState> {
	let report = match report {
		[0x01, rest @ ..] => rest,
		[0x31, _marker, rest @ ..] => rest,
		_ => report,
	};

	if report.len() < 9 {
		return None;
	}

	let left_stick = Vector2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = Vector2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]));

	let buttons = report[4];
	let buttons2 = report[5];
	let buttons3 = report[6];

	let left_trigger = normalize_trigger_u8(report[7]);
	let right_trigger = normalize_trigger_u8(report[8]);

	let mut mask = 0u32;

	let dpad = buttons & 0x0F;
	if dpad == 0 || dpad == 1 || dpad == 7 {
		mask |= BUTTON_DPAD_UP;
	}
	if dpad == 2 || dpad == 1 || dpad == 3 {
		mask |= BUTTON_DPAD_RIGHT;
	}
	if dpad == 4 || dpad == 3 || dpad == 5 {
		mask |= BUTTON_DPAD_DOWN;
	}
	if dpad == 6 || dpad == 5 || dpad == 7 {
		mask |= BUTTON_DPAD_LEFT;
	}

	if buttons & 0x10 != 0 {
		mask |= BUTTON_X;
	}
	if buttons & 0x20 != 0 {
		mask |= BUTTON_A;
	}
	if buttons & 0x40 != 0 {
		mask |= BUTTON_B;
	}
	if buttons & 0x80 != 0 {
		mask |= BUTTON_Y;
	}

	if buttons2 & 0x01 != 0 {
		mask |= BUTTON_LEFT_BUMPER;
	}
	if buttons2 & 0x02 != 0 {
		mask |= BUTTON_RIGHT_BUMPER;
	}
	if buttons2 & 0x10 != 0 {
		mask |= BUTTON_SELECT;
	}
	if buttons2 & 0x20 != 0 {
		mask |= BUTTON_START;
	}
	if buttons2 & 0x40 != 0 {
		mask |= BUTTON_LEFT_STICK;
	}
	if buttons2 & 0x80 != 0 {
		mask |= BUTTON_RIGHT_STICK;
	}

	if buttons3 & 0x01 != 0 {
		mask |= BUTTON_GUIDE;
	}

	Some(GamepadState {
		left_stick,
		right_stick,
		left_trigger,
		right_trigger,
		buttons: mask,
	})
}

fn parse_xbox(report: &[u8]) -> Option<GamepadState> {
	let report = if report.first().copied() == Some(0x01) {
		&report[1..]
	} else {
		report
	};

	if report.len() < 14 {
		return None;
	}

	let buttons = u16::from_le_bytes([report[2], report[3]]);

	let left_trigger = normalize_trigger_u8(report[4]);
	let right_trigger = normalize_trigger_u8(report[5]);

	let left_stick = Vector2::new(
		normalize_axis_i16(i16::from_le_bytes([report[6], report[7]])),
		-normalize_axis_i16(i16::from_le_bytes([report[8], report[9]])),
	);

	let right_stick = Vector2::new(
		normalize_axis_i16(i16::from_le_bytes([report[10], report[11]])),
		-normalize_axis_i16(i16::from_le_bytes([report[12], report[13]])),
	);

	let mut mask = 0u32;

	if buttons & 0x0001 != 0 {
		mask |= BUTTON_DPAD_UP;
	}
	if buttons & 0x0002 != 0 {
		mask |= BUTTON_DPAD_DOWN;
	}
	if buttons & 0x0004 != 0 {
		mask |= BUTTON_DPAD_LEFT;
	}
	if buttons & 0x0008 != 0 {
		mask |= BUTTON_DPAD_RIGHT;
	}

	if buttons & 0x0010 != 0 {
		mask |= BUTTON_START;
	}
	if buttons & 0x0020 != 0 {
		mask |= BUTTON_SELECT;
	}
	if buttons & 0x0040 != 0 {
		mask |= BUTTON_LEFT_STICK;
	}
	if buttons & 0x0080 != 0 {
		mask |= BUTTON_RIGHT_STICK;
	}

	if buttons & 0x0100 != 0 {
		mask |= BUTTON_LEFT_BUMPER;
	}
	if buttons & 0x0200 != 0 {
		mask |= BUTTON_RIGHT_BUMPER;
	}
	if buttons & 0x0400 != 0 {
		mask |= BUTTON_GUIDE;
	}

	if buttons & 0x1000 != 0 {
		mask |= BUTTON_A;
	}
	if buttons & 0x2000 != 0 {
		mask |= BUTTON_B;
	}
	if buttons & 0x4000 != 0 {
		mask |= BUTTON_X;
	}
	if buttons & 0x8000 != 0 {
		mask |= BUTTON_Y;
	}

	Some(GamepadState {
		left_stick,
		right_stick,
		left_trigger,
		right_trigger,
		buttons: mask,
	})
}

fn normalize_axis_u8(value: u8) -> f32 {
	let scaled = (value as f32 - 128.0) / 127.0;
	scaled.clamp(-1.0, 1.0)
}

fn normalize_axis_i16(value: i16) -> f32 {
	if value < 0 {
		(value as f32) / 32768.0
	} else {
		(value as f32) / 32767.0
	}
}

fn normalize_trigger_u8(value: u8) -> f32 {
	(value as f32) / 255.0
}

use std::{
	collections::{HashMap, HashSet},
	time::{Duration, Instant},
};

use hidapi::{HidApi, HidDevice};
use log::{debug, warn};
use math::Vector2;

use super::{input_manager::TriggerReference, DeviceHandle, Value};

#[cfg(test)]
mod tests {
	use super::*;

	fn assert_float_near(actual: f32, expected: f32) {
		assert!((actual - expected).abs() < 0.000_01, "expected {expected}, got {actual}");
	}

	fn assert_states_equal(actual: GamepadState, expected: GamepadState) {
		assert_float_near(actual.left_stick.x, expected.left_stick.x);
		assert_float_near(actual.left_stick.y, expected.left_stick.y);
		assert_float_near(actual.right_stick.x, expected.right_stick.x);
		assert_float_near(actual.right_stick.y, expected.right_stick.y);
		assert_float_near(actual.left_trigger, expected.left_trigger);
		assert_float_near(actual.right_trigger, expected.right_trigger);
		assert_eq!(actual.buttons, expected.buttons);
	}

	#[test]
	fn classifies_known_controllers_without_case_sensitive_product_names() {
		assert_eq!(classify_gamepad(0x054C, 0x05C4, None, 0, 0), Some(GamepadKind::DualShock4));
		assert_eq!(classify_gamepad(0x054C, 0x0CE6, None, 0, 0), Some(GamepadKind::DualSense));
		assert_eq!(classify_gamepad(0x045E, 0, None, 0, 0), Some(GamepadKind::Xbox));
		assert_eq!(
			classify_gamepad(0, 0, Some("Wireless XBOX Controller"), 0, 0),
			Some(GamepadKind::Xbox)
		);
		assert_eq!(
			classify_gamepad(0, 0, Some("Arcade JoYsTiCk"), 0, 0),
			Some(GamepadKind::GenericJoystick)
		);
		assert_eq!(classify_gamepad(0, 0, None, 0x01, 0x04), Some(GamepadKind::GenericJoystick));
		assert_eq!(classify_gamepad(0x054C, 0xFFFF, Some("joystick"), 0x01, 0x04), None);
		assert_eq!(classify_gamepad(0, 0, Some("Keyboard"), 0x01, 0x06), None);
	}

	#[test]
	fn axis_and_trigger_normalization_preserves_endpoints_and_order() {
		assert_eq!(normalize_axis_u8(0), -1.0);
		assert_eq!(normalize_axis_u8(128), 0.0);
		assert_eq!(normalize_axis_u8(u8::MAX), 1.0);

		let mut previous = -1.0;
		for value in u8::MIN..=u8::MAX {
			let normalized = normalize_axis_u8(value);
			assert!((-1.0..=1.0).contains(&normalized));
			assert!(normalized >= previous);
			previous = normalized;
		}

		assert_eq!(normalize_axis_i16(i16::MIN), -1.0);
		assert_eq!(normalize_axis_i16(0), 0.0);
		assert_eq!(normalize_axis_i16(i16::MAX), 1.0);
		assert_eq!(normalize_trigger_u8(0), 0.0);
		assert_eq!(normalize_trigger_u8(u8::MAX), 1.0);
	}

	#[test]
	fn sony_reports_decode_axes_triggers_buttons_and_transport_prefixes() {
		let payload = [0, 255, 255, 0, 0xF1, 0xF3, 0x01, 0, 255];
		let expected_buttons =
			BUTTON_A
				| BUTTON_B | BUTTON_X
				| BUTTON_Y | BUTTON_LEFT_BUMPER
				| BUTTON_RIGHT_BUMPER
				| BUTTON_SELECT
				| BUTTON_START
				| BUTTON_LEFT_STICK
				| BUTTON_RIGHT_STICK
				| BUTTON_GUIDE
				| BUTTON_DPAD_UP
				| BUTTON_DPAD_RIGHT;

		let raw = parse_dualshock4(&payload).expect("valid raw DualShock report");
		assert_eq!(raw.left_stick, Vector2::new(-1.0, -1.0));
		assert_eq!(raw.right_stick, Vector2::new(1.0, 1.0));
		assert_eq!(raw.left_trigger, 0.0);
		assert_eq!(raw.right_trigger, 1.0);
		assert_eq!(raw.buttons, expected_buttons);

		let usb = [0x01].into_iter().chain(payload).collect::<Vec<_>>();
		let bluetooth = [0x11, 0x80].into_iter().chain(payload).collect::<Vec<_>>();
		assert_states_equal(parse_dualshock4(&usb).expect("valid USB report"), raw);
		assert_states_equal(parse_dualshock4(&bluetooth).expect("valid Bluetooth report"), raw);

		let dualsense_usb = [0x01].into_iter().chain(payload).collect::<Vec<_>>();
		let dualsense_bluetooth = [0x31, 0x02].into_iter().chain(payload).collect::<Vec<_>>();
		assert_states_equal(parse_dualsense(&dualsense_usb).expect("valid USB report"), raw);
		assert_states_equal(parse_dualsense(&dualsense_bluetooth).expect("valid Bluetooth report"), raw);
	}

	#[test]
	fn xbox_reports_decode_little_endian_axes_and_button_mask() {
		let mut payload = [0u8; 14];
		payload[2..4].copy_from_slice(&u16::MAX.to_le_bytes());
		payload[4] = 0;
		payload[5] = u8::MAX;
		payload[6..8].copy_from_slice(&i16::MIN.to_le_bytes());
		payload[8..10].copy_from_slice(&i16::MAX.to_le_bytes());
		payload[10..12].copy_from_slice(&i16::MAX.to_le_bytes());
		payload[12..14].copy_from_slice(&i16::MIN.to_le_bytes());

		let raw = parse_xbox(&payload).expect("valid Xbox report");
		assert_eq!(raw.left_stick, Vector2::new(-1.0, -1.0));
		assert_eq!(raw.right_stick, Vector2::new(1.0, 1.0));
		assert_eq!(raw.left_trigger, 0.0);
		assert_eq!(raw.right_trigger, 1.0);
		assert_eq!(
			raw.buttons,
			BUTTON_TRIGGERS.iter().fold(0, |buttons, (mask, _)| buttons | mask)
		);

		let prefixed = [0x01].into_iter().chain(payload).collect::<Vec<_>>();
		assert_states_equal(parse_xbox(&prefixed).expect("valid prefixed Xbox report"), raw);
	}

	#[test]
	fn generic_reports_keep_packed_buttons_aligned_and_decode_active_low_x() {
		// The low nibble is the hat, the high nibble starts the contiguous button mask,
		// and bit 14 is the active-low X input used by AppleUserHIDDevice.
		let released_x = [0, 255, 128, 128, 0x11, 0x40, 0];
		let state = parse_generic_joystick(&released_x).expect("valid generic report");
		assert_eq!(state.left_stick, Vector2::new(-1.0, -1.0));
		assert_eq!(state.right_stick, Vector2::new(0.0, 0.0));
		assert_eq!(state.buttons, BUTTON_A | BUTTON_DPAD_UP | BUTTON_DPAD_RIGHT);

		let mut pressed_x = released_x;
		pressed_x[5] &= !0x40;
		let state = parse_generic_joystick(&pressed_x).expect("valid generic report");
		assert_eq!(state.buttons, BUTTON_A | BUTTON_X | BUTTON_DPAD_UP | BUTTON_DPAD_RIGHT);

		let prefixed = [0x07].into_iter().chain(released_x).collect::<Vec<_>>();
		assert_states_equal(
			parse_generic_joystick(&prefixed).expect("valid report-id report"),
			parse_generic_joystick(&released_x).unwrap(),
		);
	}

	#[test]
	fn parsers_reject_reports_without_their_required_payload() {
		assert!(parse_dualshock4(&[0; 8]).is_none());
		assert!(parse_dualsense(&[0; 8]).is_none());
		assert!(parse_generic_joystick(&[0; 4]).is_none());
		assert!(parse_xbox(&[0; 13]).is_none());
	}

	#[test]
	fn state_transitions_suppress_baselines_and_noise_but_emit_each_meaningful_delta() {
		let device = DeviceHandle(7);
		let mut previous = GamepadState::default();
		let mut initialized = false;
		let baseline = GamepadState {
			buttons: BUTTON_A,
			..GamepadState::default()
		};

		assert!(transition_gamepad_state(device, &mut previous, &mut initialized, baseline).is_empty());
		assert!(initialized);
		assert_eq!(previous.buttons, BUTTON_A);

		let noise = GamepadState {
			left_stick: Vector2::new(STICK_EPSILON, 0.0),
			left_trigger: TRIGGER_EPSILON,
			buttons: BUTTON_A,
			..GamepadState::default()
		};
		assert!(transition_gamepad_state(device, &mut previous, &mut initialized, noise).is_empty());

		let changed = GamepadState {
			left_stick: Vector2::new(0.5, -0.25),
			right_stick: Vector2::new(-0.75, 1.0),
			left_trigger: 0.25,
			right_trigger: 1.0,
			buttons: BUTTON_B,
		};
		let events = transition_gamepad_state(device, &mut previous, &mut initialized, changed);

		assert_eq!(events.len(), 6);
		assert!(events.iter().all(|event| event.device_handle() == device));
		let observed = events
			.iter()
			.map(|event| match event.trigger() {
				TriggerReference::Name(name) => (name, event.value()),
				TriggerReference::Handle(_) => panic!("gamepad transitions use named triggers"),
			})
			.collect::<Vec<_>>();
		assert_eq!(
			observed,
			[
				("Gamepad.LeftStick", Value::Vector2(changed.left_stick)),
				("Gamepad.RightStick", Value::Vector2(changed.right_stick)),
				("Gamepad.LeftTrigger", Value::Float(changed.left_trigger)),
				("Gamepad.RightTrigger", Value::Float(changed.right_trigger)),
				("Gamepad.A", Value::Bool(false)),
				("Gamepad.B", Value::Bool(true)),
			]
		);
		assert_states_equal(previous, changed);
	}
}
