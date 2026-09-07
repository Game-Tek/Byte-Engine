//! Compare broadcast and layered input with `cargo bench -p byte-engine --bench input --no-default-features`.

#![feature(allocator_api)]

use std::alloc::{Allocator, Global};

use byte_engine::{
	core::{
		channel::DefaultChannel,
		factory::Factory,
		listener::{DefaultListener, Listener},
	},
	input::{
		Action, ActionBindingDescription, ActionEvent, ActionProcessor, Axis2, Consumption, InputEvents, InputManager,
		ResolvedAction, SeatHandle, TickPolicy, TriggerReference, Types, Value, utils,
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

/// Measures one tick of the convenience path, which broadcasts every action.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn broadcast(bencher: divan::Bencher) {
	broadcast_in(bencher, Global);
}

/// Measures the same input workload using a retained arena.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn broadcast_arena(bencher: divan::Bencher) {
	let arena = bumpalo::Bump::new();
	broadcast_in(bencher, &arena);
}

/// Runs the workload with the supplied allocator for all retained input storage.
fn broadcast_in<A: Allocator + Clone>(bencher: divan::Bencher, allocator: A) {
	let actions = Factory::new();
	let channel = DefaultChannel::new();
	let mut listener: DefaultListener<ActionEvent> = channel.listener();
	let mut input = InputManager::new_in(actions.listener(), channel, allocator);
	let class = utils::register_mouse_device_class(&mut input);
	let mouse = input.create_device(&class);
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");
	actions.create(Action::new(&[binding], Types::Vector2));
	actions.create(Action::new(&[binding], Types::Vector2));
	input.update();

	bencher.bench_local(|| {
		for (name, value) in CLICK {
			input.record_trigger_value_for_device(
				SeatHandle::stub(),
				mouse,
				TriggerReference::Name(name),
				divan::black_box(value),
			);
		}
		input.update();
		while let Some(event) = listener.read() {
			divan::black_box(event);
		}
	});
}

/// Measures one tick of two layers competing for the same click.
///
/// The first layer consumes clicks on its own half of the window, so the second
/// layer only resolves the ones it left.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn layered(bencher: divan::Bencher) {
	layered_in(bencher, Global);
}

/// Measures the same input workload using a retained arena.
#[divan::bench(sample_count = 100, sample_size = 1000)]
fn layered_arena(bencher: divan::Bencher) {
	let arena = bumpalo::Bump::new();
	layered_in(bencher, &arena);
}

/// Runs the workload with the supplied allocator for all retained input storage.
fn layered_in<A: Allocator + Clone>(bencher: divan::Bencher, allocator: A) {
	let mut events = InputEvents::new_in(allocator.clone());
	let class = utils::register_mouse_device_class(&mut events);
	let mouse = events.create_device(&class);
	let binding = ActionBindingDescription::new("Mouse.Position").triggered_by("Mouse.LeftButton");

	let mut layers = [(); 2].map(|()| {
		let actions = Factory::new();
		let channel = DefaultChannel::new();
		let listener: DefaultListener<ActionEvent> = channel.listener();
		let processor =
			ActionProcessor::new_in(events.add_consumer(), channel, allocator.clone()).with_declarations(actions.listener());
		actions.create(Action::new(&[binding], Types::Vector2));

		(actions, processor, listener)
	});

	bencher.bench_local(|| {
		for (name, value) in CLICK {
			events.record(
				SeatHandle::stub(),
				mouse,
				TriggerReference::Name(name),
				divan::black_box(value),
			);
		}
		for (index, (_, processor, _)) in layers.iter_mut().enumerate() {
			processor.process(&mut events, |resolved: &ResolvedAction| {
				match resolved.value {
					// Only the first layer claims the left half of the window.
					Value::Vector2(position) if index == 0 && position.x < 0.0 => Consumption::Consumed,
					_ => Consumption::Ignored,
				}
			});
		}
		events.end_tick();
		for (_, _, listener) in &mut layers {
			while let Some(event) = listener.read() {
				divan::black_box(event);
			}
		}
	});
}

/// Measures directional actions through press, held, and release ticks.
#[divan::bench(args = [1, 32], sample_count = 100, sample_size = 1000)]
fn held_directions(bencher: divan::Bencher, action_count: usize) {
	held_directions_in(bencher, action_count, Global);
}

/// Measures the same input workload using a retained arena.
#[divan::bench(args = [1, 32], sample_count = 100, sample_size = 1000)]
fn held_directions_arena(bencher: divan::Bencher, action_count: usize) {
	let arena = bumpalo::Bump::new();
	held_directions_in(bencher, action_count, &arena);
}

/// Runs the workload with the supplied allocator for all retained input storage.
fn held_directions_in<A: Allocator + Clone>(bencher: divan::Bencher, action_count: usize, allocator: A) {
	let mut events = InputEvents::new_in(allocator.clone());
	let class = utils::register_keyboard_device_class(&mut events);
	let keyboard = events.create_device(&class);
	let mut processor = ActionProcessor::new_in(events.add_consumer(), DefaultChannel::new(), allocator);
	for _ in 0..action_count {
		processor.create_action(
			&events,
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
					events.record(
						SeatHandle::stub(),
						keyboard,
						TriggerReference::Name(name),
						Value::Bool(pressed),
					);
				}
			}
			processor.process(&mut events, |resolved| {
				divan::black_box(resolved);
				Consumption::Consumed
			});
			events.end_tick();
		}
	});
}
