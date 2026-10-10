//! Reads gamepads and joysticks over HID and records their sticks, triggers, and buttons as input.
//!
//! [`GamepadSystem`] finds the connected controllers when the application starts, scans again when the operating
//! system reports that a device connected or disconnected, and decodes each controller's reports into `Gamepad.*`
//! triggers.

use std::time::{Duration, Instant};

use ghi::hid::{Device, DeviceInfo, DevicePath, Monitor, Scanner, Usage};
use log::{debug, warn};

use super::Axis2;
use super::{DeviceHandle, InputCollector, SeatHandle, TriggerReference, Value, device::DeviceClassHandle};

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
	left_stick: Axis2,
	right_stick: Axis2,
	left_trigger: f32,
	right_trigger: f32,
	buttons: u32,
}

impl Default for GamepadState {
	fn default() -> Self {
		Self {
			left_stick: Axis2::new(0.0, 0.0),
			right_stick: Axis2::new(0.0, 0.0),
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

/// Joysticks and gamepads, the usages every supported controller declares.
const GAMEPAD_USAGES: &[Usage] = &[Usage { page: 0x01, usage: 0x04 }, Usage { page: 0x01, usage: 0x05 }];

/// How often to scan for controllers when the operating system refused device notifications.
const FALLBACK_RESCAN_INTERVAL: Duration = Duration::from_secs(1);

/// The largest input report any supported controller sends; DualShock 4 Bluetooth reports are 78 bytes.
const MAX_REPORT_SIZE: usize = 128;

/// The `GamepadSystem` struct owns every connected controller so the application can turn their reports into input.
///
/// Create it when the application starts, then call [`GamepadSystem::poll`] once per frame. Pass
/// [`GamepadSystem::monitor`] to `ghi::window::App::wake_on_hid_changes` so a connecting controller wakes an idle
/// application.
pub(crate) struct GamepadSystem {
	scanner: Scanner,
	/// Reports device changes; `None` when the operating system refused the subscription.
	monitor: Option<Monitor>,
	/// When the last scan ran, which paces the timed scans that stand in for a missing monitor.
	last_scan: Instant,
	devices: Vec<GamepadDevice>,
}

impl GamepadSystem {
	/// Subscribes to device changes, then opens the controllers connected now.
	///
	/// Next, call [`GamepadSystem::poll`] once per frame.
	pub(crate) fn new() -> Self {
		// Subscribing first means a controller that connects during the scan is reported as a change.
		let monitor = Monitor::new().map_err(|error| warn!("{error}")).ok();
		let mut system = Self {
			scanner: Scanner::new(GAMEPAD_USAGES),
			monitor,
			last_scan: Instant::now(),
			devices: Vec::new(),
		};
		system.rescan();
		system
	}

	/// Returns the device change monitor, for the window event loop to wake on.
	pub(crate) fn monitor(&self) -> Option<&Monitor> {
		self.monitor.as_ref()
	}

	/// Reports whether the application must poll on a timer: controllers report input only when read, and without
	/// a monitor new controllers only appear through timed scans.
	pub(crate) fn needs_polling(&self) -> bool {
		!self.devices.is_empty() || self.monitor.is_none()
	}

	/// Applies device changes, then records every controller's state changes into `input`.
	///
	/// New controllers become devices of `device_class`. Without a device class, they are dropped with a warning,
	/// because the application never called `setup_default_input`.
	pub(crate) fn poll(&mut self, input: &mut InputCollector, device_class: Option<DeviceClassHandle>) {
		let changed = match &mut self.monitor {
			Some(monitor) => monitor.take_changed(),
			// Without notifications, scan on a timer so controllers connected later still appear.
			None => self.last_scan.elapsed() >= FALLBACK_RESCAN_INTERVAL,
		};
		if changed {
			self.rescan();
		}

		// The device class is registered after startup, so controllers get their input device on their first poll.
		if self.devices.iter().any(|device| device.device_handle.is_none()) {
			match device_class {
				Some(device_class) => {
					for device in self.devices.iter_mut().filter(|device| device.device_handle.is_none()) {
						// Keep physical HID identity distinct so player and device routing is preserved.
						device.device_handle = Some(input.create_device(&device_class));
					}
				}
				None => {
					self.devices.retain(|device| device.device_handle.is_some());
					warn!(
						"Detected HID gamepad before the Gamepad device class was registered. The most likely cause is that setup_default_input was not called. See {}.",
						crate::online_docs_url("reference/input")
					);
				}
			}
		}

		for device in &mut self.devices {
			device.poll(|event| {
				debug!(
					target: "byte_engine::input::events",
					"Forwarding HID gamepad event: device={:?}, trigger={:?}, value={:?}",
					event.device_handle,
					event.trigger,
					event.value
				);
				input.record(SeatHandle::stub(), event.device_handle, event.trigger, event.value);
			});
		}
	}

	/// Scans the connected controllers, opens new ones, and drops the ones that disconnected.
	///
	/// Known controllers are matched by borrowed path, so a scan allocates only for new controllers. A controller
	/// that fails to open is skipped and retried on the next change. On Linux that change comes when udev grants
	/// access to a node it reported too early.
	fn rescan(&mut self) {
		self.last_scan = Instant::now();
		self.devices.iter_mut().for_each(|device| device.seen = false);

		let devices = &mut self.devices;
		let scanned = self.scanner.scan(|info| {
			if let Some(known) = devices.iter_mut().find(|known| info.path == known.path) {
				known.seen = true;
			} else if let Some(device) = GamepadDevice::open(info) {
				devices.push(device);
			}
		});
		if let Err(error) = scanned {
			// Keep the controllers as they are; the next change scans again.
			warn!("{error}");
			return;
		}

		self.devices.retain(|device| device.seen);
	}
}

/// The `GamepadDevice` struct keeps an open controller with the state its last report decoded to.
struct GamepadDevice {
	path: DevicePath,
	kind: GamepadKind,
	/// Steam's virtual pad already reports stick-up as negative. Hardware reports do not.
	negate_stick_y: bool,
	/// Whether the latest scan found the controller.
	seen: bool,
	device: Device,
	/// The input device its events belong to, created on the first poll after it connected.
	device_handle: Option<DeviceHandle>,
	state: GamepadState,
	initialized: bool,
}

impl GamepadDevice {
	/// Opens a scanned device when it is a supported controller, or returns `None` for other devices and failures.
	fn open(info: DeviceInfo<'_>) -> Option<Self> {
		let kind = classify_gamepad(
			info.vendor_id,
			info.product_id,
			info.product_name,
			info.usage.page,
			info.usage.usage,
		)?;
		debug!(
			target: "byte_engine::input::events",
			"Detected HID gamepad: path={}, kind={:?}, vendor={:#06x}, product={:#06x}, name={}",
			info.path,
			kind,
			info.vendor_id,
			info.product_id,
			info.product_name.unwrap_or("<unknown>")
		);
		let path = info.path.to_owned();
		let device = Device::open(&path).map_err(|error| warn!("{error}")).ok()?;
		Some(Self {
			path,
			kind,
			negate_stick_y: stick_up_is_negative(kind, info.product_name),
			seen: true,
			device,
			device_handle: None,
			state: GamepadState::default(),
			initialized: false,
		})
	}

	/// Decodes every waiting report and passes each state change to `emit`.
	fn poll(&mut self, mut emit: impl FnMut(GamepadEvent)) {
		let Some(device_handle) = self.device_handle else {
			return;
		};
		let mut buffer = [0u8; MAX_REPORT_SIZE];

		loop {
			let size = match self.device.read(&mut buffer) {
				Ok(Some(size)) => size,
				Ok(None) => break,
				Err(error) => {
					warn!("{error}");
					break;
				}
			};

			let report = &buffer[..size];
			let state = match self.kind {
				GamepadKind::DualShock4 => parse_dualshock4(report),
				GamepadKind::DualSense => parse_dualsense(report),
				GamepadKind::GenericJoystick => parse_generic_joystick(report),
				GamepadKind::Xbox => parse_xbox(report, self.negate_stick_y),
			};

			if let Some(state) = state {
				transition_gamepad_state(device_handle, &mut self.state, &mut self.initialized, state, &mut emit);
			}
		}
	}
}

/// Passes each meaningful difference between `previous` and `state` to `emit`, then stores `state`.
fn transition_gamepad_state(
	device_handle: DeviceHandle,
	previous: &mut GamepadState,
	initialized: &mut bool,
	state: GamepadState,
	emit: &mut impl FnMut(GamepadEvent),
) {
	if !*initialized {
		// The first HID report is the physical device's current state. Treat it as
		// baseline so neutral axes or held buttons do not replay as startup input.
		*previous = state;
		*initialized = true;
		return;
	}

	if (previous.left_stick.x - state.left_stick.x).abs() > STICK_EPSILON
		|| (previous.left_stick.y - state.left_stick.y).abs() > STICK_EPSILON
	{
		emit(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.LeftStick"),
			Value::Vector2(state.left_stick),
		));
	}

	if (previous.right_stick.x - state.right_stick.x).abs() > STICK_EPSILON
		|| (previous.right_stick.y - state.right_stick.y).abs() > STICK_EPSILON
	{
		emit(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.RightStick"),
			Value::Vector2(state.right_stick),
		));
	}

	if (previous.left_trigger - state.left_trigger).abs() > TRIGGER_EPSILON {
		emit(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.LeftTrigger"),
			Value::Float(state.left_trigger),
		));
	}

