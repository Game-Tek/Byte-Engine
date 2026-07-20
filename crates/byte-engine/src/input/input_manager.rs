//! Runtime storage and evaluation for input devices, triggers, and actions.
//!
//! Device classes describe layouts such as "Mouse" or "Gamepad"; devices are
//! concrete instances of those classes; triggers are named values on a class.
//! Actions map one or more triggers into application concepts such as movement.
//!
//! Most headed applications receive an [`InputManager`] through
//! `GraphicsApplication` and install the standard layouts with
//! `setup_default_input`. Custom runtimes construct it from an action factory
//! listener and an action event channel, then register classes before creating
//! devices.

/// The synthetic device reserved for actions triggered without physical input.
const MANUAL_ACTION_DEVICE: DeviceHandle = DeviceHandle(u32::MAX);

/// The [`InputManager`] struct owns input topology, current values, and action
/// evaluation state.
///
/// Feed platform records into this type before calling its update path. For the
/// standard headed integration, use
/// `process_default_window_input` rather than duplicating the mouse and keyboard
/// trigger-name mapping.
///
/// After registration, subscribe through [`Self::event_channel`], queue physical
/// values with [`Self::record_trigger_value_for_device`], and call
/// [`Self::update`] once per application tick.
pub struct InputManager {
	device_classes: Vec<DeviceClass>,
	triggers: Vec<Trigger>,
	devices: Vec<Device>,
	records: Vec<Record>,
	actions: Vec<InputAction>,
	/// Stores the latest trigger value for each seat and device.
	trigger_values: HashMap<(SeatHandle, DeviceHandle, TriggerHandle), Record>,
	/// Stores the latest action value for each seat and device.
	action_values: HashMap<(SeatHandle, DeviceHandle, ActionHandle), Value>,
	pending_manual_actions: Vec<(SeatHandle, ActionHandle, Value)>,
	action_listener: DefaultListener<CreateMessage<Action>>,
	event_channel: DefaultChannel<ActionEvent>,
}

impl InputManager {
	/// Creates an input manager connected to action creation and event channels.
	pub fn new(action_listener: DefaultListener<CreateMessage<Action>>, event_channel: DefaultChannel<ActionEvent>) -> Self {
		InputManager {
			device_classes: Vec::new(),
			triggers: Vec::new(),
			devices: Vec::new(),
			records: Vec::new(),
			actions: Vec::new(),
			trigger_values: HashMap::with_capacity(512),
			action_values: HashMap::with_capacity(64),
			pending_manual_actions: Vec::new(),
			action_listener,
			event_channel,
		}
	}

	/// Registers a named device class, such as `Keyboard`.
	///
	/// Use PascalCase for `name` so trigger paths remain consistent.
	pub fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		let device_class = DeviceClass { name: name.to_string() };

