//! Input behavior, exercised end to end through the collector and sinks.

use math::Quaternion;
use utils::RGBA;

use crate::{
	core::{
		channel::{Channel as _, DefaultChannel},
		factory::{Factory, Handle},
		listener::{DefaultListener, Listener as _},
	},
	input::{
		Action, ActionBindingDescription, ActionEvent, ActionHandle, ActionPhase, Axis2, Axis3, Capture, DeviceHandle,
		Function, InputCollector, InputSink, ResolvedAction, SeatHandle, TickPolicy, TriggerMode, TriggerReference,
		TriggerRegistry, Types, Value, ValueMapping,
		action::InputValue,
		device::DeviceClassHandle,
		trigger::TriggerDescription,
		utils::{register_gamepad_device_class, register_keyboard_device_class, register_mouse_device_class},
	},
};

/// The `Fixture` struct runs sinks over one shared collector with a mouse and a keyboard.
struct Fixture {
	collector: InputCollector,
	mouse: DeviceHandle,
	mouse_class: DeviceClassHandle,
	keyboard: DeviceHandle,
	keyboard_class: DeviceClassHandle,
}

/// The `Sink` struct is one consumer: its action factory, sink, and published events.
struct Sink {
	actions: Factory<Action>,
	sink: InputSink,
	events: DefaultListener<ActionEvent>,
}

impl Fixture {
	fn new() -> Self {
		let mut collector = InputCollector::new();
		let mouse_class = register_mouse_device_class(&mut collector);
		let keyboard_class = register_keyboard_device_class(&mut collector);
		let mouse = collector.create_device(&mouse_class);
		let keyboard = collector.create_device(&keyboard_class);

		Self {
			collector,
			mouse,
			mouse_class,
			keyboard,
			keyboard_class,
		}
	}

	/// Adds one sink, in the order the application will pull them.
	fn sink(&mut self) -> Sink {
		let actions = Factory::new();
		let channel = DefaultChannel::new();
		let events = channel.listener();
		let sink = InputSink::new(self.collector.add_sink(), channel).with_declarations(actions.listener());

		Sink { actions, sink, events }
	}

	fn record(&mut self, name: &'static str, value: impl Into<Value>) {
		let device = if name.starts_with("Mouse.") {
			self.mouse
		} else {
			self.keyboard
		};
		self.collector
			.record(SeatHandle::stub(), device, TriggerReference::Name(name), value.into());
	}

	fn value(&self, name: &'static str) -> Value {
		let device = if name.starts_with("Mouse.") {
			self.mouse
		} else {
			self.keyboard
		};
		self.collector
			.value(SeatHandle::stub(), device, TriggerReference::Name(name))
			.expect("registered trigger")
	}

	/// Pulls one sink and returns the actions it resolved this tick.
	///
	/// `capture` decides which actions the consumer handles, as an application's
	/// hit-testing or focus logic would.
	fn pull(&mut self, sink: &mut Sink, capture: impl Fn(&ResolvedAction) -> bool) -> Vec<ResolvedAction> {
		let mut resolved = Vec::new();
		sink.sink.pull(&mut self.collector, |action| {
			resolved.push(*action);
			Capture::when(capture(action))
		});

		resolved
	}
}

impl Sink {
	fn button(&mut self, trigger: &'static str, policy: TickPolicy) -> Handle {
		self.actions
			.create(Action::new(&[ActionBindingDescription::new(trigger)], Types::Boolean).tick_policy(policy))
	}

	fn published(&mut self) -> Vec<ActionEvent> {
		self.events.to_vec()
	}

	/// Pulls with nothing captured and returns how many events were published.
	fn count(&mut self, collector: &mut InputCollector) -> usize {
		self.sink.pull(collector, |_| Capture::Passed);
		self.published().len()
	}
}

fn pass(_: &ResolvedAction) -> bool {
	false
}

fn handles(action: Handle) -> impl Fn(&ResolvedAction) -> bool {
	move |resolved: &ResolvedAction| resolved.handle == Some(action)
}

fn register_headset(collector: &mut InputCollector) -> DeviceClassHandle {
	let class = collector.register_device_class("Headset");
	collector.register_trigger(
		&class,
		"Position",
		TriggerDescription::new(
			Axis3::new(0.0, 1.8, 0.0),
			Axis3::zero(),
			Axis3::min_value(),
			Axis3::max_value(),
		),
	);
	collector.register_trigger(&class, "Orientation", TriggerDescription::<Quaternion>::default());
	class
}

fn register_funky(collector: &mut InputCollector) -> DeviceClassHandle {
	let class = collector.register_device_class("Funky");
	collector.register_trigger(&class, "Int", TriggerDescription::new(0, 0, 0, 3));
	collector.register_trigger(
		&class,
		"Rgba",
		TriggerDescription::new(
			RGBA::new(0.0, 0.0, 0.0, 0.0),
			RGBA::new(0.0, 0.0, 0.0, 0.0),
			RGBA::new(0.0, 0.0, 0.0, 0.0),
			RGBA::new(1.0, 1.0, 1.0, 1.0),
		),
	);
	class
}

