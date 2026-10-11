//! How many lanes a physics step runs on.

use std::{fmt, str::FromStr};

use smallvec::SmallVec;

use crate::core::machine::performance_parallelism;

/// The number of bodies from which a step runs on two lanes by default instead of on the calling thread.
///
/// Starting lanes and meeting them at every stage costs more than they save on a smaller step. On an Apple M4, two
/// lanes matched one at about 400 stacked cubes in steps run back to back. Lanes that slept since the last frame
/// take longer to start, so the default sits above that.
const SHARED_STEP_BODIES: usize = 512;

/// The number of bodies from which a step runs on more than two lanes by default.
///
/// Every stage of a step ends only once each running lane has reached it, and a smaller step's shares are short,
/// so more lanes spend longer meeting than they save. On an Apple M4, four lanes overtook two between 2,000 and
/// 4,000 stacked cubes.
const WIDE_STEP_BODIES: usize = 4000;

/// The most lanes a step runs on by default.
///
/// One lane at a time resolves contacts and publishes transforms, which bounds what more lanes can save: on an
/// Apple M4, four lanes were 9% faster than two at 16,000 stacked cubes.
const WIDEST_STEP_LANES: usize = 4;

/// The `StepLanes` struct tells [`World::update_with_alley`](super::World::update_with_alley) how many lanes to
/// run a step on, from the number of bodies in the step.
///
/// The default, [`Self::for_machine`], suits the machine the process runs on. Replace it with
/// [`World::set_step_lanes`](super::World::set_step_lanes) when measurements on a machine show better counts.
/// [`GraphicsApplication`](crate::application::graphics::GraphicsApplication) reads a replacement from its
/// `physics-step-lanes` parameter, which this type parses.
///
/// The text form lists `bodies:lanes` pairs separated by commas, such as `512:2,4000:4`: steps of at least 512
/// bodies run on two lanes and steps of at least 4,000 on four. One lane means the calling thread, which is also
/// where smaller steps than any pair names run, so `0:1` keeps every step off the lanes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepLanes {
	/// The lanes for steps of at least some number of bodies, as `(bodies, lanes)` in ascending order of bodies.
	tiers: SmallVec<[(usize, usize); 4]>,
}

impl StepLanes {
	/// Creates lane counts from `(bodies, lanes)` pairs: a step of at least `bodies` bodies runs on `lanes` lanes.
	///
	/// The pair with the most bodies a step reaches decides. A step that reaches none runs on the calling thread.
	pub fn new(tiers: impl IntoIterator<Item = (usize, usize)>) -> Self {
		let mut tiers = tiers.into_iter().collect::<SmallVec<_>>();
		debug_assert!(
			tiers.iter().all(|(_, lanes)| *lanes > 0),
			"Step lane count is zero. The most likely cause is a pair that names no lanes; use one lane to run a step on the calling thread."
		);
		// The sort is stable, so the last of several pairs for the same body count decides, as a later setting should.
		tiers.sort_by_key(|(bodies, _)| *bodies);
		Self { tiers }
	}

	/// Returns the lane counts that suit the machine the process runs on.
	///
	/// See [`Self::for_parallelism`], which this calls with [`performance_parallelism`].
	pub fn for_machine() -> Self {
		Self::for_parallelism(performance_parallelism())
	}

	/// Returns the lane counts that suit a machine whose fastest cores run `fast_threads` hardware threads.
	///
	/// A step never uses more lanes than that, because every stage waits for its slowest lane. Small steps stay
	/// on the calling thread, steps of at least 512 bodies run on two lanes, and steps of at least 4,000 on up
	/// to four.
	pub fn for_parallelism(fast_threads: usize) -> Self {
		let widest = fast_threads.min(WIDEST_STEP_LANES);
		let shared = (widest >= 2).then_some((SHARED_STEP_BODIES, 2));
		let wide = (widest > 2).then_some((WIDE_STEP_BODIES, widest));
		Self::new(shared.into_iter().chain(wide))
	}

