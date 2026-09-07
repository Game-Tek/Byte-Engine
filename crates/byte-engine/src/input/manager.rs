//! The convenience input path for applications with a single input consumer.
//!
//! [`InputManager`] wires the two input steps together and broadcasts every
//! action: it owns one [`InputEvents`] queue and one [`ActionProcessor`], and
//! consumes nothing. Use it for applications where no two contexts compete for
//! the same control, and for the standard headed integration through
//! `GraphicsApplication`.
//!
//! Applications that need layered input, such as a UI that consumes clicks
//! before gameplay sees them, own an [`InputEvents`] and one
//! [`ActionProcessor`] per layer instead. Choose one of the two: this manager
//! broadcasts the whole queue, so a layer added beside it cannot consume
//! anything from it.
//!
//! See [Input](/docs/reference/input) for both workflows.

/// The [`InputManager`] struct broadcasts every action a device produces.
///
/// Register device classes and triggers through [`TriggerRegistry`], create
/// devices, then record platform values with
/// [`Self::record_trigger_value_for_device`] and call [`Self::update`] once per
/// application tick. For the standard headed integration, use
/// `process_default_window_input` rather than duplicating the mouse and
/// keyboard trigger-name mapping.
///
/// Subscribe through [`Self::event_channel`] to receive the resolved
/// [`ActionEvent`] values.
pub struct InputManager<A: Allocator + Clone = Global> {
	events: InputEvents<A>,
	processor: ActionProcessor<A>,
}

impl InputManager {
	/// Creates an input manager connected to action creation and event channels.
	pub fn new(action_listener: DefaultListener<CreateMessage<Action>>, event_channel: DefaultChannel<ActionEvent>) -> Self {
		Self::new_in(action_listener, event_channel, Global)
	}

	/// Returns the synthetic device used by [`Self::trigger_action`].
	pub fn manual_action_device_handle() -> DeviceHandle {
		MANUAL_ACTION_DEVICE
	}
}

impl<A: Allocator + Clone> InputManager<A> {
	/// Creates the queue and its action processor in `allocator`.
	///
	/// Keep arena storage alive until the manager is dropped. The supplied
	/// listener and channel retain their own allocation policies. Next, register
	/// controls through [`TriggerRegistry`] and call [`Self::create_device`].
	pub fn new_in(
		action_listener: DefaultListener<CreateMessage<Action>>,
		event_channel: DefaultChannel<ActionEvent>,
		allocator: A,
	) -> Self {
		let mut events = InputEvents::new_in(allocator.clone());
		let consumer = events.add_consumer();
		Self {
			events,
			processor: ActionProcessor::new_in(consumer, event_channel, allocator).with_declarations(action_listener),
		}
	}

	/// Returns the input step this manager records into.
	///
	/// Use it to read control values, and to translate platform events with
	/// `process_default_window_input`.
	pub fn events(&self) -> &InputEvents<A> {
		&self.events
	}

	/// Creates one concrete device from a registered class.
	///
	/// Call this once for each physical or virtual device, such as each connected
	/// gamepad.
	pub fn create_device(&mut self, device_class_handle: &DeviceClassHandle) -> DeviceHandle {
		self.events.create_device(device_class_handle)
	}

