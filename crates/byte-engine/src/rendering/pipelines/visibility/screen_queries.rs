//! The bookkeeping that answers screen scene queries: which queries wait, which a frame took, and which entity an
//! instance slot held in that frame.
//!
//! This module only tracks queries. The GPU reads happen in the sink's
//! `ScreenQueryPass`, and the pipeline manager moves results between the two. The rules are specified under
//! [Screen scene queries](/docs/reference/rendering#screen-scene-queries).

use std::collections::VecDeque;

use smallvec::SmallVec;

use super::render_pass::{NO_INSTANCE, SCREEN_QUERY_CAPACITY};
use crate::core::factory::Handle;
use crate::rendering::{ScreenSceneHit, ScreenSceneQuery};

/// The queries one sink's frame took, with the handles they were created with, in the order its points are written.
pub(crate) type TakenQueries = SmallVec<[(Handle, ScreenSceneQuery); SCREEN_QUERY_CAPACITY]>;

/// The `ScreenQueries` struct keeps screen scene queries from their creation until a completed frame answers them, so
/// the pipeline manager can take them frame by frame and answer each one exactly once.
///
/// Each frame, call [`Self::complete`] for the frame sequence that is starting again, then [`Self::push`] the new
/// queries, [`Self::answer_unshown`], and [`Self::take`] for each sink. Give [`Self::record`] what a sink recorded and
/// [`Self::put_back`] what it didn't, then [`Self::remember_slots`] once per frame that recorded any.
#[derive(Default)]
pub(crate) struct ScreenQueries {
	/// Queries no frame took yet, oldest first.
	pending: VecDeque<(Handle, ScreenSceneQuery)>,
	/// Per frame sequence, the queries each sink's frame took, by sink index.
	in_flight: [Vec<(usize, SmallVec<[Handle; SCREEN_QUERY_CAPACITY]>)>; ghi::MAX_FRAMES_IN_FLIGHT],
	/// Per frame sequence, the entity of every instance slot when the frame took its queries.
	slot_entities: [Vec<Option<Handle>>; ghi::MAX_FRAMES_IN_FLIGHT],
}

impl ScreenQueries {
	/// Queues a query created with `handle`.
	pub(crate) fn push(&mut self, handle: Handle, query: ScreenSceneQuery) {
		self.pending.push_back((handle, query));
	}

	/// Answers, with no entity, every waiting query whose camera no sink of this frame shows.
	pub(crate) fn answer_unshown(&mut self, shown: &[Handle], mut answer: impl FnMut(ScreenSceneHit)) {
		self.pending.retain(|(handle, query)| {
			let is_shown = shown.contains(&query.camera());
			if !is_shown {
				answer(ScreenSceneHit::new(*handle, None));
			}
			is_shown
		});
	}

	/// Takes the oldest waiting queries of `camera`, at most [`SCREEN_QUERY_CAPACITY`].
	pub(crate) fn take(&mut self, camera: Handle) -> TakenQueries {
		let mut taken = TakenQueries::new();
		self.pending.retain(|&(handle, query)| {
			let takes = taken.len() < SCREEN_QUERY_CAPACITY && query.camera() == camera;
			if takes {
				taken.push((handle, query));
			}
			!takes
		});
		taken
	}

	/// Returns queries a sink took but didn't record to the front of the queue, in their order, so a later frame takes
	/// them first.
	pub(crate) fn put_back(&mut self, taken: TakenQueries) {
		for query in taken.into_iter().rev() {
			self.pending.push_front(query);
		}
	}

	/// Notes that `sink` recorded the reads of `taken` in a frame of `sequence`.
	pub(crate) fn record(&mut self, sequence: usize, sink: usize, taken: &TakenQueries) {
		self.in_flight[sequence].push((sink, taken.iter().map(|&(handle, _)| handle).collect()));
	}

	/// Whether a frame of `sequence` recorded any reads that still wait for their answers.
	pub(crate) fn has_recorded(&self, sequence: usize) -> bool {
		!self.in_flight[sequence].is_empty()
	}

	/// Keeps the entity of every instance slot as the frame of `sequence` that recorded the reads saw them, so their
	/// answers name the entity that was drawn even if the slot changes before the frame completes.
	pub(crate) fn remember_slots(&mut self, sequence: usize, slots: impl IntoIterator<Item = Option<Handle>>) {
		let entities = &mut self.slot_entities[sequence];
		entities.clear();
		entities.extend(slots);
	}

	/// Answers every query a sink's frame of `sequence` took, now that the frame completed.
	///
	/// `read` returns what a sink's reads found, as [`resolve`] takes it, or `None` when the sink no longer exists,
	/// whose queries are answered with no entity.
	pub(crate) fn complete(
		&mut self,
		sequence: usize,
		mut read: impl FnMut(usize) -> Option<[u32; SCREEN_QUERY_CAPACITY]>,
		mut answer: impl FnMut(ScreenSceneHit),
	) {
		let entities = &self.slot_entities[sequence];
		for (sink, taken) in self.in_flight[sequence].drain(..) {
			let instances = read(sink);
			for (index, query) in taken.into_iter().enumerate() {
				let entity = instances.and_then(|instances| resolve(instances[index], entities));
				answer(ScreenSceneHit::new(query, entity));
			}
		}
	}
}

