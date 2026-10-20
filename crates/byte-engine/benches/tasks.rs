//! Run with `cargo bench -p byte-engine --bench tasks`.

use std::{cell::Cell, rc::Rc};

use byte_engine::{
	MediaTime,
	application::Time,
	core::{
		channel::{Channel as _, DefaultChannel},
		message_bus::MessageScope,
	},
	gameplay::TaskRuntime,
};
use divan::{Bencher, counter::ItemsCount};

/// One task per entity, the scale the runtime is meant for.
const TASK_COUNT: usize = 4096;

fn main() {
	divan::main();
}

/// Starts a tick the way an application does, with a fixed 16 ms step.
fn advance(runtime: &mut TaskRuntime<()>) {
	runtime.advance(Time::new(MediaTime::ZERO, MediaTime::from_millis(16)));
}

/// Measures one tick of tasks that each wait for the next tick and add its delta to a total.
#[divan::bench(sample_count = 20, sample_size = 8)]
fn tick_waits(bencher: Bencher) {
	let world = MessageScope::standalone("tasks");
	let mut runtime = TaskRuntime::new(&world);
	let total = Rc::new(Cell::new(MediaTime::ZERO));
	for _ in 0..TASK_COUNT {
		let tasks = runtime.tasks().clone();
		let total = Rc::clone(&total);
		runtime.tasks().spawn(async move {
			loop {
				let delta = tasks.tick().await;
				total.set(total.get() + delta);
			}
		});
	}
	runtime.poll(&mut ());

	bencher.counter(ItemsCount::new(TASK_COUNT)).bench_local(|| {
		advance(&mut runtime);
		runtime.poll(&mut ());
	});
	divan::black_box(total.get());
}

/// Measures one tick in which the host sends one message on a channel every task awaits through its own listener.
///
/// Every send wakes every waiting listener, so this is the fan-out cost of one broadcast.
#[divan::bench(sample_count = 20, sample_size = 8)]
fn listener_waits(bencher: Bencher) {
	let world = MessageScope::standalone("tasks");
	let mut runtime = TaskRuntime::new(&world);
	let total = Rc::new(Cell::new(0u64));
	let channel = DefaultChannel::<u64>::new();
	for _ in 0..TASK_COUNT {
		let mut messages = channel.listener();
		let total = Rc::clone(&total);
		runtime.tasks().spawn(async move {
			loop {
				let value = messages.recv().await;
				total.set(total.get() + value);
			}
		});
	}
	runtime.poll(&mut ());

	bencher.counter(ItemsCount::new(TASK_COUNT)).bench_local(|| {
		advance(&mut runtime);
		channel.send(1);
		runtime.poll(&mut ());
	});
	assert!(total.get() > 0, "The tasks must receive the message the host sends");
}
