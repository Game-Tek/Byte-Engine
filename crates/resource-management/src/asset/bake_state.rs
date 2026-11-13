//! What the asset manager reports about its bakes, for tools that show them while an application runs.
//!
//! Give an [`AssetManager`](super::manager::AssetManager) a [`BakeReporter`] with
//! [`AssetManager::set_bake_reporter`](super::manager::AssetManager::set_bake_reporter). The manager then reports its
//! [`BakeState`] each time a bake starts or ends. The engine turns these reports into the `bakes` debug data stream;
//! see the [bake stream reference](/docs/reference/resource-management#bake-stream).

use std::{
	collections::VecDeque,
	sync::Arc,
	time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use utils::sync::Mutex;

/// The most finished bakes a [`BakeState`] keeps.
pub const FINISHED_KEPT: usize = 16;

/// The `BakeState` struct is what a tool shows about an asset manager's bakes: which run now, how many ended, and how
/// the latest ones went.
///
/// A bake is one run of an asset's handler. A request that finds its resource current bakes nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BakeState {
	/// The bakes running, oldest first.
	pub baking: Vec<BakingAsset>,
	/// The bakes that stored their resource.
	pub baked: u64,
	/// The bakes that ended without storing their resource.
	pub failed: u64,
	/// The latest [`FINISHED_KEPT`] bakes to end, newest first.
	pub finished: Vec<FinishedBake>,
}

/// The `BakingAsset` struct is one bake in progress.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BakingAsset {
	/// The ID of the resource being baked.
	pub id: String,
	/// The milliseconds since the bake started, when the state was reported.
	pub elapsed_ms: f64,
}

/// The `FinishedBake` struct is one bake that ended.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FinishedBake {
	/// The ID of the resource that was baked.
	pub id: String,
	/// Whether the bake stored its resource.
	pub ok: bool,
	/// How long the bake ran, in milliseconds.
	pub duration_ms: f64,
}

/// The `BakeReporter` trait receives an asset manager's [`BakeState`] after each bake starts or ends.
///
/// Implement it to show bakes somewhere, then install it with
/// [`AssetManager::set_bake_reporter`](super::manager::AssetManager::set_bake_reporter). Reports arrive in order, one
/// at a time, from the asset worker threads; a slow reporter slows bakes instead of missing a state.
pub trait BakeReporter: Send + Sync {
	/// Shows `state`, the asset manager's bakes right after one started or ended.
	fn report(&self, state: &BakeState);
}

/// The `BakeTracker` struct keeps the bakes of one asset manager and reports them, when it has a reporter.
#[derive(Default)]
pub(crate) struct BakeTracker {
	reporter: Option<Arc<dyn BakeReporter>>,
	tracked: Mutex<Tracked>,
}

/// The bakes a tracker keeps between reports.
#[derive(Default)]
struct Tracked {
	/// The bakes running, oldest first, each with its run number and when it started.
	baking: Vec<(u64, String, Instant)>,
	/// The run number the next bake gets, which tells two bakes of one ID apart.
	next_run: u64,
	baked: u64,
	failed: u64,
	/// Newest first.
	finished: VecDeque<FinishedBake>,
	/// The state last reported, kept so its lists are reused.
	state: BakeState,
}

impl BakeTracker {
	/// Creates a tracker that reports to `reporter`.
	pub(crate) fn new(reporter: Arc<dyn BakeReporter>) -> Self {
		Self {
			reporter: Some(reporter),
			tracked: Mutex::default(),
		}
	}

	/// Records that the bake of `id` started, and returns the run that records its end when dropped.
	///
	/// Call [`BakeRun::succeeded`] once the bake stored its resource; a run dropped without it counts as failed, so
	/// every early return of a bake counts. Without a reporter this does nothing.
	pub(crate) fn begin(&self, id: &str) -> BakeRun<'_> {
		let mut run = BakeRun {
			tracker: self,
			number: 0,
			succeeded: false,
		};
		if let Some(reporter) = &self.reporter {
			let started = Instant::now();
			let mut tracked = self.tracked.lock();
			run.number = tracked.next_run;
			tracked.next_run += 1;
			tracked.baking.push((run.number, id.to_string(), started));
			tracked.report(reporter.as_ref(), started);
		}
		run
	}

	/// Moves run `number` from the running bakes to the finished ones, then reports.
	fn finish(&self, number: u64, succeeded: bool) {
		let Some(reporter) = &self.reporter else { return };
		let now = Instant::now();
		let mut tracked = self.tracked.lock();
		let Some(index) = tracked.baking.iter().position(|(run, ..)| *run == number) else {
			return;
		};
		let (_, id, started) = tracked.baking.remove(index);
		if succeeded {
			tracked.baked += 1;
		} else {
			tracked.failed += 1;
		}
		if tracked.finished.len() == FINISHED_KEPT {
			tracked.finished.pop_back();
		}
		tracked.finished.push_front(FinishedBake {
			id,
			ok: succeeded,
			duration_ms: milliseconds(now - started),
		});
		tracked.report(reporter.as_ref(), now);
	}
}

