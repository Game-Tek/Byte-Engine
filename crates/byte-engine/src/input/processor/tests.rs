//! Layered input behavior, exercised end to end through the public input steps.

use crate::{
	core::{
		channel::{Channel as _, DefaultChannel},
		factory::{Factory, Handle},
		listener::{DefaultListener, Listener as _},
	},
	input::{
		Action, ActionBindingDescription, ActionEvent, ActionHandle, ActionProcessor, Axis2, Consumption, DeviceHandle,
		InputEvents, ResolvedAction, SeatHandle, TickPolicy, TriggerReference, TriggerRegistry, Types, Value, utils,
	},
};

/// The `Fixture` struct runs two input layers over one shared input queue.
struct Fixture {
	events: InputEvents,
	mouse: DeviceHandle,
	keyboard: DeviceHandle,
	keyboard_class: crate::input::device::DeviceClassHandle,
}

/// The `Layer` struct is one consumer: its action factory, processor, and events.
struct Layer {
	actions: Factory<Action>,
	processor: ActionProcessor,
	events: DefaultListener<ActionEvent>,
}

impl Fixture {
	fn new() -> Self {
		let mut events = InputEvents::new();
		let mouse_class = utils::register_mouse_device_class(&mut events);
		let keyboard_class = utils::register_keyboard_device_class(&mut events);
		let mouse = events.create_device(&mouse_class);
		let keyboard = events.create_device(&keyboard_class);

		Self {
			events,
			mouse,
			keyboard,
			keyboard_class,
		}
	}

	/// Adds one input layer, in the order the application will process it.
	fn layer(&mut self) -> Layer {
		let actions = Factory::new();
		let channel = DefaultChannel::new();
		let events = channel.listener();
		let processor = ActionProcessor::new(self.events.add_consumer(), channel).with_declarations(actions.listener());

		Layer {
			actions,
			processor,
			events,
		}
	}

	fn record(&mut self, name: &'static str, value: impl Into<Value>) {
		let device = if name.starts_with("Mouse.") {
			self.mouse
		} else {
			self.keyboard
		};
		self.events
			.record(SeatHandle::stub(), device, TriggerReference::Name(name), value.into());
	}

	/// Processes one layer and returns the actions it resolved this tick.
	///
	/// `consume` decides which actions the layer handles, as an application's
	/// hit-testing or focus logic would.
	fn process(&mut self, layer: &mut Layer, consume: impl Fn(&ResolvedAction) -> bool) -> Vec<ResolvedAction> {
		let mut resolved = Vec::new();
		layer.processor.process(&mut self.events, |action| {
			resolved.push(*action);
			if consume(action) {
				Consumption::Consumed
			} else {
				Consumption::Ignored
			}
		});

		resolved
	}

	/// Ends the tick and drops this tick's records.
	fn end_tick(&mut self) {
		self.events.end_tick();
	}
}

impl Layer {
	fn button(&mut self, trigger: &'static str, policy: TickPolicy) -> Handle {
		self.actions
			.create(Action::new(&[ActionBindingDescription::new(trigger)], Types::Boolean).tick_policy(policy))
	}

	fn published(&mut self) -> Vec<ActionEvent> {
		self.events.to_vec()
	}
}

fn ignore(_: &ResolvedAction) -> bool {
	false
}

fn handles(action: Handle) -> impl Fn(&ResolvedAction) -> bool {
	move |resolved: &ResolvedAction| resolved.handle == Some(action)
}

