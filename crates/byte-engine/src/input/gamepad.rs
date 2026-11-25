//! Reads gamepads and joysticks over HID and records their sticks, triggers, and buttons as input.
//!
//! [`GamepadSystem`] finds the connected controllers when the application starts, scans again when the operating
//! system reports that a device connected or disconnected, and decodes each controller's reports into `Gamepad.*`
//! triggers. It also sends the outputs an application sets with [`SetDeviceOutput`](super::SetDeviceOutput), such as
//! rumble and light color, encoded in each controller's own output report.

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

/// The largest output report any supported controller takes: Sony Bluetooth output reports are 78 bytes.
const MAX_OUTPUT_REPORT_SIZE: usize = 78;

/// How often Xbox One rumble is sent again while a motor runs, inside the 2.55 s the controller runs it for.
const XBOX_ONE_RUMBLE_REFRESH: Duration = Duration::from_secs(1);

/// The output names a gamepad accepts, as `SetDeviceOutput` names them. See
/// [device outputs](/docs/reference/input#device-outputs).
const LOW_FREQUENCY_RUMBLE: &str = "Gamepad.LowFrequencyRumble";
const HIGH_FREQUENCY_RUMBLE: &str = "Gamepad.HighFrequencyRumble";
const LIGHT: &str = "Gamepad.Light";
const PLAYER_LIGHTS: &str = "Gamepad.PlayerLights";

/// Bits of [`GamepadOutput::set`], one for each group of outputs a report enables separately.
const SET_RUMBLE: u8 = 1 << 0;
const SET_LIGHT: u8 = 1 << 1;
const SET_PLAYER_LIGHTS: u8 = 1 << 2;

/// The `GamepadOutput` struct holds the outputs an application set on one controller, so they can be encoded into
/// that controller's output report once per frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GamepadOutput {
	/// The heavy motor's strength, from 0 to 1.
	low_frequency_rumble: f32,
	/// The light motor's strength, from 0 to 1.
	high_frequency_rumble: f32,
	/// The light's red, green, and blue.
	light: [u8; 3],
	/// One bit for each player indicator light, the leftmost first.
	player_lights: u8,
	/// Which outputs the application has set. A report enables only those, so the controller keeps showing its own
	/// state for the rest.
	set: u8,
}

