//! Guard the input pipeline against regressions with
//! `cargo bench -p byte-engine --bench input --no-default-features`.
//!
//! Each workload records a tick's source events into an [`InputCollector`],
//! pulls every sink in priority order, and drains the published events. Global
//! and arena variants share one implementation.

#![feature(allocator_api)]

use std::alloc::{Allocator, Global};

use byte_engine::{
	core::{
		channel::DefaultChannel,
		factory::Factory,
		listener::{DefaultListener, Listener},
	},
	input::{
		Action, ActionBindingDescription, ActionEvent, ActionPhase, Axis2, Capture, InputCollector, InputSink, ResolvedAction,
		SeatHandle, TickPolicy, TriggerReference, Types, Value, utils,
	},
};

fn main() {
	divan::main();
}

/// The controls one click records: where the pointer is, then press and release.
const CLICK: [(&str, Value); 3] = [
	("Mouse.Position", Value::Vector2(Axis2::new(-0.5, 0.0))),
	("Mouse.LeftButton", Value::Bool(true)),
	("Mouse.LeftButton", Value::Bool(false)),
];

/// The `Consumer` struct is one sink with declared actions and a listener draining its events.
struct Consumer<A: Allocator + Clone> {
	actions: Factory<Action>,
	sink: InputSink<A>,
	events: DefaultListener<ActionEvent>,
}

impl<A: Allocator + Clone> Consumer<A> {
	fn new(collector: &mut InputCollector<A>, allocator: A) -> Self {
		let actions = Factory::new();
		let channel = DefaultChannel::new();
		let events = channel.listener();
		let sink = InputSink::new_in(collector.add_sink(), channel, allocator).with_declarations(actions.listener());
		Self { actions, sink, events }
	}

	fn drain(&mut self) {
		while let Some(event) = self.events.read() {
			divan::black_box(event);
		}
	}
}

/// Records the standard mouse and keyboard, returning their devices.
fn window<A: Allocator + Clone>(collector: &mut InputCollector<A>) -> [byte_engine::input::DeviceHandle; 2] {
	let mouse = utils::register_mouse_device_class(collector);
	let keyboard = utils::register_keyboard_device_class(collector);
	[collector.create_device(&mouse), collector.create_device(&keyboard)]
}

fn record<A: Allocator + Clone>(
	collector: &mut InputCollector<A>,
	[mouse, keyboard]: [byte_engine::input::DeviceHandle; 2],
	name: &'static str,
	value: Value,
) {
	let device = if name.starts_with("Mouse.") { mouse } else { keyboard };
	collector.record(
		SeatHandle::stub(),
		device,
		TriggerReference::Name(name),
		divan::black_box(value),
	);
}

/// Measures one tick of a single sink that passes everything, as `GraphicsApplication` runs.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn single_sink(bencher: divan::Bencher) {
	single_sink_in(bencher, Global);
}

/// Measures the same workload using a retained arena.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn single_sink_arena(bencher: divan::Bencher) {
	let arena = bumpalo::Bump::new();
	single_sink_in(bencher, &arena);
}

fn single_sink_in<A: Allocator + Clone>(bencher: divan::Bencher, allocator: A) {
	let mut collector = InputCollector::new_in(allocator.clone());
	let devices = window(&mut collector);
	let mut world = Consumer::new(&mut collector, allocator);
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");
	world.actions.create(Action::new(&[binding], Types::Vector2));
	world.actions.create(Action::new(&[binding], Types::Vector2));
	world.sink.pull(&mut collector, |_| Capture::Passed);

	bencher.bench_local(|| {
		for (name, value) in CLICK {
			record(&mut collector, devices, name, value);
		}
		world.sink.pull(&mut collector, |_| Capture::Passed);
		world.drain();
	});
}

/// Measures one tick of two sinks competing for the same click.
///
/// The first sink captures clicks on its own half of the window, so the second
/// sink only resolves the ones it passed.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn two_sinks(bencher: divan::Bencher) {
	two_sinks_in(bencher, Global);
}

