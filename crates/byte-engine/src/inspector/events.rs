//! Inspector events: what changed in the application since a client's cursor.
//!
//! Clients long-poll [`DefaultInspector::wait_for_events`](super::DefaultInspector::wait_for_events) with the
//! [`EventCursor`] of their previous batch. Events are read from stores that already keep their history, the
//! [`Metrics`] tick ring and the [`Configuration`] update list, so the inspector keeps no queue or state per client.
//! The application loop raises the [`EventSignal`] after every tick to wake waiting requests.
//! The [inspector reference](/docs/reference/inspector#events) specifies the events and their rules.

use std::{
	fmt,
	str::FromStr,
	sync::{
		Condvar, Mutex,
		atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
	},
	time::Instant,
};

use serde::{Serialize, Serializer};

use crate::{
	configuration::{Configuration, ConfigurationEvent},
	metrics::{Metrics, Sample},
};

/// The longest a request may wait for events.
pub(crate) const MAX_EVENT_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// The number of requests that may wait for events at once, so waiting clients leave the transport's connection
/// threads free for other requests.
pub(crate) const MAX_EVENT_WAITERS: usize = 4;

/// The `EventSignal` struct wakes inspector requests that wait for events when the application loop has news.
///
/// The graphics application calls [`Self::notify`] after every tick and [`Self::close`] when it closes. Raising it
/// costs one atomic increment and one load while no request waits.
pub(crate) struct EventSignal {
	/// Counts notifications, so a waiter can tell whether one arrived since it last looked.
	generation: AtomicU64,
	closed: AtomicBool,
	/// The requests blocked in [`Self::wait`], so [`Self::notify`] only takes the lock when one exists.
	sleepers: AtomicUsize,
	/// The requests holding one of the [`MAX_EVENT_WAITERS`] slots.
	waiters: AtomicUsize,
	lock: Mutex<()>,
	condvar: Condvar,
}

impl EventSignal {
	pub(crate) fn new() -> Self {
		Self {
			generation: AtomicU64::new(0),
			closed: AtomicBool::new(false),
			sleepers: AtomicUsize::new(0),
			waiters: AtomicUsize::new(0),
			lock: Mutex::new(()),
			condvar: Condvar::new(),
		}
	}

	/// Wakes every waiting request so it reads what changed.
	pub(crate) fn notify(&self) {
		// Sequentially consistent with the waiter's increment and load in `wait`: either the waiter sees the new
		// generation, or this load sees the waiter and notifies after it released the lock into the condvar.
		self.generation.fetch_add(1, Ordering::SeqCst);
		if self.sleepers.load(Ordering::SeqCst) > 0 {
			let _guard = self.lock.lock().unwrap_or_else(|poison| poison.into_inner());
			self.condvar.notify_all();
		}
	}

	/// Marks the application as closing and wakes every waiting request; every later request reports it.
	pub(crate) fn close(&self) {
		self.closed.store(true, Ordering::SeqCst);
		self.notify();
	}

	fn generation(&self) -> u64 {
		self.generation.load(Ordering::SeqCst)
	}

	fn is_closed(&self) -> bool {
		self.closed.load(Ordering::SeqCst)
	}

	/// Blocks until a notification after `seen` arrives or `deadline` passes.
	fn wait(&self, seen: u64, deadline: Instant) {
		let mut guard = self.lock.lock().unwrap_or_else(|poison| poison.into_inner());
		self.sleepers.fetch_add(1, Ordering::SeqCst);
		while self.generation.load(Ordering::SeqCst) == seen {
			let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
				break;
			};
			guard = self
				.condvar
				.wait_timeout(guard, remaining)
				.unwrap_or_else(|poison| poison.into_inner())
				.0;
		}
		self.sleepers.fetch_sub(1, Ordering::SeqCst);
	}

	/// Takes a waiting slot, or returns `None` when [`MAX_EVENT_WAITERS`] requests already hold one.
	fn try_slot(&self) -> Option<WaitSlot<'_>> {
		let previous = self.waiters.fetch_add(1, Ordering::SeqCst);
		if previous >= MAX_EVENT_WAITERS {
			self.waiters.fetch_sub(1, Ordering::SeqCst);
			return None;
		}
		Some(WaitSlot(&self.waiters))
	}
}

/// Releases a waiting slot when the request that took it returns.
struct WaitSlot<'a>(&'a AtomicUsize);

impl Drop for WaitSlot<'_> {
	fn drop(&mut self) {
		self.0.fetch_sub(1, Ordering::SeqCst);
	}
}

/// The `EventCursor` struct marks how far a client has read, so the next request returns only later events.
///
/// It serializes as an opaque string, which [`FromStr`] parses back.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EventCursor {
	/// The next tick to report.
	tick: u64,
	/// The completed renderer frames already reported.
	frames: u64,
	/// The [`Configuration::changes_since`] revision already reported.
	configuration: u64,
}

impl fmt::Display for EventCursor {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}-{}-{}", self.tick, self.frames, self.configuration)
	}
}

impl FromStr for EventCursor {
	type Err = ();

