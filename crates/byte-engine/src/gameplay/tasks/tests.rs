use std::{cell::Cell, rc::Rc};

use utils::r#async::{FutureExt as _, select_biased};

use super::*;
use crate::core::channel::{Channel as _, DefaultChannel};

/// What the test tasks write into while they are polled.
#[derive(Default)]
struct Log(Vec<String>);

fn runtime() -> (MessageScope, TaskRuntime<Log>) {
	let world = MessageScope::standalone("tasks");
	let runtime = TaskRuntime::new(&world);
	(world, runtime)
}

fn advance(runtime: &mut TaskRuntime<Log>, millis: i64) {
	runtime.advance(Time::new(MediaTime::ZERO, MediaTime::from_millis(millis)));
}

fn note(tasks: &Tasks<Log>, line: String) -> impl Future<Output = ()> {
	tasks.with(move |log: &mut Log| log.0.push(line))
}

#[test]
fn tick_resolves_with_the_time_since_the_wait_began() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	runtime.tasks().spawn(async move {
		loop {
			let delta = tasks.tick().await;
			note(&tasks, format!("{}", delta.as_seconds_f64() * 1000.0)).await;
		}
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	// Two ticks pass before the next poll; the wait counts both.
	advance(&mut runtime, 20);
	advance(&mut runtime, 30);
	runtime.poll(&mut log);

	assert_eq!(log.0, ["10", "50"]);
}

#[test]
fn a_tick_wait_dropped_mid_poll_hands_its_ticks_to_the_next_one() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let switch = DefaultChannel::<()>::new();
	let mut switched = switch.listener();
	runtime.tasks().spawn(async move {
		// The first loop's tick wait is dropped when the message wins; the second loop's resolves in the same poll.
		select_biased! {
			_ = switched.recv() => {}
			_ = async { loop { tasks.tick().await; } }.fuse() => {}
		}
		let delta = tasks.tick().await;
		note(&tasks, format!("switched after {}", delta.as_seconds_f64() * 1000.0)).await;
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 16);
	switch.send(());
	runtime.poll(&mut log);

	assert_eq!(log.0, ["switched after 16"]);
}

#[test]
fn a_task_selecting_over_messages_and_ticks_counts_every_tick() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	runtime.tasks().spawn(async move {
		loop {
			select_biased! {
				message = messages.recv() => note(&tasks, format!("message {message}")).await,
				delta = tasks.tick() => note(&tasks, format!("tick {}", delta.as_seconds_f64() * 1000.0)).await,
			}
		}
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	channel.send(1);
	runtime.poll(&mut log);

	assert_eq!(log.0, ["message 1", "tick 10"]);
}

#[test]
fn sleep_waits_for_simulation_time() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	runtime.tasks().spawn(async move {
		tasks.sleep(MediaTime::from_millis(25)).await;
		note(&tasks, format!("woke at {}", tasks.now().as_seconds_f64() * 1000.0)).await;
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	// A paused tick moves no time, so the sleep holds.
	advance(&mut runtime, 0);
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert!(log.0.is_empty());

	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["woke at 30"]);
}

#[test]
fn a_task_awaiting_a_listener_reads_every_message_sent_on_its_channel() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	// Sent before the task first awaits; the listener keeps it.
	channel.send(1);
	runtime.tasks().spawn(async move {
		loop {
			let message = messages.recv().await;
			note(&tasks, format!("{message}")).await;
		}
	});
	runtime.poll(&mut log);

	// Both arrive before the task is polled again and are read one await at a time.
	channel.send(2);
	channel.send(3);
	runtime.poll(&mut log);

	assert_eq!(log.0, ["1", "2", "3"]);
}

#[test]
fn a_listener_dropped_from_a_select_keeps_its_messages() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let first = DefaultChannel::<u32>::new();
	let second = DefaultChannel::<u32>::new();
	let mut firsts = first.listener();
	let mut seconds = second.listener();
	runtime.tasks().spawn(async move {
		select_biased! {
			_ = firsts.recv() => {}
			_ = seconds.recv() => {}
		}
		// The losing branch's message, sent after it was dropped, is still read.
		let message = seconds.recv().await;
		note(&tasks, format!("{message}")).await;
	});

	runtime.poll(&mut log);
	first.send(1);
	runtime.poll(&mut log);
	second.send(2);
	runtime.poll(&mut log);

	assert_eq!(log.0, ["2"]);
}