#[test]
fn a_consuming_layer_keeps_the_same_input_from_reaching_the_next_layer() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
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

	let ui_actions = fixture.process(&mut ui, inside_panel);
	let game_actions = fixture.process(&mut game, ignore);

	// Each click keeps the position it captured, and only the ignored one falls through.
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
fn a_modal_layer_consumes_input_no_action_of_its_own_matched() {
	let mut fixture = Fixture::new();
	let mut modal = fixture.layer();
	let mut game = fixture.layer();
	let close = modal.button("Keyboard.Escape", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Keyboard.Escape", true);
	fixture.record("Mouse.LeftButton", true);

	let resolved = fixture.process(&mut modal, handles(close));
	// The modal swallows the click too, although it has no action bound to it.
	modal.processor.consume_pending(&mut fixture.events);

	assert_eq!(resolved.len(), 1);
	assert_eq!(resolved[0].handle, Some(close));
	assert!(fixture.process(&mut game, ignore).is_empty());
	assert!(game.published().is_empty());
}

#[test]
fn a_repeated_down_record_does_not_start_a_second_interaction() {
	let mut fixture = Fixture::new();
	let mut game = fixture.layer();
	game.button("Keyboard.W", TickPolicy::OnChange);

	fixture.record("Keyboard.W", true);
	fixture.record("Keyboard.W", true);
	assert_eq!(fixture.process(&mut game, ignore).len(), 1);
	fixture.end_tick();

	// The platform keeps repeating the held key, which is still one hold.
	fixture.record("Keyboard.W", true);
	assert!(fixture.process(&mut game, ignore).is_empty());
}

#[test]
fn cancelling_a_layers_action_neutralizes_it_and_requires_a_fresh_press() {
	let mut fixture = Fixture::new();
	let mut game = fixture.layer();
	let mut ui = fixture.layer();
	let movement = game.button("Keyboard.W", TickPolicy::WhileActive);
	let menu = ui.button("Keyboard.W", TickPolicy::OnChange);

	// The press resolves the action and its tick policy repeats it in the same tick.
	fixture.record("Keyboard.W", true);
	assert_eq!(fixture.process(&mut game, handles(movement)).len(), 2);
	fixture.end_tick();

	// The held action keeps repeating without new input.
	assert_eq!(fixture.process(&mut game, handles(movement)).len(), 1);
	fixture.end_tick();

	game.processor.cancel_action(SeatHandle::stub(), ActionHandle(0));
	let cancelled = game.published();
	let last = cancelled.last().expect("the cancelled action publishes its neutral value");
	assert!(last.is_cancelled());
	assert_eq!(last.value(), Value::Bool(false));

	// The cancelled action stops repeating, and the still-held key starts nothing.
	assert!(fixture.process(&mut game, handles(movement)).is_empty());
	fixture.record("Keyboard.W", true);
	assert!(fixture.process(&mut game, handles(movement)).is_empty());
	fixture.end_tick();

	// Cancellation keeps ownership through release, even if the old layer stops processing.
	fixture.record("Keyboard.W", false);
	assert!(fixture.process(&mut ui, ignore).is_empty());
	fixture.end_tick();
	fixture.record("Keyboard.W", true);
	fixture.process(&mut game, ignore);
	assert_eq!(fixture.process(&mut ui, handles(menu)).len(), 1);
}

#[test]
fn cancelling_one_device_leaves_another_device_holding() {
	let mut fixture = Fixture::new();
	let mut game = fixture.layer();
	let keys = game.button("Keyboard.W", TickPolicy::WhileActive);
	let click = game.button("Mouse.LeftButton", TickPolicy::WhileActive);

	fixture.record("Keyboard.W", true);
	fixture.record("Mouse.LeftButton", true);
	fixture.process(&mut game, |_| true);
	fixture.end_tick();
	let _ = game.published();

	game.processor.cancel_device(SeatHandle::stub(), fixture.keyboard);
	let held = fixture.process(&mut game, ignore);
	let published = game.published();

	assert!(published.iter().any(|event| event.handle() == keys && event.is_cancelled()));
	assert!(
		held.iter()
			.any(|resolved| resolved.handle == Some(click) && resolved.value == Value::Bool(true))
	);
	assert!(!held.iter().any(|resolved| resolved.handle == Some(keys)));
}

#[test]
fn a_consumed_key_does_not_contribute_to_another_layers_direction() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
	let typing = ui.button("Keyboard.W", TickPolicy::OnChange);
	game.actions.create(Action::new(
		&[
			ActionBindingDescription::new("Keyboard.W").mapped(Axis2::new(0.0, 1.0).into()),
			ActionBindingDescription::new("Keyboard.D").mapped(Axis2::new(1.0, 0.0).into()),
		],
		Types::Vector2,
	));

	fixture.record("Keyboard.W", true);
	fixture.process(&mut ui, handles(typing));
	fixture.process(&mut game, ignore);
	fixture.end_tick();

	fixture.record("Keyboard.D", true);
	let movement = fixture.process(&mut game, ignore);

	// The consumed `W` is invisible here, so it cannot join the direction vector.
	assert_eq!(movement.len(), 1);
	assert_eq!(movement[0].value, Value::Vector2(Axis2::new(1.0, 0.0)));
}

#[test]
fn impulses_keep_their_order_and_never_repeat_on_later_ticks() {
	let mut fixture = Fixture::new();
	let mut layer = fixture.layer();
	layer
		.actions
		.create(Action::new(&[ActionBindingDescription::new("Mouse.Scroll")], Types::Float).tick_policy(TickPolicy::Always));
	layer.actions.create(
		Action::new(&[ActionBindingDescription::new("Keyboard.Character")], Types::Unicode)
			.tick_policy(TickPolicy::WhileActive),
	);

	fixture.record("Mouse.Scroll", 1.0f32);
	fixture.record("Keyboard.Character", 'a');
	fixture.record("Mouse.Scroll", -1.0f32);
	fixture.record("Keyboard.Character", 'b');

	let resolved = fixture.process(&mut layer, ignore);
	assert_eq!(
		resolved.iter().map(|resolved| resolved.value).collect::<Vec<_>>(),
		[
			Value::Float(1.0),
			Value::Unicode('a'),
			Value::Float(-1.0),
			Value::Unicode('b')
		]
	);
	fixture.end_tick();

	assert!(fixture.process(&mut layer, ignore).is_empty());
}