#[test]
fn trigger_queries_reject_unknown_handles_and_malformed_paths() {
	let fixture = Fixture::new();
	for reference in [
		TriggerReference::Handle(crate::input::TriggerHandle(u32::MAX)),
		TriggerReference::Name(""),
		TriggerReference::Name("Keyboard"),
		TriggerReference::Name("Keyboard."),
		TriggerReference::Name("Keyboard.Unknown.Up"),
		TriggerReference::Name("Unknown.Up"),
	] {
		assert!(
			fixture
				.collector
				.value(SeatHandle::stub(), fixture.keyboard, reference)
				.is_err()
		);
		assert!(fixture.collector.trigger(reference).is_none());
	}
	assert_eq!(fixture.value("Keyboard.Up"), Value::Bool(false));
}

#[test]
fn device_queries_preserve_handles_across_classes_and_instances() {
	let mut fixture = Fixture::new();
	let second_keyboard = fixture.collector.create_device(&fixture.keyboard_class);
	let second_mouse = fixture.collector.create_device(&fixture.mouse_class);
	let third_keyboard = fixture.collector.create_device(&fixture.keyboard_class);

	assert_eq!(
		fixture
			.collector
			.devices_by_class_name("Keyboard")
			.map(Iterator::collect::<Vec<_>>),
		Some(vec![fixture.keyboard, second_keyboard, third_keyboard])
	);
	assert_eq!(
		fixture
			.collector
			.devices_by_class_name("Mouse")
			.map(Iterator::collect::<Vec<_>>),
		Some(vec![fixture.mouse, second_mouse])
	);
	assert!(fixture.collector.devices_by_class_name("Unknown").is_none());
}

#[test]
fn every_trigger_type_retains_its_last_value_and_rejects_other_types() {
	let mut fixture = Fixture::new();
	let headset_class = register_headset(&mut fixture.collector);
	let funky_class = register_funky(&mut fixture.collector);
	let gamepad_class = register_gamepad_device_class(&mut fixture.collector);
	let headset = fixture.collector.create_device(&headset_class);
	let funky = fixture.collector.create_device(&funky_class);
	let gamepad = fixture.collector.create_device(&gamepad_class);
	let seat = SeatHandle::stub();
	let cases: [(DeviceHandle, &str, Value, Value, Value); 8] = [
		(fixture.keyboard, "Keyboard.Up", false.into(), true.into(), 961f32.into()),
		(fixture.keyboard, "Keyboard.Character", '\0'.into(), 'a'.into(), true.into()),
		(funky, "Funky.Int", 0.into(), 1.into(), true.into()),
		(gamepad, "Gamepad.LeftTrigger", 0f32.into(), 1f32.into(), true.into()),
		(
			gamepad,
			"Gamepad.LeftStick",
			Axis2::zero().into(),
			Axis2::new(1.0, 1.0).into(),
			true.into(),
		),
		(
			headset,
			"Headset.Position",
			Axis3::new(0.0, 1.8, 0.0).into(),
			Axis3::new(1.0, 1.0, 1.0).into(),
			true.into(),
		),
		(
			headset,
			"Headset.Orientation",
			Quaternion::from_euler_angles(0.0, 0.0, 0.0).into(),
			Quaternion::from_euler_angles(1.0, 1.0, 1.0).into(),
			true.into(),
		),
		(
			funky,
			"Funky.Rgba",
			RGBA::new(0.0, 0.0, 0.0, 0.0).into(),
			RGBA::new(1.0, 1.0, 1.0, 1.0).into(),
			true.into(),
		),
	];
	for (device, name, default, alternate, other_type) in cases {
		let reference = TriggerReference::Name(name);
		let value = |fixture: &Fixture| fixture.collector.value(seat, device, reference).unwrap();
		assert_eq!(value(&fixture), default, "{name} default");
		for (records, expected) in [
			(vec![alternate], alternate),
			(vec![default], default),
			(vec![default, alternate], alternate),
			(vec![other_type], alternate),
		] {
			for record in records {
				fixture.collector.record(seat, device, reference, record);
			}
			assert_eq!(value(&fixture), expected, "{name}");
		}
	}
}

#[test]
fn untriggered_actions_have_neutral_values_for_every_input_type() {
	let mut fixture = Fixture::new();
	let mut sink = InputSink::new(fixture.collector.add_sink(), DefaultChannel::new());
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
		let action = sink.create_action(&fixture.collector, kind, &[], TickPolicy::OnChange);
		assert_eq!(sink.action_state(SeatHandle::stub(), action, fixture.mouse), expected);
	}
}

#[test]
fn declared_actions_reach_the_sink_with_their_entity_handle() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	let zoom = sink
		.actions
		.create(Action::new(&[ActionBindingDescription::new("Mouse.Scroll")], Types::Float));
	fixture.pull(&mut sink, pass);
	fixture.record("Mouse.Scroll", 1.0f32);
	let resolved = fixture.pull(&mut sink, pass);
	assert_eq!(resolved[0].handle, Some(zoom));
	let published = sink.published();
	assert_eq!(published.len(), 1);
	assert_eq!(published[0].handle(), zoom);
	assert_eq!(published[0].value(), Value::Float(1.0));
	assert_eq!(published[0].phase(), ActionPhase::Updated);
}