/// The `OutputProtocol` enum names the output report format a connected controller takes, which depends on its kind
/// and on how it is connected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputProtocol {
	DualShock4Usb,
	DualShock4Bluetooth,
	DualSenseUsb,
	DualSenseBluetooth,
	/// Xbox One and Series controllers, which speak the Gaming Input Protocol over USB.
	XboxOne,
	Xbox360,
}

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
	/// The device and output name pairs a warning was logged for, so each logs once.
	warned_outputs: Vec<(DeviceHandle, &'static str)>,
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
			warned_outputs: Vec::new(),
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
						crate::online_docs_url("use/input")
					);
				}
			}
		}

		let now = Instant::now();
		for device in &mut self.devices {
			device.poll(|event| {
				debug!(
					target: "byte_engine::input::events",
					"Forwarding HID gamepad event: device={:?}, trigger={:?}, value={:?}",
					event.device_handle,
					event.trigger,
					event.value
				);
				input.record(SeatHandle::stub(), event.device_handle, event.trigger, event.value, None);
			});
			device.flush(now);
		}
	}

	/// Records an output an application set on a controller, to send on the next [`GamepadSystem::poll`].
	///
	/// An output the controller doesn't take, a value of the wrong type, or a device that isn't a connected
	/// controller logs one warning for each device and output name, and is ignored.
	pub(crate) fn set_output(&mut self, device_handle: DeviceHandle, name: &'static str, value: Value) {
		let Some(device) = self.devices.iter_mut().find(|device| device.device_handle == Some(device_handle)) else {
			self.warn_output_once(
				device_handle,
				name,
				"Ignored a device output because the device isn't a connected controller. The most likely cause is that the controller disconnected, or that the device handle belongs to another device class. Set outputs only on devices of the Gamepad class",
			);
			return;
		};
		if !device.device.can_write() {
			self.warn_output_once(
				device_handle,
				name,
				"Ignored a device output because the controller can only be read. The most likely cause is that the operating system granted read access only, such as a udev rule without write access on Linux. Grant your user write access to the controller's device node",
			);
			return;
		}

		// Decode into a copy so a value the controller doesn't take leaves its outputs unchanged.
		let mut output = device.output;
		let set = match (name, value) {
			(LOW_FREQUENCY_RUMBLE, Value::Float(strength)) => {
				output.low_frequency_rumble = strength.clamp(0.0, 1.0);
				SET_RUMBLE
			}
			(HIGH_FREQUENCY_RUMBLE, Value::Float(strength)) => {
				output.high_frequency_rumble = strength.clamp(0.0, 1.0);
				SET_RUMBLE
			}
			(LIGHT, Value::Rgba(color)) => {
				output.light = [color.r, color.g, color.b].map(unit_to_u8);
				SET_LIGHT
			}
			(PLAYER_LIGHTS, Value::Int(lights)) => {
				output.player_lights = lights as u8;
				SET_PLAYER_LIGHTS
			}
			_ => 0,
		};
		if set & supported_outputs(device.kind) == 0 {
			self.warn_output_once(
				device_handle,
				name,
				"Ignored a device output because the controller doesn't take that output with that value type. The most likely cause is a misspelled output name, a value of the wrong type, or a controller without that output. Check the output's name, value type, and supported controllers",
			);
			return;
		}
		output.set |= set;
		device.output = output;
		device.output_dirty = true;
	}

	/// Logs `warning` the first time it happens for `device_handle` and `name`, so a value set every frame logs once.
	fn warn_output_once(&mut self, device_handle: DeviceHandle, name: &'static str, warning: &str) {
		if self.warned_outputs.contains(&(device_handle, name)) {
			return;
		}
		self.warned_outputs.push((device_handle, name));
		warn!(
			"{warning} (device {device_handle:?}, output {name:?}). See {}.",
			crate::online_docs_url("reference/input#device-outputs")
		);
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
	/// The output report format, known once the first input report shows how the controller is connected.
	protocol: Option<OutputProtocol>,
	output: GamepadOutput,
	/// Whether `output` changed since it was last sent.
	output_dirty: bool,
	/// When the outputs were last sent, which paces the resends that keep Xbox One rumble running.
	last_output: Instant,
	/// Counts output reports for the protocols that number them.
	output_sequence: u8,
	/// Whether the DualSense light was released from its startup animation, which it needs once before it shows a color.
	light_released: bool,
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
			protocol: None,
			output: GamepadOutput::default(),
			output_dirty: false,
			last_output: Instant::now(),
			output_sequence: 0,
			light_released: false,
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
			if self.protocol.is_none() {
				self.protocol = output_protocol(self.kind, report);
			}
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

	/// Sends the outputs when they changed, or when Xbox One rumble needs its resend.
	///
	/// A busy device keeps the outputs marked as changed, so the next frame sends them. A write error is logged once
	/// and the outputs wait for the next change.
	fn flush(&mut self, now: Instant) {
		let Some(protocol) = self.protocol else {
			// The controller hasn't reported yet, so its report format is unknown; keep the outputs for later.
			return;
		};
		let rumbling = self.output.low_frequency_rumble > 0.0 || self.output.high_frequency_rumble > 0.0;
		let resend = protocol == OutputProtocol::XboxOne
			&& rumbling
			&& now.duration_since(self.last_output) >= XBOX_ONE_RUMBLE_REFRESH;
		if !self.output_dirty && !resend {
			return;
		}

		let release_light = self.output.set & SET_LIGHT != 0 && !self.light_released;
		let (report, length) = encode_output(protocol, &self.output, self.output_sequence, release_light);
		match self.device.write(&report[..length]) {
			Ok(true) => {
				self.output_dirty = false;
				self.last_output = now;
				self.output_sequence = self.output_sequence.wrapping_add(1);
				self.light_released |= release_light;
			}
			Ok(false) => {}
			Err(error) => {
				self.output_dirty = false;
				warn!("{error}");
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

/// Returns the output report format a controller of `kind` takes, judging how it is connected from one input report.
///
/// Sony controllers start a USB report with ID 0x01 and send 64 bytes. Over Bluetooth they first send short 0x01
/// reports, then full 0x11 (DualShock 4) or 0x31 (DualSense) reports. Xbox One reports start with the GIP input
/// command 0x20.
fn output_protocol(kind: GamepadKind, report: &[u8]) -> Option<OutputProtocol> {
	let usb = report.len() >= 64 && report[0] == 0x01;
	match (kind, report.first()?) {
		(GamepadKind::DualShock4, 0x01 | 0x11) if usb => Some(OutputProtocol::DualShock4Usb),
		(GamepadKind::DualShock4, 0x01 | 0x11) => Some(OutputProtocol::DualShock4Bluetooth),
		(GamepadKind::DualSense, 0x01 | 0x31) if usb => Some(OutputProtocol::DualSenseUsb),
		(GamepadKind::DualSense, 0x01 | 0x31) => Some(OutputProtocol::DualSenseBluetooth),
		(GamepadKind::Xbox, 0x20) => Some(OutputProtocol::XboxOne),
		(GamepadKind::Xbox, _) => Some(OutputProtocol::Xbox360),
		_ => None,
	}
}

/// Returns the [`GamepadOutput::set`] bits a controller of `kind` takes.
fn supported_outputs(kind: GamepadKind) -> u8 {
	match kind {
		GamepadKind::DualSense => SET_RUMBLE | SET_LIGHT | SET_PLAYER_LIGHTS,
		GamepadKind::DualShock4 => SET_RUMBLE | SET_LIGHT,
		GamepadKind::Xbox => SET_RUMBLE,
		GamepadKind::GenericJoystick => 0,
	}
}

/// Builds the output report that gives a controller speaking `protocol` the outputs in `output`, and returns it with
/// its length.
///
/// The report starts with its report ID, or with 0 for controllers that don't number their reports, as
/// [`ghi::hid::Device::write`] takes it. `sequence` numbers the report for the protocols that count them, and
/// `release_light` stops the DualSense startup light animation so the light shows the set color. The layouts follow
/// the Linux `hid-sony`, `hid-playstation`, and `xpad` drivers and SDL's HIDAPI drivers.
fn encode_output(
	protocol: OutputProtocol,
	output: &GamepadOutput,
	sequence: u8,
	release_light: bool,
) -> ([u8; MAX_OUTPUT_REPORT_SIZE], usize) {
	let mut report = [0u8; MAX_OUTPUT_REPORT_SIZE];
	let (low, high) = (output.low_frequency_rumble, output.high_frequency_rumble);
	let [red, green, blue] = output.light;
	let rumble = output.set & SET_RUMBLE != 0;
	let light = output.set & SET_LIGHT != 0;

	let length = match protocol {
		OutputProtocol::DualShock4Usb | OutputProtocol::DualShock4Bluetooth => {
			let bluetooth = protocol == OutputProtocol::DualShock4Bluetooth;
			// Bluetooth reports carry two more header bytes: the HID and CRC flags with a 4 ms report interval.
			let (flags, body) = if bluetooth { (3, 6) } else { (1, 4) };
			report[0] = if bluetooth { 0x11 } else { 0x05 };
			if bluetooth {
				report[1] = 0xC4;
			}
			// Bit 0 enables the motors and bit 1 the light; fields left disabled keep their current state.
			report[flags] = u8::from(rumble) | u8::from(light) << 1;
			report[body] = unit_to_u8(high);
			report[body + 1] = unit_to_u8(low);
			report[body + 2..body + 5].copy_from_slice(&[red, green, blue]);
			if bluetooth { seal_sony_bluetooth_report(&mut report) } else { 32 }
		}
		OutputProtocol::DualSenseUsb | OutputProtocol::DualSenseBluetooth => {
			let bluetooth = protocol == OutputProtocol::DualSenseBluetooth;
			// The 47-byte effect block follows the report ID, and over Bluetooth also a sequence and a tag byte.
			let common = if bluetooth { 3 } else { 1 };
			report[0] = if bluetooth { 0x31 } else { 0x02 };
			if bluetooth {
				report[1] = (sequence & 0x0F) << 4;
				report[2] = 0x10;
			}
			let effects = &mut report[common..common + 47];
			if rumble {
				// Compatible vibration with haptics selected makes the voice-coil actuators behave as rumble motors.
				effects[0] |= 0x01 | 0x02;
				effects[2] = unit_to_u8(high);
				effects[3] = unit_to_u8(low);
			}
			if light {
				effects[1] |= 0x04;
				effects[44..47].copy_from_slice(&[red, green, blue]);
			}
			if release_light {
				// Light-out setup fades the startup animation, which otherwise keeps the light from taking a color.
				effects[38] |= 0x02;
				effects[41] = 0x02;
			}
			if output.set & SET_PLAYER_LIGHTS != 0 {
				effects[1] |= 0x10;
				effects[43] = output.player_lights & 0x1F;
			}
			if bluetooth { seal_sony_bluetooth_report(&mut report) } else { 48 }
		}
		OutputProtocol::XboxOne => {
			// The GIP rumble command: every motor enabled, trigger motors off, the longest duration, no delay, and
			// the longest repeat. Motor strengths run from 0 to 127.
			let strength = |value: f32| (value * 127.0).round() as u8;
			report[..13].copy_from_slice(&[
				0x09,
				0x00,
				sequence,
				0x09,
				0x00,
				0x0F,
				0x00,
				0x00,
				strength(low),
				strength(high),
				0xFF,
				0x00,
				0xFF,
			]);
			13
		}
		OutputProtocol::Xbox360 => {
			// The unnumbered rumble report: type 0x00 is the report ID position, then size 0x08 and the two motors.
			report[..8].copy_from_slice(&[0x00, 0x08, 0x00, unit_to_u8(low), unit_to_u8(high), 0x00, 0x00, 0x00]);
			8
		}
	};
	(report, length)
}

/// Writes the CRC-32 that Sony controllers check on Bluetooth output reports into the last four bytes, and returns the
/// report length.
///
/// The checksum covers the Bluetooth HID output header byte 0xA2 followed by the report without its checksum.
fn seal_sony_bluetooth_report(report: &mut [u8; MAX_OUTPUT_REPORT_SIZE]) -> usize {
	let checksum = !crc32_update(crc32_update(!0, &[0xA2]), &report[..MAX_OUTPUT_REPORT_SIZE - 4]);
	report[MAX_OUTPUT_REPORT_SIZE - 4..].copy_from_slice(&checksum.to_le_bytes());
	MAX_OUTPUT_REPORT_SIZE
}

/// The reflected CRC-32 (IEEE 802.3) lookup table, built at compile time.
const CRC32_TABLE: [u32; 256] = {
	let mut table = [0u32; 256];
	let mut index = 0;
	while index < 256 {
		let mut value = index as u32;
		let mut bit = 0;
		while bit < 8 {
			value = if value & 1 != 0 { 0xEDB8_8320 ^ (value >> 1) } else { value >> 1 };
			bit += 1;
		}
		table[index] = value;
		index += 1;
	}
	table
};

/// Folds `bytes` into a running CRC-32 register.
fn crc32_update(mut register: u32, bytes: &[u8]) -> u32 {
	for &byte in bytes {
		register = CRC32_TABLE[((register ^ byte as u32) & 0xFF) as usize] ^ (register >> 8);
	}
	register
}

/// Converts a 0 to 1 value to a byte, clamping values outside that range.
fn unit_to_u8(value: f32) -> u8 {
	(value.clamp(0.0, 1.0) * 255.0).round() as u8
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
		let device = DeviceHandle::from_index(7);
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

	fn encoded(protocol: OutputProtocol, output: GamepadOutput, sequence: u8, release_light: bool) -> Vec<u8> {
		let (report, length) = encode_output(protocol, &output, sequence, release_light);
		report[..length].to_vec()
	}

	#[test]
	fn output_protocol_follows_the_connection_each_input_report_reveals() {
		let usb = [0x01; 64];
		let bluetooth_simple = [0x01; 10];
		assert_eq!(output_protocol(GamepadKind::DualShock4, &usb), Some(OutputProtocol::DualShock4Usb));
		assert_eq!(output_protocol(GamepadKind::DualShock4, &bluetooth_simple), Some(OutputProtocol::DualShock4Bluetooth));
		assert_eq!(output_protocol(GamepadKind::DualShock4, &[0x11; 78]), Some(OutputProtocol::DualShock4Bluetooth));
		assert_eq!(output_protocol(GamepadKind::DualSense, &usb), Some(OutputProtocol::DualSenseUsb));
		assert_eq!(output_protocol(GamepadKind::DualSense, &bluetooth_simple), Some(OutputProtocol::DualSenseBluetooth));
		assert_eq!(output_protocol(GamepadKind::DualSense, &[0x31; 78]), Some(OutputProtocol::DualSenseBluetooth));
		assert_eq!(output_protocol(GamepadKind::Xbox, &[0x20, 0x00, 0x01, 0x0E]), Some(OutputProtocol::XboxOne));
		assert_eq!(output_protocol(GamepadKind::Xbox, &[0x00, 0x14]), Some(OutputProtocol::Xbox360));
		assert_eq!(output_protocol(GamepadKind::GenericJoystick, &usb), None);
		assert_eq!(output_protocol(GamepadKind::DualSense, &[]), None);
	}

	#[test]
	fn dualsense_usb_report_enables_only_the_outputs_that_were_set() {
		let output = GamepadOutput {
			low_frequency_rumble: 0.5,
			high_frequency_rumble: 1.0,
			light: [255, 0, 128],
			player_lights: 0b0_0100,
			set: SET_RUMBLE | SET_PLAYER_LIGHTS,
		};
		let mut expected = vec![0u8; 48];
		expected[0] = 0x02;
		expected[1] = 0x03;
		expected[2] = 0x10;
		expected[3] = 255;
		expected[4] = 128;
		expected[44] = 0b0_0100;
		// The light was never set, so its color stays out of the report.
		assert_eq!(encoded(OutputProtocol::DualSenseUsb, output, 0, false), expected);
	}

	#[test]
	fn dualsense_bluetooth_report_carries_sequence_light_release_and_checksum() {
		let output = GamepadOutput {
			low_frequency_rumble: 0.5,
			high_frequency_rumble: 1.0,
			light: [255, 0, 128],
			player_lights: 0,
			set: SET_RUMBLE | SET_LIGHT,
		};
		let mut expected = vec![0u8; 78];
		expected[..6].copy_from_slice(&[0x31, 0x30, 0x10, 0x03, 0x04, 255]);
		expected[6] = 128;
		expected[3 + 38] = 0x02;
		expected[3 + 41] = 0x02;
		expected[3 + 44..3 + 47].copy_from_slice(&[255, 0, 128]);
		expected[74..].copy_from_slice(&[0xCB, 0x8D, 0x4B, 0x45]);
		assert_eq!(encoded(OutputProtocol::DualSenseBluetooth, output, 3, true), expected);
	}

	#[test]
	fn dualshock4_reports_place_motors_and_light_after_their_transport_header() {
		let rumble = GamepadOutput {
			low_frequency_rumble: 1.0,
			high_frequency_rumble: 0.5,
			set: SET_RUMBLE,
			..GamepadOutput::default()
		};
		let mut expected = vec![0u8; 32];
		expected[..6].copy_from_slice(&[0x05, 0x01, 0x00, 0x00, 128, 255]);
		assert_eq!(encoded(OutputProtocol::DualShock4Usb, rumble, 0, false), expected);

		let light = GamepadOutput {
			light: [10, 20, 30],
			set: SET_LIGHT,
			..GamepadOutput::default()
		};
		let mut expected = vec![0u8; 78];
		expected[..4].copy_from_slice(&[0x11, 0xC4, 0x00, 0x02]);
		expected[8..11].copy_from_slice(&[10, 20, 30]);
		expected[74..].copy_from_slice(&[0xF2, 0xFA, 0xD6, 0x28]);
		assert_eq!(encoded(OutputProtocol::DualShock4Bluetooth, light, 0, false), expected);
	}

	#[test]
	fn xbox_reports_scale_each_motor_to_its_protocol_range() {
		let output = GamepadOutput {
			low_frequency_rumble: 1.0,
			high_frequency_rumble: 0.5,
			set: SET_RUMBLE,
			..GamepadOutput::default()
		};
		assert_eq!(
			encoded(OutputProtocol::XboxOne, output, 7, false),
			[0x09, 0x00, 7, 0x09, 0x00, 0x0F, 0x00, 0x00, 127, 64, 0xFF, 0x00, 0xFF]
		);
		assert_eq!(
			encoded(OutputProtocol::Xbox360, output, 0, false),
			[0x00, 0x08, 0x00, 255, 128, 0x00, 0x00, 0x00]
		);
	}
}
