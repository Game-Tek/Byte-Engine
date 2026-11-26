#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]
use std::sync::{
	Arc,
	atomic::{AtomicBool, Ordering::Relaxed},
};

use utils::r#async::{FutureExt as _, select_biased};

use super::*;
use crate::core::channel::{Channel as _, DefaultChannel};

/// What the test tasks write into while they are polled.
#[derive(Default)]
struct Log(Vec<String>);

fn runtime() -> (MessageScope, Runtime<Log>) {
	let world = MessageScope::standalone("tasks");
	let runtime = Runtime::new(&world, Log::default());
	(world, runtime)
}

fn advance(runtime: &mut Runtime<Log>, millis: i64) {
	runtime.advance(Time::new(MediaTime::ZERO, MediaTime::from_millis(millis)));
}

#[test]
fn tasks_share_time_and_borrow_the_owned_context() {
	let (_world, mut runtime) = runtime();
	for _ in 0..2 {
		runtime.spawn(|tasks| async move {
			let delta = tasks.tick().await;
			tasks.with(|log| log.0.push(delta.as_seconds_f64().to_string())).await;
		});
	}
	runtime.poll();
	advance(&mut runtime, 20);
	runtime.poll();
	runtime.poll();
	assert_eq!(runtime.context().0, ["0.02", "0.02"]);
	assert_eq!(runtime.now(), MediaTime::from_millis(20));
}

fn note(tasks: &Tasks<Log>, line: String) -> impl Future<Output = ()> {
	tasks.with(move |log: &mut Log| log.0.push(line))
}

#[test]
fn and_finishes_each_activity_before_the_next_borrows_shared_state() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut order = Vec::new();
		crate::gameplay::tasks::and! {
			async {
				note(&tasks, "waiting".into()).await;
				tasks.tick().await;
				order.push("walk");
			};
			async { order.push("follow"); };
			async {
				order.push("shoot");
				note(&tasks, order.join(", ")).await;
			};
		}
	});
	runtime.poll();
	assert_eq!(runtime.context().0, ["waiting"]);
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["waiting", "walk, follow, shoot"]);
}

#[test]
fn and_checks_events_without_stalling_or_repeating_a_consumed_event() {
	let (_world, mut runtime) = runtime();
	let events = DefaultChannel::<()>::new();
	let mut shooting = events.listener();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		for _ in 0..3 {
			tasks.tick().await;
			crate::and! {
				note(&tasks, "follow".into());
				when shooting.recv() => { note(&tasks, "shoot".into()); }
				note(&tasks, "finished".into());
			}
		}
	});
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["follow", "finished"]);
	events.send(());
	advance(&mut runtime, 10);
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(
		runtime.context().0,
		["follow", "finished", "follow", "shoot", "finished", "follow", "finished"]
	);
}

#[test]
fn and_guards_keep_task_context_and_await_selected_activities_in_order() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut order = Vec::new();
		crate::and! {
			when tasks.with(|log| log.0.push("guard".into())) => {
				async {
					order.push("shoot");
					tasks.tick().await;
				};
				async { order.push("shot finished"); };
			}
			async { order.push("next activity"); };
			note(&tasks, order.join(", "));
		}
	});
	runtime.poll();
	assert_eq!(runtime.context().0, ["guard"]);
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["guard", "shoot, shot finished, next activity"]);
}

#[test]
fn always_runs_initially_and_between_branches_with_the_same_mutable_state() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut value = 0;
		crate::perform! {
			each _ = std::future::ready(()) => {
				assert_eq!(value % 2, 1, "the shared step precedes selection");
				value += 1;
				note(&tasks, format!("branch {value}")).await;
				if value == 4 { break; }
			}
			always {
				value += 1;
				note(&tasks, format!("always {value}")).await;
				tasks.tick().await;
			}
		}
		note(&tasks, "finished".into()).await;
	});
	runtime.poll();
	assert_eq!(runtime.context().0, ["always 1"]);
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["always 1", "branch 2", "always 3"]);
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(
		runtime.context().0,
		["always 1", "branch 2", "always 3", "branch 4", "finished"]
	);
}