/// Returns the entity that held instance slot `instance` when the read was recorded, or `None` for
/// [`NO_INSTANCE`] or a slot no entity held.
pub(crate) fn resolve(instance: u32, slot_entities: &[Option<Handle>]) -> Option<Handle> {
	if instance == NO_INSTANCE {
		return None;
	}
	slot_entities.get(instance as usize).copied().flatten()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn handle() -> Handle {
		Handle::new()
	}

	/// Takes the waiting queries of `camera` for one sink, as one frame would, and returns their handles and points.
	fn take(queries: &mut ScreenQueries, camera: Handle) -> (Vec<Handle>, Vec<[u32; 2]>) {
		queries.take(camera).into_iter().map(|(handle, query)| (handle, query.point())).unzip()
	}

	#[test]
	fn a_frame_takes_the_oldest_sixteen_queries_of_its_camera_and_leaves_the_rest_waiting() {
		let mut queries = ScreenQueries::default();
		let (camera, other) = (handle(), handle());
		let created: Vec<Handle> = (0..20).map(|_| handle()).collect();
		queries.push(handle(), ScreenSceneQuery::new(other, [9, 9]));
		for (index, query) in created.iter().enumerate() {
			queries.push(*query, ScreenSceneQuery::new(camera, [index as u32, 0]));
		}

		let (first, points) = take(&mut queries, camera);
		assert_eq!(first, created[..16]);
		assert_eq!(points[3], [3, 0]);
		let (second, _) = take(&mut queries, camera);
		assert_eq!(second, created[16..]);
		assert!(take(&mut queries, camera).0.is_empty());
	}

	#[test]
	fn queries_put_back_are_taken_first_in_their_order() {
		let mut queries = ScreenQueries::default();
		let camera = handle();
		let (first, second, third) = (handle(), handle(), handle());
		queries.push(first, ScreenSceneQuery::new(camera, [1, 0]));
		queries.push(second, ScreenSceneQuery::new(camera, [2, 0]));
		let taken = queries.take(camera);
		queries.push(third, ScreenSceneQuery::new(camera, [3, 0]));
		queries.put_back(taken);

		let (again, points) = take(&mut queries, camera);
		assert_eq!(again, [first, second, third]);
		assert_eq!(points, [[1, 0], [2, 0], [3, 0]]);
	}

	#[test]
	fn a_query_whose_camera_no_sink_shows_is_answered_with_no_entity() {
		let mut queries = ScreenQueries::default();
		let (shown, hidden) = (handle(), handle());
		let (kept, answered) = (handle(), handle());
		queries.push(kept, ScreenSceneQuery::new(shown, [0, 0]));
		queries.push(answered, ScreenSceneQuery::new(hidden, [0, 0]));

		let mut hits = Vec::new();
		queries.answer_unshown(&[shown], |hit| hits.push(hit));

		assert_eq!(hits, [ScreenSceneHit::new(answered, None)]);
		assert_eq!(take(&mut queries, shown).0, [kept]);
	}

	#[test]
	fn a_completed_frame_answers_its_queries_with_the_entities_its_slots_held() {
		let mut queries = ScreenQueries::default();
		let camera = handle();
		let (mesh, on_mesh, on_sky, outside) = (handle(), handle(), handle(), handle());
		for (query, point) in [(on_mesh, [5, 5]), (on_sky, [0, 0]), (outside, [u32::MAX, 0])] {
			queries.push(query, ScreenSceneQuery::new(camera, point));
		}
		let taken = queries.take(camera);
		queries.record(1, 0, &taken);
		queries.remember_slots(1, [None, None, Some(mesh)]);
		// Another frame sequence's slots don't change this frame's answers.
		queries.remember_slots(0, [None, None, Some(handle())]);

		let mut instances = [NO_INSTANCE; SCREEN_QUERY_CAPACITY];
		instances[0] = 2;
		let mut hits = Vec::new();
		queries.complete(1, |sink| (sink == 0).then_some(instances), |hit| hits.push(hit));

		assert_eq!(
			hits,
			[
				ScreenSceneHit::new(on_mesh, Some(mesh)),
				ScreenSceneHit::new(on_sky, None),
				ScreenSceneHit::new(outside, None),
			]
		);
		assert!(!queries.has_recorded(1));
	}

	#[test]
	fn queries_of_a_sink_that_is_gone_are_answered_with_no_entity() {
		let mut queries = ScreenQueries::default();
		let (camera, query) = (handle(), handle());
		queries.push(query, ScreenSceneQuery::new(camera, [0, 0]));
		let taken = queries.take(camera);
		queries.record(0, 3, &taken);

		let mut hits = Vec::new();
		queries.complete(0, |_| None::<[u32; SCREEN_QUERY_CAPACITY]>, |hit| hits.push(hit));

		assert_eq!(hits, [ScreenSceneHit::new(query, None)]);
	}
}