		DeviceClassHandle(insert_return_length(&mut self.device_classes, device_class) as u32)
	}

	/// Registers a named trigger on a device class.
	///
	/// `value_type` defines the trigger's initial value and valid Rust type. Use
	/// the returned [`TriggerHandle`] to bind actions or submit input records.
	pub fn register_trigger<T>(
		&mut self,
		device_handle: &DeviceClassHandle,
		name: &str,
		value_type: TriggerDescription<T>,
	) -> TriggerHandle
	where
		T: InputValue + Into<Value>,
	{
		let default = value_type.default;

		let default: Value = default.into();
		let default_value_type: Types = default.into();

		assert_eq!(
			default_value_type,
			T::get_type(),
			"Default value type does not match input source type"
		);

		let input_source = Trigger {
			device_class_handle: *device_handle,
			name: name.to_string(),
			r#type: T::get_type(),
			default,
		};

		TriggerHandle(insert_return_length(&mut self.triggers, input_source) as u32)
	}

	/// Reserves an output destination on a device class.
	///
	/// Input destinations represent device feedback, such as gamepad rumble.
	pub fn register_input_destination<T: InputValue>(
		&mut self,
		_device_class_handle: &DeviceClassHandle,
		_name: &str,
		_value_type: TriggerDescription<T>,
	) -> TriggerHandle {
		TriggerHandle(0)
	}

	/// Creates one concrete device from a registered class.
	///
	/// Call this once for each physical or virtual device, such as each connected
	/// gamepad.
	pub fn create_device(&mut self, device_class_handle: &DeviceClassHandle) -> DeviceHandle {
		let other_device = self
			.devices
			.iter()
			.filter(|d| d.device_class_handle.0 == device_class_handle.0)
			.min_by_key(|d| d.index);

		let index = match other_device {
			Some(device) => device.index + 1,
			None => 0,
		};
		assert_ne!(
			index, MANUAL_ACTION_DEVICE.0,
			"Physical device index exhausted reserved manual action handle"
		);

		let device = Device {
			device_class_handle: *device_class_handle,
			index,
		};

		DeviceHandle(insert_return_length(&mut self.devices, device) as u32)
	}

	/// Queues a trigger value for a device and seat.
	///
	/// The value becomes visible when [`Self::update`] processes the queue. The
	/// manager ignores unknown triggers and values with the wrong [`Types`].
	pub fn record_trigger_value_for_device(
		&mut self,
		seat_handle: SeatHandle,
		device_handle: DeviceHandle,
		trigger_reference: TriggerReference,
		value: Value,
	) {
		let trigger = if let Some(trigger) = self.get_trigger_from_trigger_reference(&trigger_reference) {
			trigger
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		if trigger.r#type != value.into() {
			warn!("Tried to record an extraneous type into input source: {}", trigger.name);
			return; // Value type does not match input source declared type, so don't record.
		}

		let trigger_handle = if let Some(input_source_handle) = self.to_trigger_handle(&trigger_reference) {
			input_source_handle
		} else {
			warn!("Tried to record an input source action that doesn't exist");
			return;
		};

		let time = std::time::SystemTime::now();

		let record = Record {
			seat_handle,
			device_handle,
			trigger_handle,
			value,
			time,
		};

		self.records.push(record);
	}

	/// Resolves queued trigger and manual-action values, then emits action events.
	///
	/// Call this once per application tick after recording platform input. Next,
	/// drain a listener created from [`Self::event_channel`] to handle the resolved
	/// [`ActionEvent`] values.
	pub fn update(&mut self, frame_allocator: &bumpalo::Bump) {
		while let Some(message) = self.action_listener.read() {
			let handle = *message.handle();
			let action = message.into_data();

			let (name, r#type, input_events, tick_policy) = (action.name, action.r#type, action.bindings, action.tick_policy);

			let input_event = InputAction {
				name: name.to_string(),
				r#type,
				trigger_mappings: input_events
					.iter()
					.filter_map(|input_event| {
						Some(TriggerMapping {
							trigger_handle: self.to_trigger_handle(&input_event.input_source)?,
							mapping: input_event.mapping.value,
							function: Some(input_event.mapping.function),
						})
					})
					.collect(),
				handle: Some(handle),
				tick_policy,
			};

			self.actions.push(input_event);
		}

		// Phase A: Process new records (if any) and resolve actions that received input.
		if !self.records.is_empty() {
			let records = frame_allocator.alloc_slice_fill_iter(self.records.drain(..));

			// Sort records by source first and time second so each source's last record is the most recent.
			records.sort_by(compare_source_then_time);
			let record_count = compact_latest_by_source(records);
			let records = &records[..record_count];

			for record in records {
				self.trigger_values
					.insert((record.seat_handle, record.device_handle, record.trigger_handle), *record);
			}

			for (i, action) in self.actions.iter().enumerate() {
				let action_records = records
					.iter()
					.filter(|r| action.trigger_mappings.iter().any(|tm| tm.trigger_handle == r.trigger_handle));

				let most_recent_record = action_records.max_by_key(|r| r.time);

				let record = if let Some(record) = most_recent_record {
					record
				} else {
					continue;
				};

				let value = if let Some(value) = resolve_action_value(action, record, &self.trigger_values, frame_allocator) {
					value
				} else {
					continue;
				};

				self.action_values
					.insert((record.seat_handle, record.device_handle, ActionHandle(i as u32)), value);

				// OnChange actions emit here (on actual input change).
				if let Some(handle) = &action.handle {
					log::debug!(
						target: "byte_engine::input::actions",
						"Emitting input action event: policy={:?}, action={}, handle={:?}, seat={:?}, device={:?}, value={:?}",
						action.tick_policy,
						action.name,
						handle,
						record.seat_handle,
						record.device_handle,
						value
					);
					self.event_channel.send(ActionEvent::new(record.seat_handle, *handle, value));
				}
			}
		}

		// Manual actions enter the same state table as physical actions, using the
		// reserved device because they do not originate from a concrete device.
		for (seat_handle, action_handle, value) in self.pending_manual_actions.drain(..) {
			self.action_values
				.insert((seat_handle, MANUAL_ACTION_DEVICE, action_handle), value);

			if let Some(action) = self.actions.get(action_handle.0 as usize) {
				if let Some(handle) = action.handle {
					self.event_channel.send(ActionEvent::new(seat_handle, handle, value));
				}
			}
		}

		// Phase B: Tick-based emission for WhileActive and Always actions.
		// Iterates all actions and emits events based on their tick policy using the
		// most recently resolved value. Only emits for devices that have previously
		// interacted with the action.
		let entries = frame_allocator.alloc_slice_fill_iter(self.action_values.iter().map(|(key, value)| (*key, *value)));
		for &((seat_handle, device_handle, action_handle), value) in entries.iter() {
			let action = &self.actions[action_handle.0 as usize];

			let handle = match &action.handle {
				Some(h) => *h,
				None => continue,
			};

			match action.tick_policy {
				TickPolicy::OnChange => {} // Already handled in Phase A.
				TickPolicy::WhileActive => {
					if !value.is_default() {
						log::debug!(
							target: "byte_engine::input::actions",
							"Emitting input action event: policy={:?}, action={}, handle={:?}, seat={:?}, device={:?}, value={:?}",
							action.tick_policy,
							action.name,
							handle,
							seat_handle,
							device_handle,
							value
						);
						self.event_channel.send(ActionEvent::new(seat_handle, handle, value));
					}
				}
				TickPolicy::Always => {
					log::debug!(
						target: "byte_engine::input::actions",
						"Emitting input action event: policy={:?}, action={}, handle={:?}, seat={:?}, device={:?}, value={:?}",
						action.tick_policy,
						action.name,
						handle,
						seat_handle,
						device_handle,
						value
					);
					self.event_channel.send(ActionEvent::new(seat_handle, handle, value));
				}
			}
		}
	}

	/// Queues an action value for emission during the next [`Self::update`] call.
	///
	/// After this call succeeds, run [`Self::update`] and read the action from a
	/// listener created through [`Self::event_channel`].
	pub fn trigger_action(
		&mut self,
		seat_handle: SeatHandle,
		action_handle: ActionHandle,
		value: Value,
	) -> Result<(), InputActionError> {
		let action = self
			.actions
			.get(action_handle.0 as usize)
			.ok_or(InputActionError::UnknownAction(action_handle))?;
		let actual_type = value.into();
		if action.r#type != actual_type {
			return Err(InputActionError::TypeMismatch {
				expected: action.r#type,
				actual: actual_type,
			});
		}

		self.pending_manual_actions.push((seat_handle, action_handle, value));
		Ok(())
	}

	/// Returns the synthetic device used by [`Self::trigger_action`].
	pub fn manual_action_device_handle() -> DeviceHandle {
		MANUAL_ACTION_DEVICE
	}

	/// Creates an action that emits when its resolved value changes.
	///
	/// Next, record values for one of the action's trigger mappings and call
	/// [`Self::update`]. Use [`Self::event_channel`] to receive the result.
	pub fn create_action(
		&mut self,
		name: &str,
		r#type: Types,
		action_binding_descriptions: &[ActionBindingDescription],
	) -> ActionHandle {
		self.create_action_with_tick_policy(name, r#type, action_binding_descriptions, TickPolicy::OnChange)
	}

	/// Creates an action with a specific tick policy controlling how frequently events are emitted.
	///
	/// Next, create a listener from [`Self::event_channel`], record trigger values,
	/// and call [`Self::update`] once per tick.
	pub fn create_action_with_tick_policy(
		&mut self,
		name: &str,
		r#type: Types,
		action_binding_descriptions: &[ActionBindingDescription],
		tick_policy: TickPolicy,
	) -> ActionHandle {
		let input_event = InputAction {
			name: name.to_string(),
			r#type,
			trigger_mappings: action_binding_descriptions
				.iter()
				.filter_map(|input_event| {
					Some(TriggerMapping {
						trigger_handle: self.to_trigger_handle(&input_event.input_source)?,
						mapping: input_event.mapping.value,
						function: Some(input_event.mapping.function),
					})
				})
				.collect(),
			handle: None,
			tick_policy,
		};

		let handle = ActionHandle(self.actions.len() as u32);
		self.actions.push(input_event);

		handle
	}

	/// Returns all devices that belong to the named class.
	pub fn get_devices_by_class_name(&self, class_name: &str) -> Option<Vec<DeviceHandle>> {
		let device_class_handle = self
			.device_classes
			.iter()
			.enumerate()
			.find_map(|(i, d)| (d.name == class_name).then_some(DeviceClassHandle(i as u32)))?;
		Some(
			self.devices
				.iter()
				.filter(|d| d.device_class_handle == device_class_handle)
				.map(|d| DeviceHandle(d.index))
				.collect(),
		)
	}

	/// Returns the latest processed trigger value for a seat and device.
	///
	/// Returns the trigger's default value when no matching record exists.
	pub fn get_trigger_value_for_device(
		&self,
		seat_handle: SeatHandle,
		device_handle: DeviceHandle,
		trigger_reference: TriggerReference,
	) -> Result<Value, ()> {
		let trigger_handle = self.to_trigger_handle(&trigger_reference).ok_or(())?;

		let trigger = self.get_trigger_from_trigger_reference(&trigger_reference).ok_or(())?;

		Ok(self
			.trigger_values
			.get(&(seat_handle, device_handle, trigger_handle))
			.map(|record| record.value)
			.unwrap_or(trigger.default))
	}

	/// Returns the latest resolved action state for a seat and device.
	pub fn get_action_state(
		&self,
		seat_handle: SeatHandle,
		action_handle: ActionHandle,
		device_handle: DeviceHandle,
	) -> InputEventState {
		self.action_values
			.get(&(seat_handle, device_handle, action_handle))
			.map(|record| InputEventState {
				seat_handle,
				device_handle,
				input_event_handle: action_handle,
				value: *record,
			})
			.unwrap_or_else(|| {
				let action = self.actions.get(action_handle.0 as usize).unwrap();
				let default_value = match action.r#type {
					Types::Boolean => Value::Bool(false),
					Types::Float => Value::Float(0f32),
					Types::Vector2 => Value::Vector2(Vector2 { x: 0f32, y: 0f32 }),
					Types::Vector3 => Value::Vector3(Vector3 {
						x: 0f32,
						y: 0f32,
						z: 0f32,
					}),
					_ => panic!("Not implemented!"),
				};

				InputEventState {
					seat_handle,
					device_handle,
					input_event_handle: action_handle,
					value: default_value,
				}
			})
	}

	fn get_trigger_from_trigger_reference(&self, trigger_reference: &TriggerReference) -> Option<&Trigger> {
		self.to_trigger_handle(trigger_reference)
			.and_then(|trigger_handle| self.triggers.get(trigger_handle.0 as usize))
	}

	fn get_device(&self, device_handle: &DeviceHandle) -> &Device {
		&self.devices[device_handle.0 as usize]
	}

	fn to_trigger_handle(&self, trigger_reference: &TriggerReference) -> Option<TriggerHandle> {
		match trigger_reference {
			TriggerReference::Handle(handle) => Some(*handle),
			TriggerReference::Name(name) => {
				let tokens = (*name).split('.');

				let input_device_class = self
					.device_classes
					.iter()
					.enumerate()
					.find(|(_, device_class)| device_class.name == tokens.clone().next().unwrap());

				if let Some((idc_index, _)) = input_device_class {
					let input_device_class_handle = DeviceClassHandle(idc_index as u32);

					let trigger = self.triggers.iter().enumerate().find(|(_, input_source)| {
						input_source.name == tokens.clone().next_back().unwrap()
							&& input_source.device_class_handle == input_device_class_handle
					});

					trigger.map(|trigger| TriggerHandle(trigger.0 as u32))
				} else {
					None
				}
			}
		}
	}

	/// Returns the channel that publishes resolved action events.
	///
	/// Next, call [`DefaultChannel::listener`] and keep that listener with the
	/// application system that handles the action.
	pub fn event_channel(&self) -> &DefaultChannel<ActionEvent> {
		&self.event_channel
	}
}