	if (previous.right_trigger - state.right_trigger).abs() > TRIGGER_EPSILON {
		emit(GamepadEvent::new(
			device_handle,
			TriggerReference::Name("Gamepad.RightTrigger"),
			Value::Float(state.right_trigger),
		));
	}

	for (mask, name) in BUTTON_TRIGGERS {
		let was_pressed = (previous.buttons & mask) != 0;
		let current = (state.buttons & mask) != 0;
		if was_pressed != current {
			emit(GamepadEvent::new(
				device_handle,
				TriggerReference::Name(name),
				Value::Bool(current),
			));
		}
	}

	*previous = state;
}

/// The `GamepadEvent` struct carries one decoded state change to the input collector.
struct GamepadEvent {
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
		// The vendor interface on the same controller is not a stick report.
		0x045E if usage_page == 0x01 && (usage == 0x04 || usage == 0x05) => Some(GamepadKind::Xbox),
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

	let left_stick = Axis2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = Axis2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]));

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

	let left_stick = Axis2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = if report.len() >= 7 {
		Axis2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]))
	} else {
		Axis2::new(0.0, 0.0)
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

	let left_stick = Axis2::new(normalize_axis_u8(report[0]), -normalize_axis_u8(report[1]));
	let right_stick = Axis2::new(normalize_axis_u8(report[2]), -normalize_axis_u8(report[3]));

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

/// Hardware Xbox sticks report up as a positive Y. Steam's virtual `GamePad-*` pad
/// already stores up as negative, so only that source needs another flip.
fn stick_up_is_negative(kind: GamepadKind, product_name: Option<&str>) -> bool {
	matches!(kind, GamepadKind::Xbox) && product_name.is_some_and(|name| name.starts_with("GamePad-"))
}

fn parse_xbox(report: &[u8], negate_y: bool) -> Option<GamepadState> {
	// Xbox One and Series pads send a GIP input command. Reading that packet with the
	// Xbox 360 offsets places the physical left stick in the right-stick fields.
	if report.first().copied() == Some(0x20) {
		return parse_xbox_one(report, negate_y);
	}

	parse_xbox_360(report, negate_y)
}

/// Decodes an Xbox One GIP input packet. The state begins after the variable-length header.
fn parse_xbox_one(report: &[u8], negate_y: bool) -> Option<GamepadState> {
	let report = if report.len() >= 2 && report[1] == 0x20 {
		&report[1..]
	} else {
		report
	};
	if report.first().copied() != Some(0x20) {
		return None;
	}

	let options = *report.get(1)?;
	// Expansion and internal packets are not the thumbstick state.
	if options & 0x0F != 0 || options & 0x20 != 0 {
		return None;
	}

	let header_len = gip_header_length(report)?;
	let data = report.get(header_len..)?;
	if data.len() < 14 {
		return None;
	}

	let buttons = data[0];
	let buttons2 = data[1];
	let mut mask = 0u32;
	if buttons & 0x04 != 0 {
		mask |= BUTTON_START;
	}
	if buttons & 0x08 != 0 {
		mask |= BUTTON_SELECT;
	}
	if buttons & 0x10 != 0 {
		mask |= BUTTON_A;
	}
	if buttons & 0x20 != 0 {
		mask |= BUTTON_B;
	}
	if buttons & 0x40 != 0 {
		mask |= BUTTON_X;
	}
	if buttons & 0x80 != 0 {
		mask |= BUTTON_Y;
	}
	if buttons2 & 0x01 != 0 {
		mask |= BUTTON_DPAD_UP;
	}
	if buttons2 & 0x02 != 0 {
		mask |= BUTTON_DPAD_DOWN;
	}
	if buttons2 & 0x04 != 0 {
		mask |= BUTTON_DPAD_LEFT;
	}
	if buttons2 & 0x08 != 0 {
		mask |= BUTTON_DPAD_RIGHT;
	}
	if buttons2 & 0x10 != 0 {
		mask |= BUTTON_LEFT_BUMPER;
	}
	if buttons2 & 0x20 != 0 {
		mask |= BUTTON_RIGHT_BUMPER;
	}
	if buttons2 & 0x40 != 0 {
		mask |= BUTTON_LEFT_STICK;
	}
	if buttons2 & 0x80 != 0 {
		mask |= BUTTON_RIGHT_STICK;
	}

	let left_trigger = normalize_trigger_u16(u16::from_le_bytes([data[2], data[3]]));
	let right_trigger = normalize_trigger_u16(u16::from_le_bytes([data[4], data[5]]));
	let left_y = signed_stick(i16::from_le_bytes([data[8], data[9]]), negate_y);
	let right_y = signed_stick(i16::from_le_bytes([data[12], data[13]]), negate_y);

	Some(GamepadState {
		left_stick: Axis2::new(normalize_axis_i16(i16::from_le_bytes([data[6], data[7]])), left_y),
		right_stick: Axis2::new(normalize_axis_i16(i16::from_le_bytes([data[10], data[11]])), right_y),
		left_trigger,
		right_trigger,
		buttons: mask,
	})
}

/// Returns the number of bytes occupied by a GIP header, including its variable-length size.
fn gip_header_length(report: &[u8]) -> Option<usize> {
	let mut index = 3;
	let mut guard = 0;
	loop {
		let byte = *report.get(index)?;
		index += 1;
		guard += 1;
		if byte & 0x80 == 0 || guard == 4 {
			break;
		}
	}

	if report[1] & 0x80 != 0 {
		guard = 0;
		loop {
			let byte = *report.get(index)?;
			index += 1;
			guard += 1;
			if byte & 0x80 == 0 || guard == 4 {
				break;
			}
		}
	}

	Some(index)
}

fn parse_xbox_360(report: &[u8], negate_y: bool) -> Option<GamepadState> {
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

	let left_y = signed_stick(i16::from_le_bytes([report[8], report[9]]), negate_y);
	let right_y = signed_stick(i16::from_le_bytes([report[12], report[13]]), negate_y);

	let left_stick = Axis2::new(normalize_axis_i16(i16::from_le_bytes([report[6], report[7]])), left_y);
	let right_stick = Axis2::new(normalize_axis_i16(i16::from_le_bytes([report[10], report[11]])), right_y);

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

/// Xbox One triggers are 10-bit values stored in a 16-bit field.
fn normalize_trigger_u16(value: u16) -> f32 {
	(value as f32 / 1023.0).clamp(0.0, 1.0)
}

fn signed_stick(value: i16, negate_y: bool) -> f32 {
	let axis = normalize_axis_i16(value);
	if negate_y { -axis } else { axis }
}

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
		assert_eq!(
			classify_gamepad(0x045E, 0x0B12, Some("Controller"), 0x01, 0x05),
			Some(GamepadKind::Xbox)
		);
		assert_eq!(classify_gamepad(0x045E, 0x0B12, Some("BTM"), 0xFF00, 72), None);
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

		assert_eq!(raw.left_stick, Axis2::new(-1.0, -1.0));
		assert_eq!(raw.right_stick, Axis2::new(1.0, 1.0));
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

		let raw = parse_xbox(&payload, false).expect("valid Xbox report");

		// i16::MAX on Y is stick-up, which stays positive. X at i16::MIN stays left.
		assert_eq!(raw.left_stick, Axis2::new(-1.0, 1.0));
		assert_eq!(raw.right_stick, Axis2::new(1.0, -1.0));
		assert_eq!(raw.left_trigger, 0.0);
		assert_eq!(raw.right_trigger, 1.0);
		assert_eq!(
			raw.buttons,
			BUTTON_TRIGGERS.iter().fold(0, |buttons, (mask, _)| buttons | mask)
		);

		let prefixed = [0x01].into_iter().chain(payload).collect::<Vec<_>>();
		assert_states_equal(parse_xbox(&prefixed, false).expect("valid prefixed Xbox report"), raw);

		let steam = parse_xbox(&payload, true).expect("valid Steam virtual report");
		assert_eq!(steam.left_stick, Axis2::new(-1.0, -1.0));
		assert_eq!(steam.right_stick, Axis2::new(1.0, 1.0));
		assert!(stick_up_is_negative(GamepadKind::Xbox, Some("GamePad-1")));
		assert!(!stick_up_is_negative(GamepadKind::Xbox, Some("Controller")));

		// GIP puts the left stick after the triggers. The 360 offsets would call that the right stick.
		let mut gip = vec![0x20, 0x00, 0x01, 14];
		gip.extend_from_slice(&[0x10, 0x01]);
		gip.extend_from_slice(&1023u16.to_le_bytes());
		gip.extend_from_slice(&0u16.to_le_bytes());
		gip.extend_from_slice(&16_000i16.to_le_bytes());
		gip.extend_from_slice(&16_000i16.to_le_bytes());
		gip.extend_from_slice(&(-16_000i16).to_le_bytes());
		gip.extend_from_slice(&(-8_000i16).to_le_bytes());
		let one = parse_xbox(&gip, false).expect("valid Xbox One report");
		assert!(one.left_stick.x > 0.4, "physical left stick stays on the left stick");
		assert!(one.left_stick.y > 0.4);
		assert!(one.right_stick.x < -0.4);
		assert!(one.right_stick.y < -0.2);
		assert!((one.left_trigger - 1.0).abs() < 0.001);
		assert_eq!(one.right_trigger, 0.0);
		assert_eq!(one.buttons, BUTTON_A | BUTTON_DPAD_UP);
	}

	#[test]
	fn generic_reports_keep_packed_buttons_aligned_and_decode_active_low_x() {
		// The low nibble is the hat, the high nibble starts the contiguous button mask,
		// and bit 14 is the active-low X input used by AppleUserHIDDevice.
		let released_x = [0, 255, 128, 128, 0x11, 0x40, 0];
		let state = parse_generic_joystick(&released_x).expect("valid generic report");

		assert_eq!(state.left_stick, Axis2::new(-1.0, -1.0));
		assert_eq!(state.right_stick, Axis2::new(0.0, 0.0));
		assert_eq!(state.buttons, BUTTON_A | BUTTON_DPAD_UP | BUTTON_DPAD_RIGHT);

		let mut pressed_x = released_x;
		pressed_x[5] &= !0x40;
		let state = parse_generic_joystick(&pressed_x).expect("valid generic report");

		assert_eq!(state.buttons, BUTTON_A | BUTTON_X | BUTTON_DPAD_UP | BUTTON_DPAD_RIGHT);

		let prefixed = [0x07].into_iter().chain(released_x).collect::<Vec<_>>();
		assert_states_equal(
			parse_generic_joystick(&prefixed).expect("valid report-id report"),
			parse_generic_joystick(&released_x).expect("expected test value"),
		);
	}

	#[test]
	fn parsers_reject_reports_without_their_required_payload() {
		assert!(parse_dualshock4(&[0; 8]).is_none());
		assert!(parse_dualsense(&[0; 8]).is_none());
		assert!(parse_generic_joystick(&[0; 4]).is_none());
		assert!(parse_xbox(&[0; 13], false).is_none());
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

		let mut events = Vec::new();
		transition_gamepad_state(device, &mut previous, &mut initialized, baseline, &mut |event| {
			events.push(event)
		});
		assert!(events.is_empty());
		assert!(initialized);
		assert_eq!(previous.buttons, BUTTON_A);

		let noise = GamepadState {
			left_stick: Axis2::new(STICK_EPSILON, 0.0),
			left_trigger: TRIGGER_EPSILON,
			buttons: BUTTON_A,
			..GamepadState::default()
		};

		transition_gamepad_state(device, &mut previous, &mut initialized, noise, &mut |event| {
			events.push(event)
		});
		assert!(events.is_empty());

		let changed = GamepadState {
			left_stick: Axis2::new(0.5, -0.25),
			right_stick: Axis2::new(-0.75, 1.0),
			left_trigger: 0.25,
			right_trigger: 1.0,
			buttons: BUTTON_B,
		};
		transition_gamepad_state(device, &mut previous, &mut initialized, changed, &mut |event| {
			events.push(event)
		});

		assert_eq!(events.len(), 6);
		assert!(events.iter().all(|event| event.device_handle == device));
		let observed = events
			.iter()
			.map(|event| match event.trigger {
				TriggerReference::Name(name) => (name, event.value),
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