#[test]
fn opposing_scalar_bindings_follow_the_most_recent_press() {
	let mut fixture = Fixture::new();
	let mut sink = InputSink::new(fixture.collector.add_sink(), DefaultChannel::new());
	let action = sink.create_action(
		&fixture.collector,
		Types::Float,
		&[
			ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32)),
			ActionBindingDescription::new("Keyboard.Down").mapped(ValueMapping::new(Function::Boolean, -1f32)),
		],
		TickPolicy::OnChange,
	);
	let seat = SeatHandle::stub();
	assert_eq!(sink.action_state(seat, action, fixture.keyboard), Value::Float(0.0));
	for (name, pressed, expected) in [
		("Keyboard.Up", true, 1.0),
		("Keyboard.Up", false, 0.0),
		("Keyboard.Up", true, 1.0),
		("Keyboard.Down", true, -1.0),
		("Keyboard.Down", false, 1.0),
		("Keyboard.Up", false, 0.0),
		("Keyboard.Up", true, 1.0),
		("Keyboard.Down", true, -1.0),
		("Keyboard.Up", false, -1.0),
		("Keyboard.Down", false, 0.0),
	] {
		fixture.record(name, pressed);
		sink.pull(&mut fixture.collector, |_| Capture::Passed);
		assert_eq!(sink.action_state(seat, action, fixture.keyboard), Value::Float(expected));
	}
}

#[test]
fn directional_bindings_sum_and_normalize_the_active_keys() {
	let mut fixture = Fixture::new();
	let mut sink = InputSink::new(fixture.collector.add_sink(), DefaultChannel::new());
	let action = sink.create_action(
		&fixture.collector,
		Types::Vector2,
		&[
			ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, Axis2::new(0.0, 1.0))),
			ActionBindingDescription::new("Keyboard.Down").mapped(ValueMapping::new(Function::Boolean, Axis2::new(0.0, -1.0))),
			ActionBindingDescription::new("Keyboard.Left").mapped(ValueMapping::new(Function::Boolean, Axis2::new(-1.0, 0.0))),
			ActionBindingDescription::new("Keyboard.Right").mapped(ValueMapping::new(Function::Boolean, Axis2::new(1.0, 0.0))),
		],
		TickPolicy::OnChange,
	);
	let seat = SeatHandle::stub();
	let diagonal = 1.0 / 2f32.sqrt();
	for (records, expected) in [
		(
			[("Keyboard.Up", true), ("Keyboard.Right", true)],
			Axis2::new(diagonal, diagonal),
		),
		([("Keyboard.Up", false), ("Keyboard.Right", false)], Axis2::zero()),
		([("Keyboard.Left", true), ("Keyboard.Right", true)], Axis2::zero()),
	] {
		for (name, pressed) in records {
			fixture.record(name, pressed);
		}
		sink.pull(&mut fixture.collector, |_| Capture::Passed);
		assert_eq!(sink.action_state(seat, action, fixture.keyboard), Value::Vector2(expected));
	}
}

fn assert_boolean_maps_to<T>(neutral: T, active: T)
where
	T: InputValue + Into<Value> + Into<ValueMapping> + Copy,
{
	let mut fixture = Fixture::new();
	let mut sink = InputSink::new(fixture.collector.add_sink(), DefaultChannel::new());
	let action = sink.create_action(
		&fixture.collector,
		T::get_type(),
		&[ActionBindingDescription::new("Keyboard.Up").mapped(active.into())],
		TickPolicy::OnChange,
	);
	let seat = SeatHandle::stub();
	assert_eq!(sink.action_state(seat, action, fixture.keyboard), neutral.into());
	for (pressed, expected) in [(true, active), (false, neutral)] {
		fixture.record("Keyboard.Up", pressed);
		sink.pull(&mut fixture.collector, |_| Capture::Passed);
		assert_eq!(sink.action_state(seat, action, fixture.keyboard), expected.into());
	}
}

#[test]
fn a_boolean_binding_maps_to_scalar_and_vector_actions() {
	assert_boolean_maps_to(0f32, 1f32);
	assert_boolean_maps_to(Axis2::zero(), Axis2::new(0.0, 1.0));
	assert_boolean_maps_to(Axis3::zero(), Axis3::new(0.0, 0.0, 1.0));
}

#[test]
fn unicode_actions_publish_each_character() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	let typing = sink.actions.create(Action::new(
		&[ActionBindingDescription::new("Keyboard.Character")],
		Types::Unicode,
	));
	fixture.record("Keyboard.Character", 'é');
	fixture.pull(&mut sink, pass);
	let published = sink.published();
	assert_eq!(published.len(), 1);
	assert_eq!(published[0].handle(), typing);
	assert_eq!(published[0].value(), Value::Unicode('é'));
}

#[test]
fn tick_policies_decide_how_held_values_repeat() {
	for (policy, expected) in [
		(TickPolicy::OnChange, [0, 0, 1, 0, 0, 1, 0, 0]),
		(TickPolicy::WhileActive, [0, 0, 1, 1, 1, 1, 0, 0]),
		(TickPolicy::Always, [0, 0, 1, 1, 1, 1, 1, 1]),
	] {
		let mut fixture = Fixture::new();
		let mut sink = fixture.sink();
		sink.actions.create(
			Action::new(
				&[ActionBindingDescription::new("Keyboard.Up").mapped(ValueMapping::new(Function::Boolean, 1f32))],
				Types::Float,
			)
			.tick_policy(policy),
		);
		let mut counts = [0; 8];
		for (tick, count) in counts.iter_mut().enumerate() {
			if tick == 2 {
				fixture.record("Keyboard.Up", true);
			}
			if tick == 5 {
				fixture.record("Keyboard.Up", false);
			}
			*count = sink.count(&mut fixture.collector);
		}
		assert_eq!(counts, expected, "{policy:?}");
	}
}