/// The `InputActionError` enum describes why a manual action could not be queued.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InputActionError {
	/// The requested action handle is not registered with the manager.
	UnknownAction(ActionHandle),
	/// The supplied value type does not match the action declaration.
	TypeMismatch { expected: Types, actual: Types },
}

#[derive(Copy, Clone, Debug)]
/// The `TriggerReference` enum lets callers select a trigger by handle or name.
pub enum TriggerReference {
	/// Selects a trigger by its registered handle.
	Handle(TriggerHandle),
	/// Selects a trigger by its `DeviceClass.Trigger` name.
	Name(&'static str),
}

impl Message for Value {}

#[cfg(test)]
mod tests {
	use std::{cell::RefCell, ops::DerefMut, rc::Rc, sync::Arc};

	use math::Quaternion;

	use super::*;
	use crate::input::ActionBindingDescription;
	use crate::input::{
		input_trigger::TriggerDescription,
		utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
		ValueMapping,
	};

	fn declare_vr_headset_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Headset");

		let source_description = TriggerDescription::new(
			Vector3::new(0f32, 1.80f32, 0f32),
			Vector3::new(0f32, 0f32, 0f32),
			Vector3::min_value(),
			Vector3::max_value(),
		);

		let _position_input_source = input_manager.register_trigger(&device_class_handle, "Position", source_description);