	fn from_str(cursor: &str) -> Result<Self, ()> {
		let mut parts = cursor.split('-').map(|part| part.parse::<u64>().map_err(|_| ()));
		let cursor = Self {
			tick: parts.next().ok_or(())??,
			frames: parts.next().ok_or(())??,
			configuration: parts.next().ok_or(())??,
		};
		match parts.next() {
			None => Ok(cursor),
			Some(_) => Err(()),
		}
	}
}

impl Serialize for EventCursor {
	fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
		serializer.collect_str(self)
	}
}

/// The `EventFilter` struct selects the optional events a client wants; `lagged` and `closing` are always sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct EventFilter {
	tick: bool,
	gpu: bool,
	configuration: bool,
}

impl Default for EventFilter {
	fn default() -> Self {
		Self {
			tick: true,
			gpu: true,
			configuration: true,
		}
	}
}

impl FromStr for EventFilter {
	type Err = ();

	/// Parses a comma-separated list of `tick`, `gpu`, and `configuration`, rejecting any other name.
	fn from_str(names: &str) -> Result<Self, ()> {
		let mut filter = Self {
			tick: false,
			gpu: false,
			configuration: false,
		};
		for name in names.split(',') {
			match name {
				"tick" => filter.tick = true,
				"gpu" => filter.gpu = true,
				"configuration" => filter.configuration = true,
				_ => return Err(()),
			}
		}
		Ok(filter)
	}
}

/// The `Event` enum is one change a client learns about, serialized as `{"event": <name>, "data": <data>}`.
#[derive(Serialize)]
#[serde(tag = "event", content = "data", rename_all = "kebab-case")]
pub(crate) enum Event {
	/// A tick ended.
	Tick(Sample),
	/// The frame an already reported tick submitted completed, so its row now has GPU times.
	Gpu(Sample),
	/// A configuration update was requested or its state changed.
	Configuration(ConfigurationEvent),
	/// Ticks left the ring before the client read them.
	Lagged { first: u64, missed: u64 },
	/// The application is closing.
	Closing,
}

/// The `EventBatch` struct is one response: the events after the request's cursor and the cursor to pass next.
#[derive(Serialize)]
pub(crate) struct EventBatch {
	pub(crate) cursor: EventCursor,
	pub(crate) events: Vec<Event>,
}

/// The `EventWaitError` enum reports why a request for events was refused.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum EventWaitError {
	/// [`MAX_EVENT_WAITERS`] requests already wait.
	Busy,
}

/// Returns the cursor at the current tick, before any event, with the configuration revision at zero so the first
/// batch carries every retained configuration update.
fn start_cursor(metrics: &Metrics) -> EventCursor {
	let now = metrics.updates(u64::MAX, u64::MAX);
	EventCursor {
		tick: now.next_tick,
		frames: now.completed_frames,
		configuration: 0,
	}
}

/// Appends the events after `cursor` to `events`, in the order the reference specifies, and returns the cursor
/// past them. Filtered events are skipped, but the cursor still moves past them.
fn collect(
	metrics: &Metrics,
	configuration: &Configuration,
	closed: bool,
	cursor: EventCursor,
	filter: EventFilter,
	events: &mut Vec<Event>,
) -> EventCursor {
	let ticks = metrics.updates(cursor.tick, cursor.frames);
	if let Some(first) = ticks.oldest.filter(|&oldest| oldest > cursor.tick) {
		events.push(Event::Lagged {
			first,
			missed: first - cursor.tick,
		});
	}
	if filter.gpu {
		events.extend(ticks.completed.into_iter().map(Event::Gpu));
	}
	if filter.tick {
		events.extend(ticks.ticks.into_iter().map(Event::Tick));
	}
	let (changes, revision) = configuration.changes_since(cursor.configuration);
	if filter.configuration {
		events.extend(changes.into_iter().map(Event::Configuration));
	}
	if closed {
		events.push(Event::Closing);
	}
	EventCursor {
		tick: ticks.next_tick.max(cursor.tick),
		frames: ticks.completed_frames,
		configuration: revision,
	}
}

/// Returns the events after `after`, waiting up to `wait` for one, or at once with a start cursor when `after` is
/// `None`. Waiting takes one of [`MAX_EVENT_WAITERS`] slots.
pub(crate) fn wait_for_events(
	metrics: &Metrics,
	configuration: &Configuration,
	signal: &EventSignal,
	after: Option<EventCursor>,
	filter: EventFilter,
	wait: std::time::Duration,
) -> Result<EventBatch, EventWaitError> {
	let mut events = Vec::new();
	let Some(mut cursor) = after else {
		let cursor = collect(
			metrics,
			configuration,
			signal.is_closed(),
			start_cursor(metrics),
			filter,
			&mut events,
		);
		return Ok(EventBatch { cursor, events });
	};
	let deadline = Instant::now() + wait.min(MAX_EVENT_WAIT);
	let mut slot = None;
	loop {
		// Read the generation before the stores, so a notification raised while collecting ends the next wait.
		let generation = signal.generation();
		cursor = collect(metrics, configuration, signal.is_closed(), cursor, filter, &mut events);
		if !events.is_empty() || Instant::now() >= deadline {
			return Ok(EventBatch { cursor, events });
		}
		if slot.is_none() {
			slot = Some(signal.try_slot().ok_or(EventWaitError::Busy)?);
		}
		signal.wait(generation, deadline);
	}
}