#[test]
fn source_events_expose_every_queued_record_until_every_sink_pulled() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	ui.button("Mouse.LeftButton", TickPolicy::OnChange);
	fixture.record("Mouse.LeftButton", true);
	fixture.record("Mouse.Position", Axis2::new(0.5, 0.5));
	let events: Vec<_> = fixture.collector.source_events().collect();
	assert_eq!(events.len(), 2);
	assert_eq!(events[0].value, Value::Bool(true));
	assert_eq!(events[1].value, Value::Vector2(Axis2::new(0.5, 0.5)));
	assert!(events[0].sequence < events[1].sequence);
	// Capturing changes nothing; the samples leave once the last sink pulled past them.
	fixture.pull(&mut ui, |_| true);
	assert_eq!(fixture.collector.source_events().len(), 0);
}

#[test]
fn resetting_a_seat_discards_interrupted_input_and_allows_a_new_press_without_a_release() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	let button = sink.button("Mouse.LeftButton", TickPolicy::OnChange);
	fixture.record("Mouse.LeftButton", true);
	fixture.pull(&mut sink, handles(button));
	sink.published();

	// A lost window may never receive this press's release. Its queued motion
	// and retained press must disappear while another seat keeps its input.
	fixture.record("Mouse.Position", Axis2::new(0.5, 0.5));
	fixture.collector.record(
		SeatHandle(1),
		fixture.mouse,
		TriggerReference::Name("Mouse.LeftButton"),
		true.into(),
	);
	fixture.collector.reset_seat(SeatHandle::stub());
	sink.sink.cancel_seat(SeatHandle::stub());
	assert!(sink.published()[0].is_cancelled());

	fixture.record("Mouse.LeftButton", true);
	let resolved = fixture.pull(&mut sink, handles(button));
	assert_eq!(resolved.len(), 2);
	let published = sink.published();
	assert_eq!(published[0].seat_handle(), SeatHandle(1));
	assert_eq!(published[1].seat_handle(), SeatHandle::stub());
	assert!(published.iter().all(|event| event.value() == Value::Bool(true)));
	assert_eq!(fixture.value("Mouse.Position"), Value::Vector2(Axis2::zero()));
}

#[test]
fn a_capturing_sink_keeps_the_same_input_from_reaching_the_next_sink() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");
	let panel = ui.actions.create(Action::new(&[binding], Types::Vector2));
	let shoot = game.actions.create(Action::new(&[binding], Types::Vector2));

	// The UI only handles clicks on its own half of the window.
	let inside_panel = |resolved: &ResolvedAction| matches!(resolved.value, Value::Vector2(position) if position.x < 0.0);
	for x in [-0.5, 0.5, -0.25] {
		fixture.record("Mouse.Position", Axis2::new(x, 0.0));
		fixture.record("Mouse.LeftButton", true);
		fixture.record("Mouse.LeftButton", false);
	}

	let ui_actions = fixture.pull(&mut ui, inside_panel);
	let game_actions = fixture.pull(&mut game, pass);

	// Each click keeps the position it captured, and only the passed one falls through.
	assert_eq!(
		ui_actions.iter().map(|resolved| resolved.value).collect::<Vec<_>>(),
		[-0.5, 0.5, -0.25].map(|x| Value::Vector2(Axis2::new(x, 0.0)))
	);
	assert_eq!(
		game_actions.iter().map(|resolved| resolved.value).collect::<Vec<_>>(),
		[Value::Vector2(Axis2::new(0.5, 0.0))]
	);
	assert_eq!(
		ui.published().iter().map(ActionEvent::handle).collect::<Vec<_>>(),
		[panel, panel, panel]
	);
	assert_eq!(game.published().iter().map(ActionEvent::handle).collect::<Vec<_>>(), [shoot]);
}