#[test]
fn a_message_sent_from_another_thread_wakes_the_awaiting_task() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	runtime.tasks().spawn(async move {
		let message = messages.recv().await;
		note(&tasks, format!("{message}")).await;
	});

	runtime.poll(&mut log);
	std::thread::spawn(move || channel.send(7))
		.join()
		.expect("the sending thread finishes");
	runtime.poll(&mut log);

	assert_eq!(log.0, ["7"]);
}

/// Records when the future that owns it drops.
struct Guard(Rc<Cell<bool>>);

impl Drop for Guard {
	fn drop(&mut self) {
		self.0.set(true);
	}
}

#[test]
fn deleting_an_entity_ends_the_tasks_spawned_on_it() {
	let (world, mut runtime) = runtime();
	let mut log = Log::default();
	let owner = Handle::new();
	let dropped = Rc::new(Cell::new(false));

	let tasks = runtime.tasks().clone();
	let guard = Guard(Rc::clone(&dropped));
	runtime.tasks().spawn_on(owner, async move {
		let _guard = guard;
		loop {
			tasks.tick().await;
			note(&tasks, "tick".into()).await;
		}
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	world.channel::<DeleteMessage>().send(DeleteMessage::new(owner));
	advance(&mut runtime, 10);
	runtime.poll(&mut log);

	assert_eq!(log.0, ["tick"]);
	assert!(dropped.get());
}

#[test]
fn tasks_spawned_by_a_task_run_in_the_same_poll() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	runtime.tasks().spawn(async move {
		let inner = tasks.clone();
		tasks.spawn(async move { note(&inner, "child".into()).await });
		note(&tasks, "parent".into()).await;
	});

	runtime.poll(&mut log);

	assert_eq!(log.0, ["parent", "child"]);
}

#[test]
fn foreign_futures_wake_their_task() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let (sender, receiver) = utils::r#async::oneshot::channel::<u32>();
	runtime.tasks().spawn(async move {
		let value = receiver.await.expect("the sender sends before it drops");
		note(&tasks, format!("{value}")).await;
	});

	runtime.poll(&mut log);
	std::thread::spawn(move || sender.send(7).expect("the task still waits"))
		.join()
		.expect("the sending thread finishes");
	runtime.poll(&mut log);

	assert_eq!(log.0, ["7"]);
}

#[test]
fn a_task_deleted_before_its_first_poll_can_reach_the_runtime_as_it_drops() {
	let (world, mut runtime) = runtime();
	let mut log = Log::default();
	let owner = Handle::new();
	let dropped = Rc::new(Cell::new(false));

	/// Reads the runtime's clock as it drops, as a destructor that cleans up through `Tasks` would.
	struct Reader(Tasks<Log>, Rc<Cell<bool>>);
	impl Drop for Reader {
		fn drop(&mut self) {
			let _ = self.0.now();
			self.1.set(true);
		}
	}

	let reader = Reader(runtime.tasks().clone(), Rc::clone(&dropped));
	runtime.tasks().spawn_on(owner, async move {
		let _reader = reader;
	});
	world.channel::<DeleteMessage>().send(DeleteMessage::new(owner));
	advance(&mut runtime, 10);
	runtime.poll(&mut log);

	assert!(dropped.get());
}

#[test]
fn dropping_the_runtime_drops_tasks_it_never_polled() {
	let (_world, runtime) = runtime();
	let dropped = Rc::new(Cell::new(false));
	let tasks = runtime.tasks().clone();
	let guard = Guard(Rc::clone(&dropped));
	runtime.tasks().spawn(async move {
		let _guard = guard;
		tasks.tick().await;
	});

	drop(runtime);

	assert!(dropped.get());
}