impl Tracked {
	/// Builds the state as of `now` and hands it to `reporter`. The caller holds the lock, so reports keep their order.
	fn report(&mut self, reporter: &dyn BakeReporter, now: Instant) {
		let mut state = std::mem::take(&mut self.state);
		state.baking.clear();
		state.baking.extend(self.baking.iter().map(|(_, id, started)| BakingAsset {
			id: id.clone(),
			elapsed_ms: milliseconds(now.saturating_duration_since(*started)),
		}));
		state.baked = self.baked;
		state.failed = self.failed;
		state.finished.clear();
		state.finished.extend(self.finished.iter().cloned());
		reporter.report(&state);
		self.state = state;
	}
}

/// The `BakeRun` struct is one bake in progress; dropping it records the bake's end.
pub(crate) struct BakeRun<'a> {
	tracker: &'a BakeTracker,
	/// The run's number in its tracker, or 0 when the tracker has no reporter.
	number: u64,
	succeeded: bool,
}

impl BakeRun<'_> {
	/// Records that the bake stored its resource.
	pub(crate) fn succeeded(mut self) {
		self.succeeded = true;
	}
}

impl Drop for BakeRun<'_> {
	fn drop(&mut self) {
		self.tracker.finish(self.number, self.succeeded);
	}
}

fn milliseconds(duration: Duration) -> f64 {
	duration.as_secs_f64() * 1000.0
}

/// The `CapturedBakes` struct keeps every state a manager reported, for tests that check what reporters receive.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct CapturedBakes(pub(crate) Mutex<Vec<BakeState>>);

#[cfg(test)]
impl BakeReporter for CapturedBakes {
	fn report(&self, state: &BakeState) {
		self.0.lock().push(state.clone());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tracker() -> (BakeTracker, Arc<CapturedBakes>) {
		let captured = Arc::new(CapturedBakes::default());
		(BakeTracker::new(captured.clone()), captured)
	}

	#[test]
	fn a_bake_is_reported_running_then_finished() {
		let (tracker, captured) = tracker();

		tracker.begin("models/tree.glb").succeeded();

		let states = captured.0.lock();
		assert_eq!(states.len(), 2);
		assert_eq!(states[0].baking.len(), 1);
		assert_eq!(states[0].baking[0].id, "models/tree.glb");
		assert_eq!(states[0].baked, 0);
		assert!(states[1].baking.is_empty());
		assert_eq!(states[1].baked, 1);
		assert_eq!(states[1].failed, 0);
		assert_eq!(states[1].finished[0].id, "models/tree.glb");
		assert!(states[1].finished[0].ok);
	}

	#[test]
	fn a_bake_dropped_without_success_counts_as_failed() {
		let (tracker, captured) = tracker();

		drop(tracker.begin("shaders/broken.besl"));

		let last = captured.0.lock().last().cloned().expect("a report");
		assert_eq!(last.failed, 1);
		assert_eq!(last.baked, 0);
		assert!(!last.finished[0].ok);
	}

	#[test]
	fn running_bakes_are_oldest_first_and_finished_bakes_newest_first() {
		let (tracker, captured) = tracker();

		let first = tracker.begin("a");
		let second = tracker.begin("b");
		assert_eq!(
			captured
				.0
				.lock()
				.last()
				.unwrap()
				.baking
				.iter()
				.map(|bake| bake.id.as_str())
				.collect::<Vec<_>>(),
			["a", "b"]
		);
		first.succeeded();
		second.succeeded();

		let last = captured.0.lock().last().cloned().unwrap();
		assert_eq!(
			last.finished.iter().map(|bake| bake.id.as_str()).collect::<Vec<_>>(),
			["b", "a"]
		);
	}

	#[test]
	fn only_the_latest_finished_bakes_are_kept() {
		let (tracker, captured) = tracker();

		for index in 0..FINISHED_KEPT + 4 {
			tracker.begin(&index.to_string()).succeeded();
		}

		let last = captured.0.lock().last().cloned().unwrap();
		assert_eq!(last.finished.len(), FINISHED_KEPT);
		assert_eq!(last.finished[0].id, (FINISHED_KEPT + 3).to_string());
		assert_eq!(last.baked, (FINISHED_KEPT + 4) as u64);
	}

	#[test]
	fn the_state_round_trips_through_json() {
		let (tracker, captured) = tracker();
		tracker.begin("a").succeeded();

		let state = captured.0.lock().last().cloned().unwrap();
		let text = serde_json::to_string(&state).unwrap();
		assert_eq!(serde_json::from_str::<BakeState>(&text).unwrap(), state);
	}
}