#[test]
fn a_modal_sink_captures_input_no_action_of_its_own_matched() {
	let mut fixture = Fixture::new();
	let mut modal = fixture.sink();
	let mut game = fixture.sink();
	let close = modal.button("Keyboard.Escape", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Keyboard.Escape", true);
	fixture.record("Mouse.LeftButton", true);

	let resolved = fixture.pull(&mut modal, handles(close));
	// The modal swallows the click too, although it has no action bound to it.
	modal.sink.capture_pending(&mut fixture.collector);

	assert_eq!(resolved.len(), 1);
	assert_eq!(resolved[0].handle, Some(close));
	assert!(fixture.pull(&mut game, pass).is_empty());
	assert!(game.published().is_empty());
}

#[test]
fn a_repeated_down_record_does_not_start_a_second_interaction() {
	let mut fixture = Fixture::new();
	let mut game = fixture.sink();
	game.button("Keyboard.W", TickPolicy::OnChange);

	fixture.record("Keyboard.W", true);
	fixture.record("Keyboard.W", true);
	assert_eq!(fixture.pull(&mut game, pass).len(), 1);

	// The platform keeps repeating the held key, which is still one hold.
	fixture.record("Keyboard.W", true);
	assert!(fixture.pull(&mut game, pass).is_empty());
}

#[test]
fn cancelling_a_sinks_action_neutralizes_it_and_requires_a_fresh_press() {
	let mut fixture = Fixture::new();
	let mut game = fixture.sink();
	let mut ui = fixture.sink();
	let movement = game.button("Keyboard.W", TickPolicy::WhileActive);
	let menu = ui.button("Keyboard.W", TickPolicy::OnChange);

	// The press resolves the action once; the record's own event is this tick's emission.
	fixture.record("Keyboard.W", true);
	assert_eq!(fixture.pull(&mut game, handles(movement)).len(), 1);

	// The held action keeps repeating without new input.
	assert_eq!(fixture.pull(&mut game, handles(movement)).len(), 1);

	game.sink.cancel_action(SeatHandle::stub(), ActionHandle(0));
	let cancelled = game.published();
	let last = cancelled.last().expect("the cancelled action publishes its neutral value");
	assert!(last.is_cancelled());
	assert_eq!(last.value(), Value::Bool(false));

	// The cancelled action stops repeating, and the still-held key starts nothing.
	assert!(fixture.pull(&mut game, handles(movement)).is_empty());
	fixture.record("Keyboard.W", true);
	assert!(fixture.pull(&mut game, handles(movement)).is_empty());

	// Cancellation keeps the capture through release, even if the old sink stops pulling.
	fixture.record("Keyboard.W", false);
	assert!(fixture.pull(&mut ui, pass).is_empty());
	fixture.record("Keyboard.W", true);
	fixture.pull(&mut game, pass);
	assert_eq!(fixture.pull(&mut ui, handles(menu)).len(), 1);
}

#[test]
fn cancelling_one_device_leaves_another_device_holding() {
	let mut fixture = Fixture::new();
	let mut game = fixture.sink();
	let keys = game.button("Keyboard.W", TickPolicy::WhileActive);
	let click = game.button("Mouse.LeftButton", TickPolicy::WhileActive);

	fixture.record("Keyboard.W", true);
	fixture.record("Mouse.LeftButton", true);
	fixture.pull(&mut game, |_| true);
	let _ = game.published();

	game.sink.cancel_device(SeatHandle::stub(), fixture.keyboard);
	let held = fixture.pull(&mut game, pass);
	let published = game.published();

	assert!(published.iter().any(|event| event.handle() == keys && event.is_cancelled()));
	assert!(
		held.iter()
			.any(|resolved| resolved.handle == Some(click) && resolved.value == Value::Bool(true))
	);
	assert!(!held.iter().any(|resolved| resolved.handle == Some(keys)));
}

#[test]
fn a_captured_key_does_not_contribute_to_another_sinks_direction() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	let typing = ui.button("Keyboard.W", TickPolicy::OnChange);
	game.actions.create(Action::new(
		&[
			ActionBindingDescription::new("Keyboard.W").mapped(Axis2::new(0.0, 1.0).into()),
			ActionBindingDescription::new("Keyboard.D").mapped(Axis2::new(1.0, 0.0).into()),
		],
		Types::Vector2,
	));

	fixture.record("Keyboard.W", true);
	fixture.pull(&mut ui, handles(typing));
	fixture.pull(&mut game, pass);

	fixture.record("Keyboard.D", true);
	let movement = fixture.pull(&mut game, pass);

	// The captured `W` is invisible here, so it cannot join the direction vector.
	assert_eq!(movement.len(), 1);
	assert_eq!(movement[0].value, Value::Vector2(Axis2::new(1.0, 0.0)));
}

#[test]
fn impulses_keep_their_order_and_never_repeat_on_later_ticks() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	sink.actions
		.create(Action::new(&[ActionBindingDescription::new("Mouse.Scroll")], Types::Float).tick_policy(TickPolicy::Always));
	sink.actions.create(
		Action::new(&[ActionBindingDescription::new("Keyboard.Character")], Types::Unicode)
			.tick_policy(TickPolicy::WhileActive),
	);

	fixture.record("Mouse.Scroll", 1.0f32);
	fixture.record("Keyboard.Character", 'a');
	fixture.record("Mouse.Scroll", -1.0f32);
	fixture.record("Keyboard.Character", 'b');

	let resolved = fixture.pull(&mut sink, pass);
	assert_eq!(
		resolved.iter().map(|resolved| resolved.value).collect::<Vec<_>>(),
		[
			Value::Float(1.0),
			Value::Unicode('a'),
			Value::Float(-1.0),
			Value::Unicode('b')
		]
	);

	assert!(fixture.pull(&mut sink, pass).is_empty());
}

#[test]
fn a_passed_release_still_ends_its_captured_press_before_the_next_press() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	ui.button("Mouse.LeftButton", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Mouse.LeftButton", true);
	fixture.pull(&mut ui, |_| true);
	assert!(fixture.pull(&mut game, pass).is_empty());

	// The capture includes the first release even when the UI passes it. The
	// next complete click in this same tick is available to gameplay.
	for pressed in [false, true, false] {
		fixture.record("Mouse.LeftButton", pressed);
	}
	let resolved = fixture.pull(&mut ui, pass);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(false), Value::Bool(true), Value::Bool(false)]
	);
	let resolved = fixture.pull(&mut game, pass);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(true), Value::Bool(false)]
	);
}

#[test]
fn capturing_a_press_does_not_take_a_release_captured_by_an_earlier_sink() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	ui.button("Mouse.LeftButton", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);
	fixture.record("Mouse.LeftButton", true);
	fixture.record("Mouse.LeftButton", false);
	fixture.pull(&mut ui, |action| action.value == Value::Bool(false));
	let resolved = fixture.pull(&mut game, |_| true);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(true)]
	);
}