/// Measures the same workload using a retained arena.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn two_sinks_arena(bencher: divan::Bencher) {
	let arena = bumpalo::Bump::new();
	two_sinks_in(bencher, &arena);
}

fn two_sinks_in<A: Allocator + Clone>(bencher: divan::Bencher, allocator: A) {
	let mut collector = InputCollector::new_in(allocator.clone());
	let devices = window(&mut collector);
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");
	let mut sinks = [(); 2].map(|()| {
		let consumer = Consumer::new(&mut collector, allocator.clone());
		consumer.actions.create(Action::new(&[binding], Types::Vector2));
		consumer
	});

	bencher.bench_local(|| {
		for (name, value) in CLICK {
			record(&mut collector, devices, name, value);
		}
		for (index, consumer) in sinks.iter_mut().enumerate() {
			consumer
				.sink
				.pull(&mut collector, |resolved: &ResolvedAction| match resolved.value {
					// Only the first sink claims the left half of the window.
					Value::Vector2(position) if index == 0 && position.x < 0.0 => Capture::Captured,
					_ => Capture::Passed,
				});
		}
		for consumer in &mut sinks {
			consumer.drain();
		}
	});
}

/// Measures directional actions through press, held, and release ticks.
#[divan::bench(args = [1, 32], sample_count = 100, sample_size = 1000)]
fn held_directions(bencher: divan::Bencher, action_count: usize) {
	held_directions_in(bencher, action_count, Global);
}

/// Measures the same workload using a retained arena.
#[divan::bench(args = [1, 32], sample_count = 100, sample_size = 1000)]
fn held_directions_arena(bencher: divan::Bencher, action_count: usize) {
	let arena = bumpalo::Bump::new();
	held_directions_in(bencher, action_count, &arena);
}

fn held_directions_in<A: Allocator + Clone>(bencher: divan::Bencher, action_count: usize, allocator: A) {
	let mut collector = InputCollector::new_in(allocator.clone());
	let devices = window(&mut collector);
	let mut sink = InputSink::new_in(collector.add_sink(), DefaultChannel::new(), allocator);
	for _ in 0..action_count {
		sink.create_action(
			&collector,
			Types::Vector2,
			&[
				ActionBindingDescription::new("Keyboard.W").mapped(Axis2::new(0.0, 1.0).into()),
				ActionBindingDescription::new("Keyboard.D").mapped(Axis2::new(1.0, 0.0).into()),
			],
			TickPolicy::WhileActive,
		);
	}
	bencher.bench_local(|| {
		for pressed in [Some(true), None, Some(false)] {
			if let Some(pressed) = pressed {
				for name in ["Keyboard.W", "Keyboard.D"] {
					record(&mut collector, devices, name, Value::Bool(pressed));
				}
			}
			sink.pull(&mut collector, |resolved| {
				divan::black_box(resolved);
				Capture::Captured
			});
		}
	});
}

/// Measures a gameplay sink with many actions bound to distinct keys, two of which change per tick.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn bound_keys(bencher: divan::Bencher) {
	use byte_engine::input::{TriggerRegistry as _, trigger::TriggerDescription};

	let mut collector = InputCollector::new();
	let class = collector.register_device_class("Deck");
	let keys: Vec<_> = (0..32)
		.map(|key| collector.register_trigger(&class, &format!("Key{key}"), TriggerDescription::<bool>::default()))
		.collect();
	let device = collector.create_device(&class);
	let mut sink = InputSink::new(collector.add_sink(), DefaultChannel::new());
	let names: Vec<String> = (0..32).map(|key| format!("Deck.Key{key}")).collect();
	for name in &names {
		let name: &'static str = Box::leak(name.clone().into_boxed_str());
		sink.create_action(
			&collector,
			Types::Boolean,
			&[ActionBindingDescription::new(name)],
			TickPolicy::OnChange,
		);
	}
	bencher.bench_local(|| {
		for pressed in [true, false] {
			for key in [keys[3], keys[29]] {
				collector.record(
					SeatHandle::stub(),
					device,
					TriggerReference::Handle(key),
					divan::black_box(Value::Bool(pressed)),
				);
			}
			sink.pull(&mut collector, |resolved| {
				divan::black_box(resolved);
				Capture::Passed
			});
		}
	});
}