#[test]
fn always_runs_after_a_suspended_branch_finishes_and_preserves_raw_select_syntax() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut finished = false;
		crate::perform! {
			always { note(&tasks, "always".into()).await; }
			_ = std::future::ready(()).fuse() => {
				if finished { break; }
				note(&tasks, "waiting".into()).await;
				tasks.tick().await;
				finished = true;
				note(&tasks, "branch finished".into()).await;
			}
		}
	});
	runtime.poll();
	runtime.poll();
	assert_eq!(runtime.context().0, ["always", "waiting"]);
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["always", "waiting", "branch finished", "always"]);
}

#[test]
fn perform_handles_repeated_updates_and_prioritizes_the_first_ready_wait() {
	let (_world, mut runtime) = runtime();
	let stop = DefaultChannel::<()>::new();
	let mut stopped = stop.listener();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			when stopped.recv() => { break; }
			each delta = tasks.tick() => {
				assert_eq!(delta, MediaTime::from_millis(10));
				note(&tasks, "tick".into()).await;
			}
		}
		note(&tasks, "stopped".into()).await;
	});
	runtime.poll();
	for _ in 0..2 {
		advance(&mut runtime, 10);
		runtime.poll();
	}
	stop.send(());
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["tick", "tick", "stopped"]);
}

#[test]
fn perform_checks_ready_events_in_written_order() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			each _ = std::future::ready(()) => { break; }
			when std::future::ready(()) => {
				note(&tasks, "started".into()).await;
			}
		}
		note(&tasks, "stopped".into()).await;
	});
	runtime.poll();
	assert_eq!(runtime.context().0, ["stopped"]);
}

#[test]
fn perform_finishes_selected_actions_before_handling_another_event() {
	let (_world, mut runtime) = runtime();
	let stop = DefaultChannel::<()>::new();
	let mut stopped = stop.listener();
	let dropped = Arc::new(AtomicBool::new(false));
	let observed = Arc::clone(&dropped);
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			when stopped.recv() => { break; }
			each _ = std::future::ready(()) => {
				let _guard = Guard(Arc::clone(&observed));
				note(&tasks, "started".into()).await;
				tasks.sleep(MediaTime::from_millis(100)).await;
				note(&tasks, "finished".into()).await;
			}
		}
		note(&tasks, "stopped".into()).await;
	});
	runtime.poll();
	assert!(!dropped.load(Relaxed));
	stop.send(());
	runtime.poll();
	assert!(!dropped.load(Relaxed));
	advance(&mut runtime, 100);
	runtime.poll();
	assert!(dropped.load(Relaxed));
	assert_eq!(runtime.context().0, ["started", "finished", "stopped"]);
}

#[test]
fn a_nested_performance_replaces_outer_updates_until_it_ends() {
	let (_world, mut runtime) = runtime();
	let events = DefaultChannel::<()>::new();
	let mut changed = events.listener();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			when changed.recv() => {
				note(&tasks, "paused".into()).await;
				crate::gameplay::tasks::perform! {
					when changed.recv() => { break; }
					each _ = tasks.tick() => {
						note(&tasks, "inner".into()).await;
					}
				}
				note(&tasks, "resumed".into()).await;
			}
			each _ = tasks.tick() => {
				note(&tasks, "outer".into()).await;
			}
		}
	});
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	events.send(());
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	events.send(());
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["outer", "paused", "inner", "resumed", "outer"]);
}

#[test]
fn tick_resolves_with_the_time_since_the_wait_began() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		loop {
			let delta = tasks.tick().await;
			note(&tasks, format!("{}", delta.as_seconds_f64() * 1000.0)).await;
		}
	});

	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	// Two ticks pass before the next poll; the wait counts both.
	advance(&mut runtime, 20);
	advance(&mut runtime, 30);
	runtime.poll();

	assert_eq!(runtime.context().0, ["10", "50"]);
}