#[test]
fn a_later_capture_cannot_change_an_earlier_direction_in_the_same_tick() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	ui.button("Keyboard.W", TickPolicy::OnChange);
	game.actions.create(Action::new(
		&[
			ActionBindingDescription::new("Keyboard.W").mapped(Axis2::new(0.0, 1.0).into()),
			ActionBindingDescription::new("Keyboard.D").mapped(Axis2::new(1.0, 0.0).into()),
		],
		Types::Vector2,
	));
	for (name, pressed) in [
		("Keyboard.D", true),
		("Keyboard.W", true),
		("Keyboard.D", false),
		("Keyboard.W", false),
		("Keyboard.W", true),
	] {
		fixture.record(name, pressed);
	}
	let presses = std::cell::Cell::new(0);
	fixture.pull(&mut ui, |action| {
		if action.value == Value::Bool(true) {
			presses.set(presses.get() + 1);
			return presses.get() == 2;
		}
		false
	});
	let resolved = fixture.pull(&mut game, pass);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[
			Axis2::new(1.0, 0.0),
			Axis2::new(1.0, 1.0).normalized(),
			Axis2::new(0.0, 1.0),
			Axis2::zero(),
		]
		.map(Value::Vector2)
	);
}

#[test]
fn cancelling_a_seat_preserves_other_seats_values_for_the_same_action() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	sink.button("Keyboard.W", TickPolicy::WhileActive);
	let second = fixture.collector.create_device(&fixture.keyboard_class);
	let third = fixture.collector.create_device(&fixture.keyboard_class);
	let sources = [
		(SeatHandle(0), fixture.keyboard),
		(SeatHandle(0), second),
		(SeatHandle(1), third),
	];
	for (seat, device) in sources {
		fixture
			.collector
			.record(seat, device, TriggerReference::Name("Keyboard.W"), Value::Bool(true));
	}
	fixture.pull(&mut sink, pass);
	let _ = sink.published();
	sink.sink.cancel_seat(SeatHandle(0));
	sink.sink.cancel_seat(SeatHandle(0));
	assert_eq!(sink.published().iter().filter(|event| event.is_cancelled()).count(), 2);
	for (seat, device) in sources {
		assert_eq!(
			sink.sink.action_state(seat, ActionHandle(0), device),
			Value::Bool(seat == SeatHandle(1))
		);
	}
	assert_eq!(fixture.pull(&mut sink, pass).len(), 1);
	assert_eq!(sink.published()[0].seat_handle(), SeatHandle(1));
}

#[test]
fn transient_boolean_records_do_not_claim_or_repeat() {
	let mut fixture = Fixture::new();
	let class = fixture.collector.register_device_class("Impulse");
	let trigger = fixture
		.collector
		.register_trigger(&class, "Pulse", TriggerDescription::<bool>::default().transient());
	let device = fixture.collector.create_device(&class);
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	ui.button("Impulse.Pulse", TickPolicy::Always);
	game.button("Impulse.Pulse", TickPolicy::Always);
	for capture in [true, false] {
		for _ in 0..2 {
			fixture.collector.record(
				SeatHandle::stub(),
				device,
				TriggerReference::Handle(trigger),
				Value::Bool(true),
			);
		}
		assert_eq!(fixture.pull(&mut ui, |_| capture).len(), 2);
		assert_eq!(fixture.pull(&mut game, pass).len(), if capture { 0 } else { 2 });
		assert!(fixture.pull(&mut ui, pass).is_empty());
		assert!(fixture.pull(&mut game, pass).is_empty());
	}
}

#[test]
fn an_arena_backed_sink_shares_input_with_a_global_sink() {
	let arena = bumpalo::Bump::new();
	let mut fixture = Fixture::new();
	let mut ui = InputSink::new_in(fixture.collector.add_sink(), DefaultChannel::new(), &arena);
	let drag = ui.create_action(
		&fixture.collector,
		Types::Boolean,
		&[ActionBindingDescription::new("Mouse.LeftButton")],
		TickPolicy::OnChange,
	);
	let mut game = fixture.sink();
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Mouse.LeftButton", true);
	ui.pull(&mut fixture.collector, |_| Capture::Captured);
	assert_eq!(ui.action_state(SeatHandle::stub(), drag, fixture.mouse), Value::Bool(true));
	assert!(fixture.pull(&mut game, pass).is_empty());
}