#[test]
fn an_ignored_release_still_ends_its_owned_press_before_the_next_press() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
	ui.button("Mouse.LeftButton", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Mouse.LeftButton", true);
	fixture.process(&mut ui, |_| true);
	assert!(fixture.process(&mut game, ignore).is_empty());
	fixture.end_tick();

	// Ownership includes the first release even when the UI ignores it. The
	// next complete click in this same tick is available to gameplay.
	for pressed in [false, true, false] {
		fixture.record("Mouse.LeftButton", pressed);
	}
	let resolved = fixture.process(&mut ui, ignore);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(false), Value::Bool(true), Value::Bool(false)]
	);
	let resolved = fixture.process(&mut game, ignore);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(true), Value::Bool(false)]
	);
}

#[test]
fn claiming_a_press_does_not_take_a_release_consumed_by_an_earlier_layer() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
	ui.button("Mouse.LeftButton", TickPolicy::OnChange);
	game.button("Mouse.LeftButton", TickPolicy::OnChange);
	fixture.record("Mouse.LeftButton", true);
	fixture.record("Mouse.LeftButton", false);
	fixture.process(&mut ui, |action| action.value == Value::Bool(false));
	let resolved = fixture.process(&mut game, |_| true);
	assert_eq!(
		resolved.iter().map(|action| action.value).collect::<Vec<_>>(),
		[Value::Bool(true)]
	);
}

#[test]
fn a_later_claim_cannot_change_an_earlier_direction_in_the_same_tick() {
	let mut fixture = Fixture::new();
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
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
	fixture.process(&mut ui, |action| {
		if action.value == Value::Bool(true) {
			presses.set(presses.get() + 1);
			return presses.get() == 2;
		}
		false
	});
	let resolved = fixture.process(&mut game, ignore);
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
	let mut layer = fixture.layer();
	layer.button("Keyboard.W", TickPolicy::WhileActive);
	let second = fixture.events.create_device(&fixture.keyboard_class);
	let third = fixture.events.create_device(&fixture.keyboard_class);
	let sources = [
		(SeatHandle(0), fixture.keyboard),
		(SeatHandle(0), second),
		(SeatHandle(1), third),
	];
	for (seat, device) in sources {
		fixture
			.events
			.record(seat, device, TriggerReference::Name("Keyboard.W"), Value::Bool(true));
	}
	fixture.process(&mut layer, ignore);
	fixture.end_tick();
	let _ = layer.published();
	layer.processor.cancel_seat(SeatHandle(0));
	layer.processor.cancel_seat(SeatHandle(0));
	assert_eq!(layer.published().iter().filter(|event| event.is_cancelled()).count(), 2);
	for (seat, device) in sources {
		assert_eq!(
			layer.processor.action_state(seat, ActionHandle(0), device),
			Value::Bool(seat == SeatHandle(1))
		);
	}
	assert_eq!(fixture.process(&mut layer, ignore).len(), 1);
	assert_eq!(layer.published()[0].seat_handle(), SeatHandle(1));
}

#[test]
fn transient_boolean_records_do_not_claim_or_repeat() {
	let mut fixture = Fixture::new();
	let class = fixture.events.register_device_class("Impulse");
	let trigger = fixture.events.register_trigger(
		&class,
		"Pulse",
		crate::input::trigger::TriggerDescription::<bool>::default().transient(),
	);
	let device = fixture.events.create_device(&class);
	let mut ui = fixture.layer();
	let mut game = fixture.layer();
	ui.button("Impulse.Pulse", TickPolicy::Always);
	game.button("Impulse.Pulse", TickPolicy::Always);
	for consume in [true, false] {
		for _ in 0..2 {
			fixture.events.record(
				SeatHandle::stub(),
				device,
				TriggerReference::Handle(trigger),
				Value::Bool(true),
			);
		}
		assert_eq!(fixture.process(&mut ui, |_| consume).len(), 2);
		assert_eq!(fixture.process(&mut game, ignore).len(), if consume { 0 } else { 2 });
		fixture.end_tick();
		assert!(fixture.process(&mut ui, ignore).is_empty());
		assert!(fixture.process(&mut game, ignore).is_empty());
	}
}

#[test]
fn an_arena_backed_layer_shares_input_with_a_global_layer() {
	let arena = bumpalo::Bump::new();
	let mut fixture = Fixture::new();
	let mut ui = ActionProcessor::new_in(fixture.events.add_consumer(), DefaultChannel::new(), &arena);
	let drag = ui.create_action(
		&fixture.events,
		Types::Boolean,
		&[ActionBindingDescription::new("Mouse.LeftButton")],
		TickPolicy::OnChange,
	);
	let mut game = fixture.layer();
	game.button("Mouse.LeftButton", TickPolicy::OnChange);

	fixture.record("Mouse.LeftButton", true);
	ui.process(&mut fixture.events, |_| Consumption::Consumed);
	assert_eq!(ui.action_state(SeatHandle::stub(), drag, fixture.mouse), Value::Bool(true));
	assert!(fixture.process(&mut game, ignore).is_empty());
}