#[test]
fn perform_repeats_in_branch_order_and_returns_a_break_value() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks| async move {
		let mut iterations = 0;
		let result = crate::gameplay::tasks::perform! {
			value = std::future::ready(3).fuse() => {
				iterations += value;
				if iterations == 6 {
					break iterations;
				}
			},
			_ = std::future::ready(()).fuse() => panic!("the first ready branch must win"),
		};
		note(&tasks, result.to_string()).await;
	});
	runtime.poll();
	assert_eq!(runtime.context().0, ["6"]);
}

#[test]
fn a_tick_wait_dropped_mid_poll_hands_its_ticks_to_the_next_one() {
	let (_world, mut runtime) = runtime();
	let switch = DefaultChannel::<()>::new();
	let mut switched = switch.listener();
	runtime.spawn(|tasks| async move {
		// The first loop's tick wait is dropped when the message wins; the second loop's resolves in the same poll.
		select_biased! {
			_ = switched.recv() => {}
			_ = async { loop { tasks.tick().await; } }.fuse() => {}
		}
		let delta = tasks.tick().await;
		note(&tasks, format!("switched after {}", delta.as_seconds_f64() * 1000.0)).await;
	});

	runtime.poll();
	advance(&mut runtime, 16);
	switch.send(());
	runtime.poll();

	assert_eq!(runtime.context().0, ["switched after 16"]);
}

#[test]
fn a_task_selecting_over_messages_and_ticks_counts_every_tick() {
	let (_world, mut runtime) = runtime();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	runtime.spawn(|tasks| async move {
		loop {
			select_biased! {
				message = messages.recv() => note(&tasks, format!("message {message}")).await,
				delta = tasks.tick() => note(&tasks, format!("tick {}", delta.as_seconds_f64() * 1000.0)).await,
			}
		}
	});

	runtime.poll();
	advance(&mut runtime, 10);
	channel.send(1);
	runtime.poll();

	assert_eq!(runtime.context().0, ["message 1", "tick 10"]);
}

#[test]
fn sleep_waits_for_simulation_time() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks| async move {
		tasks.sleep(MediaTime::from_millis(25)).await;
		note(&tasks, format!("woke at {}", tasks.now().await.as_seconds_f64() * 1000.0)).await;
	});

	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	// A paused tick moves no time, so the sleep holds.
	advance(&mut runtime, 0);
	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	assert!(runtime.context().0.is_empty());

	advance(&mut runtime, 10);
	runtime.poll();
	assert_eq!(runtime.context().0, ["woke at 30"]);
}

#[test]
fn a_task_awaiting_a_listener_reads_every_message_sent_on_its_channel() {
	let (_world, mut runtime) = runtime();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	// Sent before the task first awaits; the listener keeps it.
	channel.send(1);
	runtime.spawn(|tasks| async move {
		loop {
			let message = messages.recv().await;
			note(&tasks, format!("{message}")).await;
		}
	});
	runtime.poll();

	// Both arrive before the task is polled again and are read one await at a time.
	channel.send(2);
	channel.send(3);
	runtime.poll();

	assert_eq!(runtime.context().0, ["1", "2", "3"]);
}

#[test]
fn a_listener_dropped_from_a_select_keeps_its_messages() {
	let (_world, mut runtime) = runtime();
	let first = DefaultChannel::<u32>::new();
	let second = DefaultChannel::<u32>::new();
	let mut firsts = first.listener();
	let mut seconds = second.listener();
	runtime.spawn(|tasks| async move {
		select_biased! {
			_ = firsts.recv() => {}
			_ = seconds.recv() => {}
		}
		// The losing branch's message, sent after it was dropped, is still read.
		let message = seconds.recv().await;
		note(&tasks, format!("{message}")).await;
	});

	runtime.poll();
	first.send(1);
	runtime.poll();
	second.send(2);
	runtime.poll();

	assert_eq!(runtime.context().0, ["2"]);
}