		let _rotation_input_source = input_manager.register_trigger(
			&device_class_handle,
			"Orientation",
			TriggerDescription::<Quaternion>::default(),
		);

		device_class_handle
	}

	fn declare_funky_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Funky");

		let _funky_input_source =
			input_manager.register_trigger(&device_class_handle, "Int", TriggerDescription::new(0, 0, 0, 3));

		input_manager.register_trigger(
			&device_class_handle,
			"Rgba",
			TriggerDescription::new(
				RGBA {
					r: 0.0f32,
					g: 0.0f32,
					b: 0.0f32,
					a: 0.0f32,
				},
				RGBA {
					r: 0.0f32,
					g: 0.0f32,
					b: 0.0f32,
					a: 0.0f32,
				},
				RGBA {
					r: 0.0f32,
					g: 0.0f32,
					b: 0.0f32,
					a: 0.0f32,
				},
				RGBA {
					r: 1.0f32,
					g: 1.0f32,
					b: 1.0f32,
					a: 1.0f32,
				},
			),
		);

		device_class_handle
	}

	fn build_input_manager() -> InputManager {
		let action_chanel = DefaultChannel::new();
		let action_listener = action_chanel.listener();
		let event_channel = DefaultChannel::new();

		InputManager::new(action_listener, event_channel)
	}

	fn update_input_manager(input_manager: &mut InputManager) {
		let frame_allocator = bumpalo::Bump::new();
		input_manager.update(&frame_allocator);
	}

	#[test]
	fn create_device_class() {
		let mut input_manager = build_input_manager();

		let _device_class_handle = input_manager.register_device_class("Keyboard");
	}

	#[test]
	fn create_input_sources() {
		let mut input_manager = build_input_manager();

		let gamepad_class_handle = register_gamepad_device_class(&mut input_manager);
		register_keyboard_device_class(&mut input_manager);

		let stick_source_description = TriggerDescription::new(
			Vector2::zero(),
			Vector2::zero(),
			Vector2 { x: -1.0, y: -1.0 },
			Vector2 { x: 1.0, y: 1.0 },
		);

		let _gamepad_left_stick_input_source =
			input_manager.register_trigger(&gamepad_class_handle, "LeftStick", stick_source_description);
		let _gamepad_right_stick_input_source =
			input_manager.register_trigger(&gamepad_class_handle, "RightStick", stick_source_description);

		let trigger_source_description = TriggerDescription::<f32>::default();

		let _trigger_input_source =
			input_manager.register_trigger(&gamepad_class_handle, "LeftTrigger", trigger_source_description);
	}

	#[test]
	fn test_boolean_source_input_overlap_action() {
		let mut input_manager = build_input_manager();

		let x = register_keyboard_device_class(&mut input_manager);

		let action = input_manager.create_action(
			"MoveLongitudinally",
			Types::Float,
			&[
				ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32)),
				ActionBindingDescription::new("Keyboard.Down").mapped(ValueMapping::new(Function::Boolean, -1f32)),
			],
		);

		let device = input_manager.create_device(&x);
		let seat = SeatHandle::stub();

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Float(-1f32)
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Float(-1f32)
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Float(-1f32)
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, Value::Float(0f32));
	}

	#[test]
	fn test_boolean_trigger_2d_action_binding_combination() {
		let action_chanel = DefaultChannel::new();
		let action_listener = action_chanel.listener();
		let event_channel = DefaultChannel::new();

		let mut input_manager = InputManager::new(action_listener, event_channel);

		let x = register_keyboard_device_class(&mut input_manager);

		let action = input_manager.create_action(
			"Move",
			Types::Vector2,
			&[
				ActionBindingDescription::new("Keyboard.Up")
					.mapped(ValueMapping::new(Function::Boolean, Vector2::new(0f32, 1f32))),
				ActionBindingDescription::new("Keyboard.Down")
					.mapped(ValueMapping::new(Function::Boolean, Vector2::new(0f32, -1f32))),
				ActionBindingDescription::new("Keyboard.Left")
					.mapped(ValueMapping::new(Function::Boolean, Vector2::new(-1f32, 0f32))),
				ActionBindingDescription::new("Keyboard.Right")
					.mapped(ValueMapping::new(Function::Boolean, Vector2::new(1f32, 0f32))),
			],
		);

		let device = input_manager.create_device(&x);
		let seat = SeatHandle::stub();

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Vector2(Vector2::new(0f32, 0f32))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Vector2(Vector2::new(1f32 / 2f32.sqrt(), 1f32 / 2f32.sqrt()))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), false.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Vector2(Vector2::new(0f32, 0f32))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Left"), true.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), true.into());

		update_input_manager(&mut input_manager);

		assert_eq!(
			input_manager.get_action_state(seat, action, device).value,
			Value::Vector2(Vector2::new(0f32, 0f32))
		);
	}

	fn record_and_assert_input_source_action_sequence<A, Z>(
		input_manager: &mut InputManager,
		device: DeviceHandle,
		trigger_reference: TriggerReference,
		a: A,
		b: A,
		z: Z,
	) where
		A: Into<Value>,
		Z: Into<Value>,
	{
		let a: Value = a.into();
		let b: Value = b.into();
		let z: Value = z.into();
		let seat = SeatHandle::stub();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			a
		); // Assert default value

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, b); // Record alternate value.

		update_input_manager(input_manager);

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			b
		); // Assert alternate value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value.

		update_input_manager(input_manager);

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			a
		); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value again.

		update_input_manager(input_manager);

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			a
		); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value.
		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, b); // Record alternate value after recording default value.

		update_input_manager(input_manager);

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			b
		); // Assert value is last value recorded.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, z); // Record a different type.

		update_input_manager(input_manager);

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.unwrap(),
			b
		);
		// Assert last value is kept after recording a different type.
	}

	#[test]
	fn record_bool_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_keyboard_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, false, true, 961f32);
	}

	#[test]
	fn record_unicode_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_keyboard_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Character");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, '\0', 'a', true);
	}

	#[test]
	fn unicode_action_emits_character_events() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		let device_class_handle = register_keyboard_device_class(&mut input_manager);
		let device = input_manager.create_device(&device_class_handle);
		let seat = SeatHandle::stub();

		let action = Action::new(
			"KeyboardCharacter",
			&[ActionBindingDescription::new("Keyboard.Character")],
			Types::Unicode,
		);
		let handle = factory.create(action);
		update_input_manager(&mut input_manager);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Character"), 'é'.into());
		update_input_manager(&mut input_manager);

		let event = Listener::read(&mut event_listener).expect("expected character action event");
		assert_eq!(event.handle(), handle);
		assert_eq!(event.value(), Value::Unicode('é'));
		assert!(Listener::read(&mut event_listener).is_none());
	}

	#[test]
	fn record_int_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Funky.Int");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0, 1, true);
	}

	#[test]
	fn record_float_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_gamepad_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Gamepad.LeftTrigger");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0.0f32, 1f32, true);
	}

	#[test]
	fn record_vector2_input_source_action() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_gamepad_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Gamepad.LeftStick");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			Vector2 { x: 0f32, y: 0f32 },
			Vector2 { x: 1f32, y: 1f32 },
			true,
		);
	}

	#[test]
	fn record_vector3_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Headset.Position");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			Vector3 {
				x: 0f32,
				y: 1.8f32,
				z: 0f32,
			},
			Vector3 {
				x: 1f32,
				y: 1f32,
				z: 1f32,
			},
			true,
		);
	}

	#[test]
	fn record_quaternion_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = declare_vr_headset_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Headset.Orientation");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			Quaternion::from_euler_angles(0f32, 0f32, 0f32),
			Quaternion::from_euler_angles(1f32, 1f32, 1f32),
			true,
		);
	}

	#[test]
	fn record_rgba_input_source_actions() {
		let mut input_manager = build_input_manager();

		let device_class_handle = declare_funky_input_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Funky.Rgba");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			RGBA {
				r: 0f32,
				g: 0f32,
				b: 0f32,
				a: 0f32,
			},
			RGBA {
				r: 1f32,
				g: 1f32,
				b: 1f32,
				a: 1f32,
			},
			true,
		);
	}

	fn record_and_assert_boolean_input_source_action_interpolation<T>(
		input_manager: &mut InputManager,
		device: DeviceHandle,
		handle: TriggerReference,
		action_name: &str,
		input_source_name: &'static str,
		a: T,
		b: T,
	) where
		T: InputValue + Into<Value> + Into<ValueMapping> + Copy,
	{
		let action = input_manager.create_action(
			action_name,
			T::get_type(),
			&[ActionBindingDescription::new(input_source_name).mapped(b.into())],
		);
		let seat = SeatHandle::stub();

		assert_eq!(input_manager.get_action_state(seat, action, device).value, a.into());

		input_manager.record_trigger_value_for_device(seat, device, handle, true.into());

		update_input_manager(input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, b.into());

		input_manager.record_trigger_value_for_device(seat, device, handle, false.into());

		update_input_manager(input_manager);

		assert_eq!(input_manager.get_action_state(seat, action, device).value, a.into());
	}

	#[test]
	fn test_boolean_float_interpolation() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_keyboard_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"MoveForward",
			"Keyboard.Up",
			0f32,
			1f32,
		);
	}

	#[test]
	fn test_boolean_vector2_interpolation() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_keyboard_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"MoveForward",
			"Keyboard.Up",
			Vector2::zero(),
			Vector2::new(0f32, 1f32),
		);
	}

	#[test]
	fn test_boolean_vector3_interpolation() {
		let mut input_manager = build_input_manager();

		let device_class_handle = register_keyboard_device_class(&mut input_manager);

		let device = input_manager.create_device(&device_class_handle);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"MoveForward",
			"Keyboard.Up",
			Vector3::zero(),
			Vector3::new(0f32, 0f32, 1f32),
		);
	}

	fn build_input_manager_with_factory() -> (
		InputManager,
		crate::core::factory::Factory<Action>,
		DefaultListener<ActionEvent>,
	) {
		let action_factory = crate::core::factory::Factory::<Action>::new();
		let action_listener = action_factory.listener();
		let event_channel = DefaultChannel::new();
		let event_listener = event_channel.listener();
		let input_manager = InputManager::new(action_listener, event_channel);
		(input_manager, action_factory, event_listener)
	}

	fn count_events(listener: &mut DefaultListener<ActionEvent>) -> usize {
		let mut count = 0;
		while let Some(_) = Listener::read(listener) {
			count += 1;
		}
		count
	}

	#[test]
	fn test_tick_policy_on_change_only_emits_on_input() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		let device_class_handle = register_keyboard_device_class(&mut input_manager);
		let device = input_manager.create_device(&device_class_handle);
		let seat = SeatHandle::stub();

		let action = Action::new(
			"MoveForward",
			&[ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32))],
			Types::Float,
		)
		.tick_policy(TickPolicy::OnChange);
		factory.create(action);

		// First update with no input: drains factory, no records -> no events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);

		// Second update with no input: no events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);

		// Press key -> 1 event.
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// No new input -> no events (OnChange doesn't re-emit).
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);

		// Release key -> 1 event.
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// No new input -> no events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);
	}

	#[test]
	fn manual_action_is_queued_and_updates_synthetic_state() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		let seat = SeatHandle(7);
		let action = Action::new("Manual", &[], Types::Float);
		let event_handle = factory.create(action);
		update_input_manager(&mut input_manager);
		let action_handle = ActionHandle(0);

		input_manager.trigger_action(seat, action_handle, Value::Float(3.5)).unwrap();
		assert!(Listener::read(&mut event_listener).is_none());
		update_input_manager(&mut input_manager);

		let event = Listener::read(&mut event_listener).expect("expected manual action event");
		assert_eq!(event.seat_handle(), seat);
		assert_eq!(event.handle(), event_handle);
		assert_eq!(event.value(), Value::Float(3.5));
		assert_eq!(
			input_manager
				.get_action_state(seat, action_handle, InputManager::manual_action_device_handle())
				.value,
			Value::Float(3.5)
		);
	}

	#[test]
	fn manual_action_rejects_unknown_handles_and_wrong_values() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		factory.create(Action::new("Manual", &[], Types::Float));
		update_input_manager(&mut input_manager);

		assert!(matches!(
			input_manager.trigger_action(SeatHandle::stub(), ActionHandle(99), Value::Float(1.0)),
			Err(InputActionError::UnknownAction(ActionHandle(99)))
		));
		assert!(matches!(
			input_manager.trigger_action(SeatHandle::stub(), ActionHandle(0), Value::Bool(true)),
			Err(InputActionError::TypeMismatch {
				expected: Types::Float,
				actual: Types::Boolean
			})
		));
		update_input_manager(&mut input_manager);
		assert!(Listener::read(&mut event_listener).is_none());
	}

	#[test]
	fn manual_actions_preserve_queue_order() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		factory.create(Action::new("Manual", &[], Types::Int));
		update_input_manager(&mut input_manager);

		input_manager
			.trigger_action(SeatHandle::stub(), ActionHandle(0), Value::Int(1))
			.unwrap();
		input_manager
			.trigger_action(SeatHandle::stub(), ActionHandle(0), Value::Int(2))
			.unwrap();
		update_input_manager(&mut input_manager);

		assert_eq!(Listener::read(&mut event_listener).unwrap().value(), Value::Int(1));
		assert_eq!(Listener::read(&mut event_listener).unwrap().value(), Value::Int(2));
	}

	#[test]
	fn test_tick_policy_while_active_emits_while_non_default() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		let device_class_handle = register_keyboard_device_class(&mut input_manager);
		let device = input_manager.create_device(&device_class_handle);
		let seat = SeatHandle::stub();

		let action = Action::new(
			"MoveForward",
			&[ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32))],
			Types::Float,
		)
		.tick_policy(TickPolicy::WhileActive);
		factory.create(action);

		// First update registers the action, no records -> no events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);

		// Press key -> Phase A emits 1 event + Phase B sees value is non-default and emits 1 = 2.
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());
		update_input_manager(&mut input_manager);
		let events = count_events(&mut event_listener);
		assert!(events >= 1, "Expected at least 1 event on key press, got {}", events);

		// No new input, key still held -> WhileActive re-emits because value is non-default.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// Still held -> re-emits again.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// Release key.
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());
		update_input_manager(&mut input_manager);
		// Phase A emits change event, Phase B sees value is default so does not re-emit.
		// The value is now 0.0 (default), so WhileActive should not emit in Phase B.
		let events_on_release = count_events(&mut event_listener);
		assert!(
			events_on_release >= 1,
			"Expected at least 1 event on key release, got {}",
			events_on_release
		);

		// No new input, value is default -> no events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);
	}

	#[test]
	fn test_tick_policy_always_emits_every_frame() {
		let (mut input_manager, mut factory, mut event_listener) = build_input_manager_with_factory();
		let device_class_handle = register_keyboard_device_class(&mut input_manager);
		let device = input_manager.create_device(&device_class_handle);
		let seat = SeatHandle::stub();

		let action = Action::new(
			"MoveForward",
			&[ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32))],
			Types::Float,
		)
		.tick_policy(TickPolicy::Always);
		factory.create(action);

		// Registers the action, no device has interacted yet -> no Always events.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 0);

		// Press key -> events emitted.
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());
		update_input_manager(&mut input_manager);
		let events = count_events(&mut event_listener);
		assert!(events >= 1, "Expected at least 1 event on key press, got {}", events);

		// No new input -> Always still emits.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// Release key -> Still emits (Always emits regardless of value).
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());
		update_input_manager(&mut input_manager);
		let events = count_events(&mut event_listener);
		assert!(events >= 1);

		// No new input, value is default -> Always STILL emits.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);

		// And again.
		update_input_manager(&mut input_manager);
		assert_eq!(count_events(&mut event_listener), 1);
	}
}

use std::{collections::HashMap, default};

use log::warn;
use math::{Base, Vector2, Vector3};
use serde::de;
use utils::{insert_return_length, RGBA};

pub use super::action_evaluator::InputEventState;
use super::{
	action::{InputValue, TriggerMapping},
	action_evaluator::{resolve_action_value, InputAction},
	device::Device,
	device_class::{DeviceClass, DeviceClassHandle},
	input_trigger::{Trigger, TriggerDescription},
	records::{compact_latest_by_source, compare_source_then_time, Record},
	Action, ActionBindingDescription, ActionHandle, DeviceHandle, Function, SeatHandle, TickPolicy, TriggerHandle, Types,
	Value,
};
use crate::{
	core::{
		channel::{Channel as _, DefaultChannel},
		factory::{CreateMessage, Factory},
		listener::{DefaultListener, Listener},
		message::Message,
		Entity, EntityHandle,
	},
	input::ActionEvent,
};
