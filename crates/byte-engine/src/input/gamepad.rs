use std::{
	collections::{HashMap, HashSet},
	time::{Duration, Instant},
};

use hidapi::{HidApi, HidDevice};
use log::warn;
use math::Vector2;

use super::{input_manager::TriggerReference, DeviceHandle, Value};

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

pub(super) enum GamepadKind {
	DualShock4,
	DualSense,
	Xbox,
}

pub(super) struct GamepadSystem {
	api: HidApi,
	devices: HashMap<String, GamepadDevice>,
	last_refresh: Instant,
}

impl GamepadSystem {
	pub(super) fn new() -> Result<Self, String> {
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

	pub(super) fn poll(&mut self) -> (Vec<(String, GamepadKind, HidDevice)>, Vec<GamepadEvent>) {
		let new_devices = self.refresh_devices();
		let mut events = Vec::new();

		for device in self.devices.values_mut() {
			events.extend(device.poll());
		}

		(new_devices, events)
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
}

impl GamepadDevice {
	fn new(kind: GamepadKind, device: HidDevice, device_handle: DeviceHandle) -> Self {
		Self {
			kind,
			device,
			device_handle,
			state: GamepadState::default(),
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
					warn!("Failed to read HID input report. The most likely cause is that the device disconnected unexpectedly: {}", error);
					break;
				}
			};

			let report = &buffer[..size];
			let state = match self.kind {
				GamepadKind::DualShock4 => parse_dualshock4(report),
				GamepadKind::DualSense => parse_dualsense(report),
				GamepadKind::Xbox => parse_xbox(report),
			};

			if let Some(state) = state {
				events.extend(self.emit_changes(state));
			}
		}

		events
	}

	fn emit_changes(&mut self, state: GamepadState) -> Vec<GamepadEvent> {
		let mut events = Vec::new();

		if (self.state.left_stick.x - state.left_stick.x).abs() > STICK_EPSILON
			|| (self.state.left_stick.y - state.left_stick.y).abs() > STICK_EPSILON
		{
			events.push(GamepadEvent::new(
				self.device_handle,
				TriggerReference::Name("Gamepad.LeftStick"),
				Value::Vector2(state.left_stick),
			));
		}

		if (self.state.right_stick.x - state.right_stick.x).abs() > STICK_EPSILON
			|| (self.state.right_stick.y - state.right_stick.y).abs() > STICK_EPSILON
		{
			events.push(GamepadEvent::new(
				self.device_handle,
				TriggerReference::Name("Gamepad.RightStick"),
				Value::Vector2(state.right_stick),
			));
		}

		if (self.state.left_trigger - state.left_trigger).abs() > TRIGGER_EPSILON {
			events.push(GamepadEvent::new(
				self.device_handle,
				TriggerReference::Name("Gamepad.LeftTrigger"),
				Value::Float(state.left_trigger),
			));
		}

		if (self.state.right_trigger - state.right_trigger).abs() > TRIGGER_EPSILON {
			events.push(GamepadEvent::new(
				self.device_handle,
				TriggerReference::Name("Gamepad.RightTrigger"),
				Value::Float(state.right_trigger),
			));
		}

		for (mask, name) in BUTTON_TRIGGERS {
			let previous = (self.state.buttons & mask) != 0;
			let current = (state.buttons & mask) != 0;
			if previous != current {
				events.push(GamepadEvent::new(
					self.device_handle,
					TriggerReference::Name(name),
					Value::Bool(current),
				));
			}
		}

		self.state = state;
		events
	}
}

pub(super) struct GamepadEvent {
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
}

fn classify_gamepad(vendor_id: u16, product_id: u16, product_string: Option<&str>) -> Option<GamepadKind> {
	match vendor_id {
		0x054C => match product_id {
			0x05C4 | 0x09CC | 0x0BA0 | 0x0E5F => Some(GamepadKind::DualShock4),
			0x0CE6 | 0x0DF2 => Some(GamepadKind::DualSense),
			_ => None,
		},
		0x045E => Some(GamepadKind::Xbox),
		_ => {
			let product = product_string.unwrap_or_default().to_lowercase();
			if product.contains("xbox") {
				Some(GamepadKind::Xbox)
			} else {
				None
			}
		}
	}
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