#[test]
fn a_message_or_a_foreign_future_completed_on_another_thread_wakes_the_awaiting_task() {
	let (_world, mut runtime) = runtime();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	let (sender, receiver) = utils::r#async::oneshot::channel::<u32>();
	runtime.spawn(|tasks| async move {
		let message = messages.recv().await;
		note(&tasks, format!("{message}")).await;
		let value = receiver.await.expect("the sender sends before it drops");
		note(&tasks, format!("{value}")).await;
	});

	runtime.poll();
	std::thread::spawn(move || channel.send(7))
		.join()
		.expect("the sending thread finishes");
	runtime.poll();
	std::thread::spawn(move || sender.send(8).expect("the task still waits"))
		.join()
		.expect("the sending thread finishes");
	runtime.poll();

	assert_eq!(runtime.context().0, ["7", "8"]);
}

/// Records when the future that owns it drops.
struct Guard(Arc<AtomicBool>);

impl Drop for Guard {
	fn drop(&mut self) {
		self.0.store(true, Relaxed);
	}
}

#[test]
fn deleting_an_entity_ends_the_tasks_spawned_on_it() {
	let (world, mut runtime) = runtime();
	let owner = Handle::new();
	let dropped = Arc::new(AtomicBool::new(false));

	let guard = Guard(Arc::clone(&dropped));
	runtime.spawn_on(owner, |tasks: Tasks<Log>| async move {
		let _guard = guard;
		loop {
			tasks.tick().await;
			note(&tasks, "tick".into()).await;
		}
	});

	runtime.poll();
	advance(&mut runtime, 10);
	runtime.poll();
	world.channel::<DeleteMessage>().send(DeleteMessage::new(owner));
	advance(&mut runtime, 10);
	runtime.poll();

	assert_eq!(runtime.context().0, ["tick"]);
	assert!(dropped.load(Relaxed));
}

#[test]
fn tasks_spawned_by_a_task_run_in_the_same_poll() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		tasks.spawn(async move { note(&tasks, "child".into()).await }).await;
		note(&tasks, "parent".into()).await;
	});

	runtime.poll();

	assert_eq!(runtime.context().0, ["parent", "child"]);
}

#[test]
fn dropping_the_runtime_drops_tasks_it_never_polled() {
	let (_world, mut runtime) = runtime();
	let dropped = Arc::new(AtomicBool::new(false));
	let guard = Guard(Arc::clone(&dropped));
	runtime.spawn(|tasks| async move {
		let _guard = guard;
		tasks.tick().await;
	});

	drop(runtime);

	assert!(dropped.load(Relaxed));
}

/// Returns `time` in whole milliseconds.
fn millis(time: MediaTime) -> i64 {
	time.as_ticks() / (MediaTime::TICKS_PER_SECOND / 1000)
}

/// Advances by each of `steps` milliseconds and polls after each one.
fn run(runtime: &mut Runtime<Log>, steps: &[i64]) {
	for &step in steps {
		advance(runtime, step);
		runtime.poll();
	}
}

/// Logs every fire of an interval of `period` with `phase`, as `"<now>+<since previous fire>"`, while simulation time
/// advances by each of `steps` milliseconds; after each fire the period changes to `new_period` when one is given.
fn fires(period: i64, phase: i64, new_period: Option<i64>, steps: &[i64]) -> Vec<String> {
	let (_world, mut runtime) = runtime();
	runtime.spawn(move |tasks: Tasks<Log>| async move {
		let mut interval = tasks
			.every(MediaTime::from_millis(period))
			.await
			.phase(MediaTime::from_millis(phase));
		loop {
			let since = interval.next().await;
			note(&tasks, format!("{}+{}", millis(tasks.now().await), millis(since))).await;
			if let Some(new_period) = new_period {
				interval.set_period(MediaTime::from_millis(new_period));
			}
		}
	});
	runtime.poll();
	run(&mut runtime, steps);
	std::mem::take(&mut runtime.context_mut().0)
}