/// Measures a UI sink capturing a drag ahead of a scene sink, as the isometric sandbox does.
///
/// Each iteration runs three ticks: a press on a card that the UI captures,
/// movement across the scene while captured, and a release followed by a scene
/// click, a scroll, and a key the UI passes.
#[divan::bench(sample_count = 100, sample_size = 500)]
fn drag_capture(bencher: divan::Bencher) {
	drag_capture_in(bencher, Global);
}

/// Measures the same workload using a retained arena.
#[divan::bench(sample_count = 100, sample_size = 500)]
fn drag_capture_arena(bencher: divan::Bencher) {
	let arena = bumpalo::Bump::new();
	drag_capture_in(bencher, &arena);
}

fn drag_capture_in<A: Allocator + Clone>(bencher: divan::Bencher, allocator: A) {
	let mut collector = InputCollector::new_in(allocator.clone());
	let devices = window(&mut collector);
	let mut interface = Consumer::new(&mut collector, allocator.clone());
	let card = interface.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position").dragged_by("Mouse.LeftButton")],
		Types::Vector2,
	));
	interface.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position")],
		Types::Vector2,
	));
	interface
		.actions
		.create(Action::new(&[ActionBindingDescription::new("Mouse.Scroll")], Types::Float));
	interface.actions.create(Action::new(
		&[ActionBindingDescription::new("Keyboard.Escape")],
		Types::Boolean,
	));
	let mut scene = Consumer::new(&mut collector, allocator);
	scene.actions.create(Action::new(
		&[ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton")],
		Types::Vector2,
	));
	scene
		.actions
		.create(Action::new(&[ActionBindingDescription::new("Mouse.Scroll")], Types::Float));
	scene.actions.create(Action::new(
		&[ActionBindingDescription::new("Keyboard.Space")],
		Types::Boolean,
	));
	let ticks: [&[(&str, Value)]; 3] = [
		&[
			("Mouse.Position", Value::Vector2(Axis2::new(-0.8, -0.8))),
			("Mouse.LeftButton", Value::Bool(true)),
		],
		&[
			("Mouse.Position", Value::Vector2(Axis2::new(-0.2, -0.3))),
			("Mouse.Position", Value::Vector2(Axis2::new(0.3, 0.1))),
			("Mouse.Position", Value::Vector2(Axis2::new(0.5, 0.4))),
		],
		&[
			("Mouse.LeftButton", Value::Bool(false)),
			("Mouse.LeftButton", Value::Bool(true)),
			("Mouse.LeftButton", Value::Bool(false)),
			("Mouse.Scroll", Value::Float(1.0)),
			("Keyboard.Space", Value::Bool(true)),
			("Keyboard.Space", Value::Bool(false)),
		],
	];

	bencher.bench_local(|| {
		for tick in ticks {
			for &(name, value) in tick {
				record(&mut collector, devices, name, value);
			}
			interface.sink.pull(&mut collector, |resolved: &ResolvedAction| {
				// The card roster covers the bottom-left corner; a captured drag follows the pointer out.
				Capture::when(
					resolved.handle == Some(card)
						&& resolved.phase == ActionPhase::Started
						&& matches!(resolved.value, Value::Vector2(position) if position.x < -0.5 && position.y < -0.5),
				)
			});
			scene.sink.pull(&mut collector, |resolved| {
				divan::black_box(resolved);
				Capture::Passed
			});
			interface.drain();
			scene.drain();
		}
	});
}