	/// Returns all devices that belong to the named class.
	pub fn get_devices_by_class_name(&self, class_name: &str) -> Option<impl Iterator<Item = DeviceHandle> + '_> {
		self.events.devices_by_class_name(class_name)
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
		self.events.record(seat_handle, device_handle, trigger_reference, value);
	}

	/// Resolves queued trigger and manual-action values, then emits action events.
	///
	/// Call this once per application tick after recording platform input. Next,
	/// drain a listener created from [`Self::event_channel`] to handle the resolved
	/// [`ActionEvent`] values.
	pub fn update(&mut self) {
		self.processor.broadcast(&self.events);
		self.events.end_tick();
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
		self.processor.trigger_action(seat_handle, action_handle, value)
	}

	/// Creates an action that emits when its resolved value changes.
	///
	/// Next, record values for one of the action's trigger mappings and call
	/// [`Self::update`]. Use [`Self::event_channel`] to receive the result.
	pub fn create_action(&mut self, r#type: Types, action_binding_descriptions: &[ActionBindingDescription]) -> ActionHandle {
		self.create_action_with_tick_policy(r#type, action_binding_descriptions, TickPolicy::OnChange)
	}

	/// Creates an action with a specific tick policy controlling how frequently events are emitted.
	///
	/// Next, create a listener from [`Self::event_channel`], record trigger values,
	/// and call [`Self::update`] once per tick.
	pub fn create_action_with_tick_policy(
		&mut self,
		r#type: Types,
		action_binding_descriptions: &[ActionBindingDescription],
		tick_policy: TickPolicy,
	) -> ActionHandle {
		self.processor
			.create_action(&self.events, r#type, action_binding_descriptions, tick_policy)
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
		self.events.value(seat_handle, device_handle, trigger_reference)
	}

	/// Returns the latest resolved action state for a seat and device.
	pub fn get_action_state(&self, seat_handle: SeatHandle, action_handle: ActionHandle, device_handle: DeviceHandle) -> Value {
		self.processor.action_state(seat_handle, action_handle, device_handle)
	}

	/// Returns the channel that publishes resolved action events.
	///
	/// Next, call [`DefaultChannel::listener`] and keep that listener with the
	/// application system that handles the action.
	pub fn event_channel(&self) -> &DefaultChannel<ActionEvent> {
		self.processor.event_channel()
	}
}

impl<A: Allocator + Clone> TriggerRegistry for InputManager<A> {
	fn register_device_class(&mut self, name: &str) -> DeviceClassHandle {
		self.events.register_device_class(name)
	}

	fn register_trigger<T: InputValue + Into<Value>>(
		&mut self,
		device_class: &DeviceClassHandle,
		name: &str,
		description: TriggerDescription<T>,
	) -> TriggerHandle {
		self.events.register_trigger(device_class, name, description)
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

impl Message for Value {}

#[cfg(test)]
mod tests {
	use std::{cell::RefCell, ops::DerefMut, rc::Rc, sync::Arc};

	use math::Quaternion;
	use utils::RGBA;

	use super::*;
	use crate::core::channel::Channel as _;
	use crate::core::factory::Factory;
	use crate::core::listener::Listener;
	use crate::input::ActionBindingDescription;
	use crate::input::{
		Axis2, Axis3, Function, ValueMapping,
		trigger::TriggerDescription,
		utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	};

	fn declare_vr_headset_input_device_class(input_manager: &mut InputManager) -> DeviceClassHandle {
		let device_class_handle = input_manager.register_device_class("Headset");

		let source_description = TriggerDescription::new(
			Axis3::new(0f32, 1.80f32, 0f32),
			Axis3::new(0f32, 0f32, 0f32),
			Axis3::min_value(),
			Axis3::max_value(),
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

	fn build_input_manager_with_device(
		register_device_class: fn(&mut InputManager) -> DeviceClassHandle,
	) -> (InputManager, DeviceHandle) {
		let mut input_manager = build_input_manager();
		let device_class = register_device_class(&mut input_manager);
		let device = input_manager.create_device(&device_class);
		(input_manager, device)
	}

	#[test]
	fn trigger_queries_reject_unknown_handles_and_malformed_paths() {
		let (input, device) = build_input_manager_with_device(register_keyboard_device_class);
		for reference in [
			TriggerReference::Handle(TriggerHandle(u32::MAX)),
			TriggerReference::Name(""),
			TriggerReference::Name("Keyboard"),
			TriggerReference::Name("Keyboard."),
			TriggerReference::Name("Keyboard.Unknown.Up"),
			TriggerReference::Name("Unknown.Up"),
		] {
			assert!(input.get_trigger_value_for_device(SeatHandle(0), device, reference).is_err());
		}
		assert_eq!(
			input.get_trigger_value_for_device(SeatHandle(0), device, TriggerReference::Name("Keyboard.Up")),
			Ok(Value::Bool(false))
		);
	}

	#[test]
	fn untriggered_actions_have_neutral_values_for_every_input_type() {
		let mut input = build_input_manager();
		for (kind, expected) in [
			(Types::Boolean, Value::Bool(false)),
			(Types::Unicode, Value::Unicode('\0')),
			(Types::Int, Value::Int(0)),
			(Types::Float, Value::Float(0.0)),
			(Types::Rgba, Value::Rgba(RGBA::new(0.0, 0.0, 0.0, 1.0))),
			(Types::Vector2, Value::Vector2(Axis2::zero())),
			(Types::Vector3, Value::Vector3(Axis3::zero())),
			(Types::Quaternion, Value::Quaternion(Quaternion::identity())),
		] {
			let action = input.create_action(kind, &[]);
			assert_eq!(
				input.get_action_state(SeatHandle(0), action, InputManager::manual_action_device_handle()),
				expected
			);
		}
	}

	#[test]
	fn device_queries_preserve_handles_across_classes_and_instances() {
		let mut input = build_input_manager();
		let keyboard = register_keyboard_device_class(&mut input);
		let mouse = register_mouse_device_class(&mut input);
		let first_keyboard = input.create_device(&keyboard);
		let first_mouse = input.create_device(&mouse);
		let second_keyboard = input.create_device(&keyboard);
		let second_mouse = input.create_device(&mouse);
		let third_keyboard = input.create_device(&keyboard);

		assert_eq!(
			input.get_devices_by_class_name("Keyboard").map(Iterator::collect::<Vec<_>>),
			Some(vec![first_keyboard, second_keyboard, third_keyboard])
		);
		assert_eq!(
			input.get_devices_by_class_name("Mouse").map(Iterator::collect::<Vec<_>>),
			Some(vec![first_mouse, second_mouse])
		);
		assert!(input.get_devices_by_class_name("Unknown").is_none());
	}

	#[test]
	fn test_boolean_source_input_overlap_action() {
		let mut input_manager = build_input_manager();

		let x = register_keyboard_device_class(&mut input_manager);

		let action = input_manager.create_action(
			Types::Float,
			&[
				ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32)),
				ActionBindingDescription::new("Keyboard.Down").mapped(ValueMapping::new(Function::Boolean, -1f32)),
			],
		);

		let device = input_manager.create_device(&x);
		let seat = SeatHandle::stub();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(0f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(-1f32));

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Down"), false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), Value::Float(0f32));
	}

	#[test]
	fn test_boolean_trigger_2d_action_binding_combination() {
		let action_chanel = DefaultChannel::new();
		let action_listener = action_chanel.listener();
		let event_channel = DefaultChannel::new();

		let mut input_manager = InputManager::new(action_listener, event_channel);

		let x = register_keyboard_device_class(&mut input_manager);

		let action = input_manager.create_action(
			Types::Vector2,
			&[
				ActionBindingDescription::new("Keyboard.Up")
					.mapped(ValueMapping::new(Function::Boolean, Axis2::new(0f32, 1f32))),
				ActionBindingDescription::new("Keyboard.Down")
					.mapped(ValueMapping::new(Function::Boolean, Axis2::new(0f32, -1f32))),
				ActionBindingDescription::new("Keyboard.Left")
					.mapped(ValueMapping::new(Function::Boolean, Axis2::new(-1f32, 0f32))),
				ActionBindingDescription::new("Keyboard.Right")
					.mapped(ValueMapping::new(Function::Boolean, Axis2::new(1f32, 0f32))),
			],
		);

		let device = input_manager.create_device(&x);
		let seat = SeatHandle::stub();

		assert_eq!(
			input_manager.get_action_state(seat, action, device),
			Value::Vector2(Axis2::new(0f32, 0f32))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), true.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), true.into());

		input_manager.update();

		assert_eq!(
			input_manager.get_action_state(seat, action, device),
			Value::Vector2(Axis2::new(1f32 / 2f32.sqrt(), 1f32 / 2f32.sqrt()))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Up"), false.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), false.into());

		input_manager.update();

		assert_eq!(
			input_manager.get_action_state(seat, action, device),
			Value::Vector2(Axis2::new(0f32, 0f32))
		);

		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Left"), true.into());
		input_manager.record_trigger_value_for_device(seat, device, TriggerReference::Name("Keyboard.Right"), true.into());

		input_manager.update();

		assert_eq!(
			input_manager.get_action_state(seat, action, device),
			Value::Vector2(Axis2::new(0f32, 0f32))
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
				.expect("expected test value"),
			a
		); // Assert default value

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, b); // Record alternate value.

		input_manager.update();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.expect("expected test value"),
			b
		); // Assert alternate value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value.

		input_manager.update();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.expect("expected test value"),
			a
		); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value again.

		input_manager.update();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.expect("expected test value"),
			a
		); // Assert default value after recording.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, a); // Record default value.
		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, b); // Record alternate value after recording default value.

		input_manager.update();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.expect("expected test value"),
			b
		); // Assert value is last value recorded.

		input_manager.record_trigger_value_for_device(seat, device, trigger_reference, z); // Record a different type.

		input_manager.update();

		assert_eq!(
			input_manager
				.get_trigger_value_for_device(seat, device, trigger_reference)
				.expect("expected test value"),
			b
		);
		// Assert last value is kept after recording a different type.
	}

	#[test]
	fn record_bool_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(register_keyboard_device_class);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, false, true, 961f32);
	}

	#[test]
	fn record_unicode_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(register_keyboard_device_class);

		let handle = TriggerReference::Name("Keyboard.Character");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, '\0', 'a', true);
	}

	#[test]
	fn unicode_action_emits_character_events() {
		let mut fixture = InputFixture::with_keyboard();
		let handle = fixture.factory.create(Action::new(
			&[ActionBindingDescription::new("Keyboard.Character")],
			Types::Unicode,
		));
		fixture.update();

		fixture.input_manager.record_trigger_value_for_device(
			fixture.seat,
			fixture.device.expect("keyboard fixture should have a device"),
			TriggerReference::Name("Keyboard.Character"),
			'é'.into(),
		);
		fixture.update();

		let event = fixture.next_event().expect("expected character action event");

		assert_eq!(event.handle(), handle);
		assert_eq!(event.value(), Value::Unicode('é'));
		assert!(fixture.next_event().is_none());
	}

	#[test]
	fn record_int_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(declare_funky_input_device_class);

		let handle = TriggerReference::Name("Funky.Int");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0, 1, true);
	}

	#[test]
	fn record_float_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(register_gamepad_device_class);

		let handle = TriggerReference::Name("Gamepad.LeftTrigger");

		record_and_assert_input_source_action_sequence(&mut input_manager, device, handle, 0.0f32, 1f32, true);
	}

	#[test]
	fn record_vector2_input_source_action() {
		let (mut input_manager, device) = build_input_manager_with_device(register_gamepad_device_class);

		let handle = TriggerReference::Name("Gamepad.LeftStick");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			Axis2 { x: 0f32, y: 0f32 },
			Axis2 { x: 1f32, y: 1f32 },
			true,
		);
	}

	#[test]
	fn record_vector3_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(declare_vr_headset_input_device_class);

		let handle = TriggerReference::Name("Headset.Position");

		record_and_assert_input_source_action_sequence(
			&mut input_manager,
			device,
			handle,
			Axis3 {
				x: 0f32,
				y: 1.8f32,
				z: 0f32,
			},
			Axis3 {
				x: 1f32,
				y: 1f32,
				z: 1f32,
			},
			true,
		);
	}

	#[test]
	fn record_quaternion_input_source_actions() {
		let (mut input_manager, device) = build_input_manager_with_device(declare_vr_headset_input_device_class);

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
		let (mut input_manager, device) = build_input_manager_with_device(declare_funky_input_device_class);

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
		input_source_name: &'static str,
		a: T,
		b: T,
	) where
		T: InputValue + Into<Value> + Into<ValueMapping> + Copy,
	{
		let action = input_manager.create_action(
			T::get_type(),
			&[ActionBindingDescription::new(input_source_name).mapped(b.into())],
		);
		let seat = SeatHandle::stub();

		assert_eq!(input_manager.get_action_state(seat, action, device), a.into());

		input_manager.record_trigger_value_for_device(seat, device, handle, true.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), b.into());

		input_manager.record_trigger_value_for_device(seat, device, handle, false.into());

		input_manager.update();

		assert_eq!(input_manager.get_action_state(seat, action, device), a.into());
	}

	#[test]
	fn test_boolean_float_interpolation() {
		let (mut input_manager, device) = build_input_manager_with_device(register_keyboard_device_class);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"Keyboard.Up",
			0f32,
			1f32,
		);
	}

	#[test]
	fn test_boolean_vector2_interpolation() {
		let (mut input_manager, device) = build_input_manager_with_device(register_keyboard_device_class);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"Keyboard.Up",
			Axis2::zero(),
			Axis2::new(0f32, 1f32),
		);
	}

	#[test]
	fn test_boolean_vector3_interpolation() {
		let (mut input_manager, device) = build_input_manager_with_device(register_keyboard_device_class);

		let handle = TriggerReference::Name("Keyboard.Up");

		record_and_assert_boolean_input_source_action_interpolation(
			&mut input_manager,
			device,
			handle,
			"Keyboard.Up",
			Axis3::zero(),
			Axis3::new(0f32, 0f32, 1f32),
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
		while Listener::read(listener).is_some() {
			count += 1;
		}
		count
	}

	/// The `InputFixture` struct keeps input tests focused on actions, transitions, and emitted events.
	struct InputFixture {
		input_manager: InputManager,
		factory: crate::core::factory::Factory<Action>,
		event_listener: DefaultListener<ActionEvent>,
		seat: SeatHandle,
		device: Option<DeviceHandle>,
	}

	impl InputFixture {
		fn new() -> Self {
			let (input_manager, factory, event_listener) = build_input_manager_with_factory();
			Self {
				input_manager,
				factory,
				event_listener,
				seat: SeatHandle::stub(),
				device: None,
			}
		}

		fn with_keyboard() -> Self {
			let mut fixture = Self::new();
			let device_class = register_keyboard_device_class(&mut fixture.input_manager);
			fixture.device = Some(fixture.input_manager.create_device(&device_class));
			fixture
		}

		fn register_tick_action(&mut self, policy: TickPolicy) {
			let action = Action::new(
				&[ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32))],
				Types::Float,
			)
			.tick_policy(policy);
			self.factory.create(action);
		}

		fn set_up_key(&mut self, pressed: bool) {
			self.input_manager.record_trigger_value_for_device(
				self.seat,
				self.device.expect("keyboard fixture should have a device"),
				TriggerReference::Name("Keyboard.Up"),
				pressed.into(),
			);
		}

		fn update(&mut self) {
			self.input_manager.update();
		}

		fn tick(&mut self) -> usize {
			self.update();
			count_events(&mut self.event_listener)
		}

		fn next_event(&mut self) -> Option<ActionEvent> {
			Listener::read(&mut self.event_listener)
		}
	}

	#[test]
	fn triggered_binding_snapshots_each_press_in_queue_order() {
		let mut fixture = InputFixture::new();
		let class = crate::input::utils::register_mouse_device_class(&mut fixture.input_manager);
		let mouse = fixture.input_manager.create_device(&class);
		let other_mouse = fixture.input_manager.create_device(&class);
		fixture.factory.create(
			Action::new(
				&[ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton")],
				Types::Vector2,
			)
			.tick_policy(TickPolicy::Always),
		);
		// Neither a missing sample nor another device's position supplies a snapshot.
		for (device, source, value) in [
			(other_mouse, "Mouse.Position", Value::Vector2(Axis2::new(99.0, 99.0))),
			(mouse, "Mouse.LeftButton", Value::Bool(true)),
		] {
			fixture
				.input_manager
				.record_trigger_value_for_device(fixture.seat, device, TriggerReference::Name(source), value);
		}
		assert_eq!(fixture.tick(), 0);
		for (source, value) in [
			("Mouse.Position", Value::Vector2(Axis2::new(1.0, 2.0))),
			("Mouse.LeftButton", Value::Bool(true)),
			("Mouse.LeftButton", Value::Bool(false)),
			("Mouse.Position", Value::Vector2(Axis2::new(3.0, 4.0))),
			("Mouse.LeftButton", Value::Bool(true)),
			("Mouse.Position", Value::Vector2(Axis2::new(5.0, 6.0))),
			("Mouse.LeftButton", Value::Bool(false)),
		] {
			fixture
				.input_manager
				.record_trigger_value_for_device(fixture.seat, mouse, TriggerReference::Name(source), value);
		}
		fixture.update();
		assert_eq!(fixture.next_event().unwrap().value(), Value::Vector2(Axis2::new(1.0, 2.0)));
		assert_eq!(fixture.next_event().unwrap().value(), Value::Vector2(Axis2::new(3.0, 4.0)));
		assert!(fixture.next_event().is_none());
		assert_eq!(fixture.tick(), 0);
		// A later press samples the retained position without requiring more motion.
		fixture.input_manager.record_trigger_value_for_device(
			fixture.seat,
			mouse,
			TriggerReference::Name("Mouse.LeftButton"),
			Value::Bool(true),
		);
		fixture.update();
		assert_eq!(fixture.next_event().unwrap().value(), Value::Vector2(Axis2::new(5.0, 6.0)));
		assert!(fixture.next_event().is_none());
		// Motion alone stays silent, and another seat cannot sample this seat's value.
		fixture.input_manager.record_trigger_value_for_device(
			fixture.seat,
			mouse,
			TriggerReference::Name("Mouse.Position"),
			Value::Vector2(Axis2::new(7.0, 8.0)),
		);
		fixture.input_manager.record_trigger_value_for_device(
			SeatHandle(42),
			mouse,
			TriggerReference::Name("Mouse.LeftButton"),
			Value::Bool(true),
		);
		assert_eq!(fixture.tick(), 0);
	}

	#[test]
	fn test_tick_policy_on_change_only_emits_on_input() {
		let mut fixture = InputFixture::with_keyboard();
		fixture.register_tick_action(TickPolicy::OnChange);

		assert_eq!(fixture.tick(), 0);
		assert_eq!(fixture.tick(), 0);

		fixture.set_up_key(true);

		assert_eq!(fixture.tick(), 1);
		assert_eq!(fixture.tick(), 0);

		fixture.set_up_key(false);

		assert_eq!(fixture.tick(), 1);
		assert_eq!(fixture.tick(), 0);
	}

	#[test]
	fn manual_action_is_queued_and_updates_synthetic_state() {
		let mut fixture = InputFixture::new();
		fixture.seat = SeatHandle(7);
		let event_handle = fixture.factory.create(Action::new(&[], Types::Float));
		fixture.update();
		let action_handle = ActionHandle(0);

		fixture
			.input_manager
			.trigger_action(fixture.seat, action_handle, Value::Float(3.5))
			.expect("registered manual action should accept a float value");

		assert!(fixture.next_event().is_none());
		fixture.update();

		let event = fixture.next_event().expect("expected manual action event");

		assert_eq!(event.seat_handle(), fixture.seat);
		assert_eq!(event.handle(), event_handle);
		assert_eq!(event.value(), Value::Float(3.5));
		// The synthetic value remains available after another update.
		fixture.update();
		assert_eq!(
			fixture
				.input_manager
				.get_action_state(fixture.seat, action_handle, InputManager::manual_action_device_handle()),
			Value::Float(3.5)
		);
	}

	#[test]
	fn manual_action_rejects_unknown_handles_and_wrong_values() {
		let mut fixture = InputFixture::new();
		fixture.factory.create(Action::new(&[], Types::Float));
		fixture.update();

		assert!(matches!(
			fixture
				.input_manager
				.trigger_action(fixture.seat, ActionHandle(99), Value::Float(1.0)),
			Err(InputActionError::UnknownAction(ActionHandle(99)))
		));
		assert!(matches!(
			fixture
				.input_manager
				.trigger_action(fixture.seat, ActionHandle(0), Value::Bool(true)),
			Err(InputActionError::TypeMismatch {
				expected: Types::Float,
				actual: Types::Boolean
			})
		));
		fixture.update();

		assert!(fixture.next_event().is_none());
	}

	#[test]
	fn manual_actions_preserve_queue_order() {
		let mut fixture = InputFixture::new();
		fixture.factory.create(Action::new(&[], Types::Int));
		fixture.update();

		fixture
			.input_manager
			.trigger_action(fixture.seat, ActionHandle(0), Value::Int(1))
			.expect("registered manual action should accept the first integer");
		fixture
			.input_manager
			.trigger_action(fixture.seat, ActionHandle(0), Value::Int(2))
			.expect("registered manual action should accept the second integer");
		fixture.update();

		assert_eq!(
			fixture.next_event().expect("first queued event should exist").value(),
			Value::Int(1)
		);
		assert_eq!(
			fixture.next_event().expect("second queued event should exist").value(),
			Value::Int(2)
		);
	}

	#[test]
	fn test_tick_policy_while_active_emits_while_non_default() {
		let mut fixture = InputFixture::with_keyboard();
		fixture.register_tick_action(TickPolicy::WhileActive);

		assert_eq!(fixture.tick(), 0);

		fixture.set_up_key(true);

		assert!(fixture.tick() >= 1);
		assert_eq!(fixture.tick(), 1);
		assert_eq!(fixture.tick(), 1);

		fixture.set_up_key(false);

		assert!(fixture.tick() >= 1);
		assert_eq!(fixture.tick(), 0);
	}

	#[test]
	fn test_tick_policy_always_emits_every_frame() {
		let mut fixture = InputFixture::with_keyboard();
		fixture.register_tick_action(TickPolicy::Always);

		assert_eq!(fixture.tick(), 0);

		fixture.set_up_key(true);

		assert!(fixture.tick() >= 1);
		assert_eq!(fixture.tick(), 1);

		fixture.set_up_key(false);

		assert!(fixture.tick() >= 1);
		assert_eq!(fixture.tick(), 1);
		assert_eq!(fixture.tick(), 1);
	}
}

use std::alloc::{Allocator, Global};

use super::events::InputEvents;
use super::processor::{ActionProcessor, MANUAL_ACTION_DEVICE};
use super::trigger::{TriggerDescription, TriggerHandle, TriggerReference, TriggerRegistry};
use super::{
	Action, ActionBindingDescription, ActionHandle, DeviceHandle, SeatHandle, TickPolicy, Types, Value, action::InputValue,
	device::DeviceClassHandle,
};
use crate::{
	core::{channel::DefaultChannel, factory::CreateMessage, listener::DefaultListener, message::Message},
	input::ActionEvent,
};