#[test]
fn an_interval_fires_once_per_advance_that_passes_a_deadline() {
	let cases: [(i64, i64, Option<i64>, &[i64], &[&str]); 6] = [
		// Deadlines stay at 100, 200, and 300 ms although every fire lands late.
		(100, 0, None, &[30; 10], &["120+120", "210+90", "300+90"]),
		// Missed deadlines fire once, and the schedule keeps its anchor.
		(100, 0, None, &[350, 0, 40, 10], &["350+350", "400+50"]),
		// A period shorter than the step fires once per advance that moves time.
		(10, 0, None, &[30, 0, 30], &["30+30", "60+30"]),
		// A phase delays every deadline.
		(100, 50, None, &[50; 5], &["150+150", "250+100"]),
		// The fire lands at 120 ms for the 100 ms deadline, so the new period counts from 100 ms, not 120 ms.
		(100, 0, Some(50), &[30; 5], &["120+120", "150+30"]),
		// The shortened period's 110 ms deadline has passed at the first fire, yet it waits for the next advance.
		(100, 0, Some(10), &[120, 0, 30], &["120+120", "150+30"]),
	];
	for (period, phase, new_period, steps, expected) in cases {
		assert_eq!(
			fires(period, phase, new_period, steps),
			expected,
			"period {period} ms, phase {phase} ms, then {new_period:?} ms, steps {steps:?}"
		);
	}
}

#[test]
fn an_interval_wait_that_loses_a_select_keeps_its_deadline() {
	let (_world, mut runtime) = runtime();
	let messages = DefaultChannel::<()>::new();
	let mut received = messages.listener();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut interval = tasks.every(MediaTime::from_millis(100)).await;
		crate::gameplay::tasks::perform! {
			when received.recv() => {}
			each since = interval.next() => {
				note(&tasks, format!("{}+{}", millis(tasks.now().await), millis(since))).await;
			}
		}
	});
	runtime.poll();

	// A message wins every selection before the deadline.
	for _ in 0..4 {
		messages.send(());
		run(&mut runtime, &[20]);
	}
	assert!(runtime.context().0.is_empty());
	run(&mut runtime, &[20]);

	assert_eq!(runtime.context().0, ["100+100"]);
}

#[test]
#[should_panic(expected = "interval period")]
fn an_interval_period_must_be_greater_than_zero() {
	let (_world, mut runtime) = runtime();
	runtime.spawn(|tasks| async move {
		tasks.every(MediaTime::ZERO).await;
	});
	runtime.poll();
}

/// The `Patrol` struct is a kind that attached tasks start for.
#[derive(Clone)]
struct Patrol(&'static str);

#[test]
fn an_attached_task_starts_for_each_entity_that_gets_its_kind_and_ends_with_it() {
	use crate::core::factory::Creator as _;

	let (world, mut runtime) = runtime();
	// An entity created before `attach` starts no task.
	world.create(Patrol("early"));
	runtime.attach(|tasks: Tasks<Log>, _, patrol: Patrol| async move {
		loop {
			note(&tasks, patrol.0.to_string()).await;
			tasks.tick().await;
		}
	});
	let guard = world.create(Patrol("guard")).handle();
	// A replaced component starts no second task.
	world.factory::<Patrol>().derive(guard, Patrol("again"));

	runtime.poll();
	world.channel::<DeleteMessage>().send(DeleteMessage::new(guard));
	advance(&mut runtime, 10);
	runtime.poll();

	assert_eq!(runtime.context().0, ["guard"], "the task ends when its entity is deleted");
}
