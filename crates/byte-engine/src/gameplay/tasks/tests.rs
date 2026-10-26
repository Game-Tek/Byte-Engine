use std::{cell::Cell, rc::Rc};

use utils::r#async::{FutureExt as _, select_biased};

use super::*;
use crate::core::channel::{Channel as _, DefaultChannel};

/// What the test tasks write into while they are polled.
#[derive(Default)]
struct Log(Vec<String>);

fn runtime() -> (MessageScope, Runtime) {
	let world = MessageScope::standalone("tasks");
	let runtime = Runtime::new(&world);
	(world, runtime)
}

fn advance(runtime: &mut Runtime, millis: i64) {
	runtime.advance(Time::new(MediaTime::ZERO, MediaTime::from_millis(millis)));
}

#[test]
fn separately_requested_handles_share_time_and_borrow_the_polled_state() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	for _ in 0..2 {
		let tasks = runtime.tasks::<Log>();
		runtime.tasks::<Log>().spawn(async move {
			let delta = tasks.tick().await;
			tasks.with(|log| log.0.push(delta.as_seconds_f64().to_string())).await;
		});
	}
	runtime.poll(&mut log);
	advance(&mut runtime, 20);
	runtime.poll(&mut log);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["0.02", "0.02"]);
	assert_eq!(runtime.tasks::<Log>().now(), MediaTime::from_millis(20));
}

fn note(tasks: &Tasks<Log>, line: String) -> impl Future<Output = ()> {
	tasks.with(move |log: &mut Log| log.0.push(line))
}

#[test]
fn and_finishes_each_activity_before_the_next_borrows_shared_state() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	assert_eq!(log.0, ["waiting"]);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["waiting", "walk, follow, shoot"]);
}

#[test]
fn and_checks_events_without_stalling_or_repeating_a_consumed_event() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["follow", "finished"]);
	events.send(());
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(
		log.0,
		["follow", "finished", "follow", "shoot", "finished", "follow", "finished"]
	);
}

#[test]
fn and_guards_keep_task_context_and_await_selected_activities_in_order() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	assert_eq!(log.0, ["guard"]);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["guard", "shoot, shot finished, next activity"]);
}

#[test]
fn always_runs_initially_and_between_branches_with_the_same_mutable_state() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	assert_eq!(log.0, ["always 1"]);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["always 1", "branch 2", "always 3"]);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["always 1", "branch 2", "always 3", "branch 4", "finished"]);
}

#[test]
fn always_runs_after_a_suspended_branch_finishes_and_preserves_raw_select_syntax() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["always", "waiting"]);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["always", "waiting", "branch finished", "always"]);
}

#[test]
fn perform_handles_repeated_updates_and_prioritizes_the_first_ready_wait() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	for _ in 0..2 {
		advance(&mut runtime, 10);
		runtime.poll(&mut log);
	}
	stop.send(());
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["tick", "tick", "stopped"]);
}

#[test]
fn perform_checks_ready_events_in_written_order() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			each _ = std::future::ready(()) => { break; }
			when std::future::ready(()) => {
				note(&tasks, "started".into()).await;
			}
		}
		note(&tasks, "stopped".into()).await;
	});
	runtime.poll(&mut log);
	assert_eq!(log.0, ["stopped"]);
}

#[test]
fn perform_finishes_selected_actions_before_handling_another_event() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let stop = DefaultChannel::<()>::new();
	let mut stopped = stop.listener();
	let dropped = Rc::new(Cell::new(false));
	let observed = Rc::clone(&dropped);
	runtime.spawn(|tasks: Tasks<Log>| async move {
		crate::gameplay::tasks::perform! {
			when stopped.recv() => { break; }
			each _ = std::future::ready(()) => {
				let _guard = Guard(Rc::clone(&observed));
				note(&tasks, "started".into()).await;
				tasks.sleep(MediaTime::from_millis(100)).await;
				note(&tasks, "finished".into()).await;
			}
		}
		note(&tasks, "stopped".into()).await;
	});
	runtime.poll(&mut log);
	assert!(!dropped.get());
	stop.send(());
	runtime.poll(&mut log);
	assert!(!dropped.get());
	advance(&mut runtime, 100);
	runtime.poll(&mut log);
	assert!(dropped.get());
	assert_eq!(log.0, ["started", "finished", "stopped"]);
}