#[test]
fn snapshot_bindings_sample_each_press_in_queue_order() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	let other_mouse = fixture.collector.create_device(&fixture.mouse_class);
	sink.actions.create(
		Action::new(
			&[ActionBindingDescription::new("Mouse.Position")
				.triggered_by("Mouse.LeftButton")
				.trigger_on(TriggerMode::Press)],
			Types::Vector2,
		)
		.tick_policy(TickPolicy::Always),
	);
	let seat = SeatHandle::stub();
	// Neither a missing sample nor another device's position supplies a snapshot.
	fixture.collector.record(
		seat,
		other_mouse,
		TriggerReference::Name("Mouse.Position"),
		Value::Vector2(Axis2::new(99.0, 99.0)),
	);
	fixture.record("Mouse.LeftButton", true);
	assert_eq!(sink.count(&mut fixture.collector), 0);
	// The held button must be released first; a repeated press is not a new click.
	for (source, value) in [
		("Mouse.LeftButton", Value::Bool(false)),
		("Mouse.Position", Value::Vector2(Axis2::new(1.0, 2.0))),
		("Mouse.LeftButton", Value::Bool(true)),
		("Mouse.LeftButton", Value::Bool(false)),
		("Mouse.Position", Value::Vector2(Axis2::new(3.0, 4.0))),
		("Mouse.LeftButton", Value::Bool(true)),
		("Mouse.Position", Value::Vector2(Axis2::new(5.0, 6.0))),
		("Mouse.LeftButton", Value::Bool(false)),
	] {
		fixture.record(source, value);
	}
	fixture.pull(&mut sink, pass);
	assert_eq!(
		sink.published().iter().map(ActionEvent::value).collect::<Vec<_>>(),
		[Value::Vector2(Axis2::new(1.0, 2.0)), Value::Vector2(Axis2::new(3.0, 4.0))]
	);
	assert_eq!(sink.count(&mut fixture.collector), 0);
	// A later press samples the retained position without requiring more motion.
	fixture.record("Mouse.LeftButton", true);
	fixture.pull(&mut sink, pass);
	assert_eq!(
		sink.published().iter().map(ActionEvent::value).collect::<Vec<_>>(),
		[Value::Vector2(Axis2::new(5.0, 6.0))]
	);
	// Motion alone stays silent, and another seat cannot sample this seat's value.
	fixture.record("Mouse.Position", Axis2::new(7.0, 8.0));
	fixture.collector.record(
		SeatHandle(42),
		fixture.mouse,
		TriggerReference::Name("Mouse.LeftButton"),
		Value::Bool(true),
	);
	assert_eq!(sink.count(&mut fixture.collector), 0);
}

#[test]
fn click_phase_selects_the_snapshot_across_ticks() {
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");
	let release = sink
		.actions
		.create(Action::new(&[binding], Types::Vector2).tick_policy(TickPolicy::Always));
	let press = sink
		.actions
		.create(Action::new(&[binding.trigger_on(TriggerMode::Press)], Types::Vector2));
	fixture.record("Mouse.Position", Axis2::new(1.0, 2.0));
	fixture.record("Mouse.LeftButton", true);
	let actions = fixture.pull(&mut sink, pass);
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].handle, Some(press));
	assert_eq!(actions[0].value, Value::Vector2(Axis2::new(1.0, 2.0)));
	assert!(fixture.pull(&mut sink, pass).is_empty());
	fixture.record("Mouse.Position", Axis2::new(3.0, 4.0));
	fixture.record("Mouse.LeftButton", false);
	let actions = fixture.pull(&mut sink, pass);
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].handle, Some(release));
	assert_eq!(actions[0].value, Value::Vector2(Axis2::new(3.0, 4.0)));
	assert!(fixture.pull(&mut sink, pass).is_empty());
}

#[test]
fn drag_events_preserve_queue_order_and_do_not_repeat_when_held() {
	use ActionPhase::{Ended, Started, Updated};
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	sink.actions.create(
		Action::new(
			&[ActionBindingDescription::new("Mouse.Position").dragged_by("Mouse.LeftButton")],
			Types::Vector2,
		)
		.tick_policy(TickPolicy::Always),
	);
	for (source, value) in [
		("Mouse.Position", Value::Vector2(Axis2::new(1.0, 2.0))),
		("Mouse.LeftButton", Value::Bool(true)),
		("Mouse.LeftButton", Value::Bool(true)),
		("Mouse.Position", Value::Vector2(Axis2::new(3.0, 4.0))),
		("Mouse.LeftButton", Value::Bool(false)),
		("Mouse.LeftButton", Value::Bool(false)),
		("Mouse.LeftButton", Value::Bool(true)),
	] {
		fixture.record(source, value);
	}
	fixture.pull(&mut sink, pass);
	assert_eq!(
		sink.published()
			.iter()
			.map(|event| (event.phase(), event.value()))
			.collect::<Vec<_>>(),
		[
			(Started, Value::Vector2(Axis2::new(1.0, 2.0))),
			(Updated, Value::Vector2(Axis2::new(3.0, 4.0))),
			(Ended, Value::Vector2(Axis2::new(3.0, 4.0))),
			(Started, Value::Vector2(Axis2::new(3.0, 4.0))),
		]
	);
	assert_eq!(sink.count(&mut fixture.collector), 0);
	sink.sink.cancel_seat(SeatHandle::stub());
	let cancelled = sink.published();
	assert_eq!(cancelled.len(), 1);
	assert!(cancelled[0].is_cancelled());
	assert_eq!(cancelled[0].value(), Value::Vector2(Axis2::new(3.0, 4.0)));
	// Neither the repeated press nor its release restarts the cancelled drag.
	for pressed in [true, false] {
		fixture.record("Mouse.LeftButton", pressed);
	}
	assert_eq!(sink.count(&mut fixture.collector), 0);
}