	/// Returns the number of lanes a step of `bodies` bodies runs on. One lane is the calling thread.
	pub fn lanes(&self, bodies: usize) -> usize {
		self.tiers
			.iter()
			.take_while(|(from, _)| *from <= bodies)
			.last()
			.map_or(1, |(_, lanes)| *lanes)
	}
}

impl Default for StepLanes {
	fn default() -> Self {
		Self::for_machine()
	}
}

impl FromStr for StepLanes {
	type Err = StepLanesError;

	fn from_str(text: &str) -> Result<Self, Self::Err> {
		let pair = |entry: &str| {
			let invalid = || StepLanesError {
				entry: entry.trim().to_owned(),
			};
			let (bodies, lanes) = entry.split_once(':').ok_or_else(invalid)?;
			let bodies = bodies.trim().parse::<usize>().map_err(|_| invalid())?;
			let lanes = lanes.trim().parse::<usize>().map_err(|_| invalid())?;
			if lanes == 0 { Err(invalid()) } else { Ok((bodies, lanes)) }
		};
		text.split(',')
			.map(pair)
			.collect::<Result<SmallVec<[_; 4]>, _>>()
			.map(Self::new)
	}
}

/// The `StepLanesError` struct reports the entry that kept a [`StepLanes`] text from parsing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepLanesError {
	entry: String,
}

impl fmt::Display for StepLanesError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			formatter,
			"Step lanes entry `{}` is invalid. The most likely cause is an entry that is not `bodies:lanes` with at least one lane, such as `512:2`.",
			self.entry
		)
	}
}

impl std::error::Error for StepLanesError {}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_reached_pair_with_the_most_bodies_decides() {
		let lanes = StepLanes::new([(4000, 4), (512, 2)]);

		assert_eq!(lanes.lanes(0), 1);
		assert_eq!(lanes.lanes(511), 1);
		assert_eq!(lanes.lanes(512), 2);
		assert_eq!(lanes.lanes(3999), 2);
		assert_eq!(lanes.lanes(4000), 4);
		assert_eq!(lanes.lanes(usize::MAX), 4);
	}

	#[test]
	fn no_pairs_keep_every_step_on_the_calling_thread() {
		assert_eq!(StepLanes::new([]).lanes(1_000_000), 1);
	}

	#[test]
	fn machine_defaults_use_no_more_lanes_than_fast_threads() {
		assert_eq!(StepLanes::for_parallelism(1), StepLanes::new([]));
		assert_eq!(StepLanes::for_parallelism(2), StepLanes::new([(512, 2)]));
		assert_eq!(StepLanes::for_parallelism(3), StepLanes::new([(512, 2), (4000, 3)]));
		assert_eq!(StepLanes::for_parallelism(4), StepLanes::new([(512, 2), (4000, 4)]));
		assert_eq!(StepLanes::for_parallelism(24), StepLanes::new([(512, 2), (4000, 4)]));
	}

	#[test]
	fn text_lists_pairs_in_any_order() {
		assert_eq!("512:2,4000:4".parse(), Ok(StepLanes::new([(512, 2), (4000, 4)])));
		assert_eq!(" 4000 : 6 , 300:3 ".parse(), Ok(StepLanes::new([(300, 3), (4000, 6)])));
		assert_eq!("0:1".parse::<StepLanes>().map(|lanes| lanes.lanes(50_000)), Ok(1));
	}

	#[test]
	fn text_that_is_not_pairs_names_the_entry_at_fault() {
		for (text, entry) in [("512:2,4000", "4000"), ("many:2", "many:2"), ("512:0", "512:0"), ("", "")] {
			let error = text.parse::<StepLanes>().unwrap_err();

			assert_eq!(error, StepLanesError { entry: entry.to_owned() });
		}
	}
}