#[test]
fn a_nested_performance_replaces_outer_updates_until_it_ends() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
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
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	events.send(());
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	events.send(());
	runtime.poll(&mut log);
	advance(&mut runtime, 10);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["outer", "paused", "inner", "resumed", "outer"]);
}

#[test]
fn tick_resolves_with_the_time_since_the_wait_began() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	runtime.spawn(|tasks: Tasks<Log>| async move {
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
fn perform_repeats_in_branch_order_and_returns_a_break_value() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks::<Log>();
	runtime.tasks::<Log>().spawn(async move {
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
	runtime.poll(&mut log);
	assert_eq!(log.0, ["6"]);
}

#[test]
fn a_tick_wait_dropped_mid_poll_hands_its_ticks_to_the_next_one() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let tasks = runtime.tasks::<Log>();
	let switch = DefaultChannel::<()>::new();
	let mut switched = switch.listener();
	runtime.tasks::<Log>().spawn(async move {
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
	let tasks = runtime.tasks::<Log>();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	runtime.tasks::<Log>().spawn(async move {
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
	let tasks = runtime.tasks::<Log>();
	runtime.tasks::<Log>().spawn(async move {
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
	let tasks = runtime.tasks::<Log>();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	// Sent before the task first awaits; the listener keeps it.
	channel.send(1);
	runtime.tasks::<Log>().spawn(async move {
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
	let tasks = runtime.tasks::<Log>();
	let first = DefaultChannel::<u32>::new();
	let second = DefaultChannel::<u32>::new();
	let mut firsts = first.listener();
	let mut seconds = second.listener();
	runtime.tasks::<Log>().spawn(async move {
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
	let tasks = runtime.tasks::<Log>();
	let channel = DefaultChannel::<u32>::new();
	let mut messages = channel.listener();
	runtime.tasks::<Log>().spawn(async move {
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

	let guard = Guard(Rc::clone(&dropped));
	runtime.spawn_on(owner, |tasks: Tasks<Log>| async move {
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
	runtime.spawn(|tasks: Tasks<Log>| async move {
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
	let tasks = runtime.tasks::<Log>();
	let (sender, receiver) = utils::r#async::oneshot::channel::<u32>();
	runtime.tasks::<Log>().spawn(async move {
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

	let reader = Reader(runtime.tasks::<Log>(), Rc::clone(&dropped));
	runtime.tasks::<Log>().spawn_on(owner, async move {
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
	let tasks = runtime.tasks::<Log>();
	let guard = Guard(Rc::clone(&dropped));
	runtime.tasks::<Log>().spawn(async move {
		let _guard = guard;
		tasks.tick().await;
	});

	drop(runtime);

	assert!(dropped.get());
}

/// Returns `time` in whole milliseconds.
fn millis(time: MediaTime) -> i64 {
	time.as_ticks() / (MediaTime::TICKS_PER_SECOND / 1000)
}

/// Advances by each of `steps` milliseconds and polls after each one.
fn run(runtime: &mut Runtime, log: &mut Log, steps: &[i64]) {
	for &step in steps {
		advance(runtime, step);
		runtime.poll(log);
	}
}

/// Spawns a task that logs every fire of the interval `make` returns as `"<now>+<since previous fire>"`.
fn log_fires(runtime: &mut Runtime, make: impl FnOnce(&Tasks<Log>) -> Interval<Log> + 'static) {
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut interval = make(&tasks);
		loop {
			let since = interval.next().await;
			note(&tasks, format!("{}+{}", millis(tasks.now()), millis(since))).await;
		}
	});
}

#[test]
fn an_interval_fires_at_anchored_deadlines_with_the_time_since_its_previous_fire() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	log_fires(&mut runtime, |tasks| tasks.every(MediaTime::from_millis(100)));
	runtime.poll(&mut log);

	run(&mut runtime, &mut log, &[30; 10]);

	// Deadlines stay at 100, 200, and 300 ms although every fire lands late.
	assert_eq!(log.0, ["120+120", "210+90", "300+90"]);
}

#[test]
fn an_interval_that_missed_deadlines_fires_once_and_keeps_its_schedule() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	log_fires(&mut runtime, |tasks| tasks.every(MediaTime::from_millis(100)));
	runtime.poll(&mut log);

	run(&mut runtime, &mut log, &[350]);
	runtime.poll(&mut log);
	assert_eq!(log.0, ["350+350"]);

	run(&mut runtime, &mut log, &[40, 10]);
	assert_eq!(log.0, ["350+350", "400+50"]);
}

#[test]
fn an_interval_wait_that_loses_a_select_keeps_its_deadline() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	let messages = DefaultChannel::<()>::new();
	let mut received = messages.listener();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut interval = tasks.every(MediaTime::from_millis(100));
		crate::gameplay::tasks::perform! {
			when received.recv() => {}
			each since = interval.next() => {
				note(&tasks, format!("{}+{}", millis(tasks.now()), millis(since))).await;
			}
		}
	});
	runtime.poll(&mut log);

	// A message wins every selection before the deadline.
	for _ in 0..4 {
		messages.send(());
		run(&mut runtime, &mut log, &[20]);
	}
	assert!(log.0.is_empty());
	run(&mut runtime, &mut log, &[20]);

	assert_eq!(log.0, ["100+100"]);
}

#[test]
fn a_phase_delays_every_deadline() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	log_fires(&mut runtime, |tasks| tasks.every(MediaTime::from_millis(100)));
	log_fires(&mut runtime, |tasks| {
		tasks.every(MediaTime::from_millis(100)).phase(MediaTime::from_millis(50))
	});
	runtime.poll(&mut log);

	run(&mut runtime, &mut log, &[50; 5]);

	assert_eq!(log.0, ["100+100", "150+150", "200+100", "250+100"]);
}

#[test]
fn a_new_period_counts_from_the_latest_passed_deadline() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut interval = tasks.every(MediaTime::from_millis(100));
		loop {
			let since = interval.next().await;
			note(&tasks, format!("{}+{}", millis(tasks.now()), millis(since))).await;
			interval.set_period(MediaTime::from_millis(50));
		}
	});
	runtime.poll(&mut log);

	// The first fire lands at 120 ms for the 100 ms deadline, so the next deadline is 150 ms rather than 170 ms.
	run(&mut runtime, &mut log, &[30; 5]);

	assert_eq!(log.0, ["120+120", "150+30"]);
}

#[test]
fn an_interval_shorter_than_the_step_fires_once_per_advance_that_moves_time() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	log_fires(&mut runtime, |tasks| tasks.every(MediaTime::from_millis(10)));
	runtime.poll(&mut log);

	run(&mut runtime, &mut log, &[30, 0, 30]);

	assert_eq!(log.0, ["30+30", "60+30"]);
}

#[test]
#[should_panic(expected = "interval period")]
fn an_interval_period_must_be_greater_than_zero() {
	let (_world, runtime) = runtime();
	runtime.tasks::<Log>().every(MediaTime::ZERO);
}

#[test]
fn a_period_shortened_after_a_fire_waits_for_the_next_advance() {
	let (_world, mut runtime) = runtime();
	let mut log = Log::default();
	runtime.spawn(|tasks: Tasks<Log>| async move {
		let mut interval = tasks.every(MediaTime::from_millis(100));
		loop {
			let since = interval.next().await;
			note(&tasks, format!("{}+{}", millis(tasks.now()), millis(since))).await;
			interval.set_period(MediaTime::from_millis(10));
		}
	});
	runtime.poll(&mut log);

	// The 110 ms deadline has passed when the first fire lands at 120 ms, yet the interval fires once in that advance.
	run(&mut runtime, &mut log, &[120]);
	assert_eq!(log.0, ["120+120"]);

	run(&mut runtime, &mut log, &[0, 30]);
	assert_eq!(log.0, ["120+120", "150+30"]);
}