#[test]
fn a_drag_captures_movement_until_release_and_then_allows_other_input() {
	use ActionPhase::{Ended, Started, Updated};
	let mut fixture = Fixture::new();
	let mut ui = fixture.sink();
	let mut game = fixture.sink();
	let binding = ActionBindingDescription::new("Mouse.Position").dragged_by("Mouse.LeftButton");
	let drag = ui
		.actions
		.create(Action::new(&[binding], Types::Vector2).tick_policy(TickPolicy::Always));
	game.actions.create(Action::new(&[binding], Types::Vector2));
	let game_motion = game.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position")],
		Types::Vector2,
	));
	// Only the start is accepted by hit testing. Capture must survive moving outside.
	let start = |action: &ResolvedAction| action.phase == Started;
	fixture.record("Mouse.Position", Axis2::new(-0.5, 0.0));
	fixture.record("Mouse.LeftButton", true);
	let actions = fixture.pull(&mut ui, start);
	assert_eq!(actions.len(), 1);
	assert_eq!(actions[0].phase, Started);
	assert_eq!(actions[0].handle, Some(drag));
	assert!(
		fixture
			.pull(&mut game, pass)
			.iter()
			.all(|action| action.handle == Some(game_motion))
	);
	assert!(fixture.pull(&mut ui, pass).is_empty());
	fixture.record("Mouse.Position", Axis2::new(0.8, 0.2));
	fixture.record("Mouse.LeftButton", false);
	let actions = fixture.pull(&mut ui, pass);
	assert_eq!(
		actions.iter().map(|action| (action.phase, action.value)).collect::<Vec<_>>(),
		[
			(Updated, Value::Vector2(Axis2::new(0.8, 0.2))),
			(Ended, Value::Vector2(Axis2::new(0.8, 0.2)))
		]
	);
	assert!(fixture.pull(&mut game, pass).is_empty());
	assert_eq!(
		ui.published().iter().map(ActionEvent::phase).collect::<Vec<_>>(),
		[Started, Updated, Ended]
	);
	fixture.record("Mouse.Position", Axis2::new(0.9, 0.3));
	assert!(fixture.pull(&mut ui, pass).is_empty());
	assert_eq!(fixture.pull(&mut game, pass)[0].handle, Some(game_motion));
}

#[test]
fn cancelling_a_drag_at_the_origin_does_not_end_or_restart_it() {
	use ActionPhase::{Cancelled, Started};
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	sink.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position").dragged_by("Mouse.LeftButton")],
		Types::Vector2,
	));
	fixture.record("Mouse.Position", Axis2::zero());
	fixture.record("Mouse.LeftButton", true);
	fixture.pull(&mut sink, |_| true);
	sink.published();
	sink.sink.cancel_action(SeatHandle::stub(), ActionHandle(0));
	sink.sink.cancel_action(SeatHandle::stub(), ActionHandle(0));
	let cancelled = sink.published();
	assert_eq!(cancelled.len(), 1);
	assert_eq!(cancelled[0].phase(), Cancelled);
	assert!(cancelled[0].is_cancelled());
	assert_eq!(cancelled[0].value(), Value::Vector2(Axis2::zero()));
	fixture.record("Mouse.LeftButton", true); // Platform repeat is not a new drag.
	fixture.record("Mouse.Position", Axis2::new(1.0, 2.0));
	fixture.record("Mouse.LeftButton", false);
	assert!(fixture.pull(&mut sink, pass).is_empty());
	fixture.record("Mouse.LeftButton", true);
	assert_eq!(fixture.pull(&mut sink, pass)[0].phase, Started);
}

#[test]
fn drags_require_a_sample_at_press_and_keep_devices_and_seats_separate() {
	use ActionPhase::{Ended, Started, Updated};
	let mut fixture = Fixture::new();
	let mut sink = fixture.sink();
	sink.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position").dragged_by("Mouse.LeftButton")],
		Types::Vector2,
	));
	let other = fixture.collector.create_device(&fixture.mouse_class);
	let other_seat = SeatHandle(12);
	// A value from another seat or device cannot supply a start position.
	for (seat, device) in [(SeatHandle::stub(), other), (other_seat, fixture.mouse)] {
		fixture.collector.record(
			seat,
			device,
			TriggerReference::Name("Mouse.Position"),
			Axis2::new(8.0, 9.0).into(),
		);
	}
	fixture.record("Mouse.LeftButton", true);
	fixture.record("Mouse.Position", Axis2::new(1.0, 2.0));
	fixture.record("Mouse.LeftButton", false);
	assert!(fixture.pull(&mut sink, pass).is_empty());
	let sources = [
		(SeatHandle::stub(), fixture.mouse),
		(SeatHandle::stub(), other),
		(other_seat, fixture.mouse),
	];
	for (seat, device) in sources {
		fixture
			.collector
			.record(seat, device, TriggerReference::Name("Mouse.LeftButton"), true.into());
	}
	assert_eq!(
		fixture
			.pull(&mut sink, pass)
			.iter()
			.map(|action| action.phase)
			.collect::<Vec<_>>(),
		[Started; 3]
	);
	sink.published();
	sink.sink.cancel_device(SeatHandle::stub(), fixture.mouse);
	assert_eq!(sink.published().len(), 1);
	for (seat, device) in sources {
		fixture.collector.record(
			seat,
			device,
			TriggerReference::Name("Mouse.Position"),
			Axis2::new(3.0, 4.0).into(),
		);
		fixture
			.collector
			.record(seat, device, TriggerReference::Name("Mouse.LeftButton"), false.into());
	}
	assert_eq!(
		fixture
			.pull(&mut sink, pass)
			.iter()
			.map(|action| action.phase)
			.collect::<Vec<_>>(),
		[Updated, Ended, Updated, Ended]
	);
}
