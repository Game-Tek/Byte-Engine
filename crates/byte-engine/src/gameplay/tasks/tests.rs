use std::{cell::Cell, rc::Rc};

use utils::r#async::{FutureExt as _, select_biased};

use super::*;

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
	let (world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let switch = Handle::new();
	runtime.tasks().spawn(async move {
		// The first loop's tick wait is dropped when the message wins; the second loop's resolves in the same poll.
		select_biased! {
			_ = tasks.received::<()>(switch) => {}
			_ = async { loop { tasks.tick().await; } }.fuse() => {}
		}
		let delta = tasks.tick().await;
		note(&tasks, format!("switched after {}", delta.as_seconds_f64() * 1000.0)).await;
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 16);
	runtime.tasks().send(switch, ());
	runtime.poll(&mut log);

	assert_eq!(log.0, ["switched after 16"]);
	drop(world);
}

#[test]
fn a_task_selecting_over_messages_and_ticks_counts_every_tick() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let target = Handle::new();
	runtime.tasks().spawn(async move {
		loop {
			select_biased! {
				message = tasks.received::<u32>(target) => note(&tasks, format!("message {message}")).await,
				delta = tasks.tick() => note(&tasks, format!("tick {}", delta.as_seconds_f64() * 1000.0)).await,
			}
		}
	});

	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.tasks().send(target, 1u32);
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
fn messages_reach_waiting_tasks_and_drop_otherwise() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let target = Handle::new();

	assert!(!runtime.tasks().send(target, 1u32), "nobody waits yet");

	runtime.tasks().spawn(async move {
		loop {
			let message = tasks.received::<u32>(target).await;
			note(&tasks, format!("{message}")).await;
		}
	});
	runtime.poll(&mut log);

	// Both arrive before the task is polled again and are read one wait at a time.
	assert!(runtime.tasks().send(target, 2u32));
	assert!(runtime.tasks().send(target, 3u32));
	// Another type or another entity is a different wait.
	assert!(!runtime.tasks().send(target, 4u64));
	assert!(!runtime.tasks().send(Handle::new(), 5u32));
	runtime.poll(&mut log);

	assert_eq!(log.0, ["2", "3"]);
}

#[test]
fn a_dropped_wait_receives_nothing() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks().clone();
	let first = Handle::new();
	let second = Handle::new();
	runtime.tasks().spawn(async move {
		select_biased! {
			_ = tasks.received::<()>(first) => {}
			_ = tasks.received::<()>(second) => {}
		}
		tasks.sleep(MediaTime::from_millis(100)).await;
	});

	runtime.poll(&mut log);
	runtime.tasks().send(first, ());
	runtime.poll(&mut log);

	assert!(!runtime.tasks().send(second, ()), "the losing branch was dropped");
}

#[test]
fn deleting_an_entity_ends_the_tasks_spawned_on_it() {
	let (world, mut runtime) = runtime();
	let mut log = Log::default();
	let owner = Handle::new();
	let dropped = Rc::new(Cell::new(false));

	/// Records when the task's future drops.
	struct Guard(Rc<Cell<bool>>);
	impl Drop for Guard {
		fn drop(&mut self) {
			self.0.set(true);
		}
	}

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
	world.delete(owner);
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
