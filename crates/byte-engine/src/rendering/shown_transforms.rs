//! The transforms frames show, blended between the samples simulation steps publish.
//!
//! [`ShownTransforms`] reads the world's transform route once for the whole renderer, so scene pipelines, lights,
//! cameras, and particle emitters move on one timeline. The rules are specified under
//! [shown transforms](/docs/reference/rendering#shown-transforms).

use utils::hash::HashMap;

use crate::application::Time;
use crate::core::factory::Handle;
use crate::core::listener::{DefaultListener, Listener as _};
use crate::core::message::DeleteMessage;
use crate::core::message_bus::MessageScope;
use crate::gameplay::Transform;
use crate::gameplay::transform::TransformationUpdate;
use crate::time::MediaTime;

/// The longest time one sample is shown moving toward. A source quieter than this rests between its samples.
const SPAN_LIMIT: MediaTime = MediaTime::from_ticks(MediaTime::TICKS_PER_SECOND / 4);

/// The `ShownTransforms` struct gives every consumer of a frame the same transform for each entity, blended between
/// the samples its source published, so what one pipeline draws lines up with what the cameras and other pipelines
/// show.
///
/// The application keeps it and lends it to [`crate::rendering::PipelineManager::update`] and
/// [`crate::rendering::PipelineManager::prepare`]. Read [`Self::changed`] in `prepare` to move what a manager
/// already draws, and [`Self::get`] to place what it starts drawing.
pub struct ShownTransforms {
	transforms_listener: DefaultListener<TransformationUpdate>,
	deletions_listener: DefaultListener<DeleteMessage>,
	timelines: HashMap<Handle, Timeline>,
	/// The handles whose shown transform may still change, so frames skip entities at rest.
	moving: Vec<Handle>,
	/// The transforms the last resolved frame changed, in no particular order.
	changed: Vec<(Handle, Transform)>,
}

impl ShownTransforms {
	/// Subscribes to `world`'s transforms and deletions. Next, call [`Self::step`] after every simulation step and
	/// [`Self::adopt`] once per tick.
	pub(crate) fn new(world: &MessageScope) -> Self {
		Self {
			transforms_listener: world.channel::<TransformationUpdate>().listener(),
			deletions_listener: world.channel::<DeleteMessage>().listener(),
			timelines: HashMap::default(),
			moving: Vec::new(),
			changed: Vec::new(),
		}
	}

	/// Keeps the transforms published by the step that ended at `time.elapsed()` and lasted `time.delta()` as that
	/// step's samples.
	pub(crate) fn step(&mut self, time: Time) {
		self.read_transforms(Some(time));
	}

	/// Places the transforms published outside a step and forgets deleted handles.
	///
	/// Call it once per tick, after the application's frame code and whether or not the tick renders, so the
	/// routes never hold back their publishers.
	pub(crate) fn adopt(&mut self) {
		self.read_transforms(None);
		while let Some(message) = self.deletions_listener.read() {
			self.timelines.remove(&message.into_handle());
		}
	}

	/// Drains the transform route as samples of the step `step` names, or as placements without one, and queues
	/// every handle it touched for the next resolved frame.
	fn read_transforms(&mut self, step: Option<Time>) {
		while let Some(message) = self.transforms_listener.read() {
			let handle = message.handle();
			let timeline = self
				.timelines
				.entry(handle)
				.and_modify(|timeline| match step {
					Some(time) => timeline.sample(message.transform(), time.elapsed(), time.delta()),
					None => timeline.place(message.transform()),
				})
				.or_insert_with(|| Timeline::new(message.transform(), step.map(|time| time.elapsed())));
			if !timeline.moving {
				timeline.moving = true;
				self.moving.push(handle);
			}
		}
	}

	/// Moves every handle that isn't at rest to where it stands at `display_time`, and lists the new transforms in
	/// [`Self::changed`].
	pub(crate) fn resolve(&mut self, display_time: MediaTime) {
		self.changed.clear();
		let (timelines, changed) = (&mut self.timelines, &mut self.changed);
		// A handle leaves the list once its latest sample is shown, or when it was deleted.
		self.moving.retain(|handle| {
			let Some(timeline) = timelines.get_mut(handle) else {
				return false;
			};
			timeline.shown = timeline.segment.at(display_time);
			changed.push((*handle, timeline.shown.clone()));
			timeline.moving = display_time < timeline.segment.end();
			timeline.moving
		});
	}

	/// Returns the transforms the frame being prepared changed: one entry per handle that moved, was placed, or
	/// appeared since the last frame.
	pub fn changed(&self) -> &[(Handle, Transform)] {
		&self.changed
	}

	/// Returns the transform `handle` is shown at, or `None` when nothing published one.
	///
	/// A handle whose first transform arrived this tick is already shown at it.
	pub fn get(&self, handle: Handle) -> Option<&Transform> {
		self.timelines.get(&handle).map(|timeline| &timeline.shown)
	}
}

/// The `Timeline` struct keeps what one handle shows: the segment toward its latest sample and that sample's stamp.
struct Timeline {
	segment: Segment,
	/// The stamp of the latest sample, or `None` when a placement was the latest update.
	stamp: Option<MediaTime>,
	/// The transform the last resolved frame showed.
	shown: Transform,
	/// Whether the handle is queued for the next resolved frame.
	moving: bool,
}

impl Timeline {
	/// Starts a handle at rest at its first update, stamped when it was a sample.
	fn new(transform: &Transform, stamp: Option<MediaTime>) -> Self {
		Self {
			segment: Segment::at_rest(transform),
			stamp,
			shown: transform.clone(),
			moving: false,
		}
	}

	/// Starts a segment toward a sample stamped `stamp` by a step that lasted `step`.
	fn sample(&mut self, transform: &Transform, stamp: MediaTime, step: MediaTime) {
		self.segment = self.segment.toward(transform, stamp, step, self.stamp);
		self.stamp = Some(stamp);
	}

	/// Shows `transform` at once and forgets the stamp, so the next sample spans one step.
	fn place(&mut self, transform: &Transform) {
		self.segment = Segment::at_rest(transform);
		self.stamp = None;
	}
}

/// The `Segment` struct is the motion a handle shows from one transform to its latest sample.
#[derive(Clone)]
struct Segment {
	from: Transform,
	to: Transform,
	start: MediaTime,
	span: MediaTime,
}

impl Segment {
	/// A segment that already reached `transform`.
	fn at_rest(transform: &Transform) -> Self {
		Self {
			from: transform.clone(),
			to: transform.clone(),
			start: MediaTime::ZERO,
			span: MediaTime::ZERO,
		}
	}

	/// The segment toward a sample of `transform` stamped `stamp` by a step that lasted `step`, after the sample
	/// stamped `previous`.
	///
	/// It starts at the stamp less one step, from wherever this segment stands then, and lasts the time since the
	/// previous stamp up to [`SPAN_LIMIT`], or one step after a placement. A later sample of the same step keeps
	/// this segment and replaces its target.
	fn toward(&self, transform: &Transform, stamp: MediaTime, step: MediaTime, previous: Option<MediaTime>) -> Self {
		if previous == Some(stamp) {
			return Self {
				to: transform.clone(),
				..self.clone()
			};
		}
		let start = stamp - step;
		let span = previous.map_or(step, |previous| (stamp - previous).clamp(MediaTime::ZERO, SPAN_LIMIT));
		Self {
			from: self.at(start),
			to: transform.clone(),
			start,
			span,
		}
	}

	/// The display time this segment reaches its target at.
	fn end(&self) -> MediaTime {
		self.start + self.span
	}

	/// The transform shown at `time`: the start before the segment begins, the target once it ends, and the
	/// blend of the two in between.
	fn at(&self, time: MediaTime) -> Transform {
		if time >= self.end() {
			return self.to.clone();
		}
		if time <= self.start {
			return self.from.clone();
		}
		let progress = (time - self.start).as_ticks() as f64 / self.span.as_ticks() as f64;
		self.from.interpolate(&self.to, progress as f32)
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::channel::Channel as _;

	/// The stepped loop's default step.
	const STEP: MediaTime = MediaTime::from_ticks(MediaTime::TICKS_PER_SECOND / 60);

	fn at(x: f32) -> Transform {
		Transform::from_position(math::Point::new(x, 0.0, 0.0))
	}

	/// The time `steps` steps into the run.
	fn steps(steps: i64) -> MediaTime {
		STEP * steps
	}

	/// The stepped loop's display time after `step` steps and `pending` more time.
	fn display(step: i64, pending: MediaTime) -> MediaTime {
		steps(step) - STEP + pending
	}

	/// A world, the renderer's shown transforms on it, and the handle every test moves.
	struct Rig {
		world: MessageScope,
		shown: ShownTransforms,
		handle: Handle,
	}

	impl Rig {
		fn new() -> Self {
			let world = MessageScope::standalone("shown transforms");
			let shown = ShownTransforms::new(&world);
			Self {
				world,
				shown,
				handle: Handle::new(),
			}
		}

		fn publish(&self, x: f32) {
			self.world
				.channel::<TransformationUpdate>()
				.send(TransformationUpdate::new(self.handle, at(x)));
		}

		/// Publishes `x` from the step that ends `step` steps into the run.
		fn sample(&mut self, step: i64, x: f32) {
			self.publish(x);
			self.shown.step(Time::new(steps(step), STEP));
		}

		/// Publishes `x` outside a step.
		fn place(&mut self, x: f32) {
			self.publish(x);
			self.shown.adopt();
		}

		/// The x the frame at `display_time` shows, or `None` when the frame changed nothing.
		fn frame(&mut self, display_time: MediaTime) -> Option<f32> {
			self.shown.adopt();
			self.shown.resolve(display_time);
			self.shown
				.changed()
				.iter()
				.find(|(handle, _)| *handle == self.handle)
				.map(|(_, transform)| transform.get_position().x())
		}
	}

	fn close(shown: Option<f32>, expected: f32) -> bool {
		shown.is_some_and(|x| (x - expected).abs() < 1e-4)
	}

	#[test]
	fn a_source_publishing_every_step_moves_by_the_blend_factor_then_rests() {
		let mut rig = Rig::new();
		rig.sample(1, 0.0);
		rig.sample(2, 2.0);

		assert!(close(rig.frame(display(2, MediaTime::ZERO)), 0.0));
		assert!(close(rig.frame(display(2, STEP / 2)), 1.0));
		// No sample in step 3: the handle reaches its latest sample, reports it once, and rests.
		rig.shown.step(Time::new(steps(3), STEP));
		assert!(close(rig.frame(display(3, STEP / 4)), 2.0));
		assert_eq!(rig.frame(display(3, STEP / 2)), None);
	}

	#[test]
	fn sources_publishing_every_fourth_step_move_without_stopping_four_steps_behind_at_every_phase() {
		let world = MessageScope::standalone("shown transforms");
		let mut shown = ShownTransforms::new(&world);
		let transforms = world.channel::<TransformationUpdate>();
		// One handle per phase. Each publishes its step number as its x every fourth step, so it moves one unit per
		// step.
		let handles: [Handle; 4] = std::array::from_fn(|_| Handle::new());
		let second_sample = |phase: i64| if phase == 0 { 8 } else { phase + 4 };
		for step in 1..=24 {
			let phase = step % 4;
			transforms.send(TransformationUpdate::new(handles[phase as usize], at(step as f32)));
			shown.step(Time::new(steps(step), STEP));
			// Frames at four times the step rate.
			for quarter in 0..4 {
				shown.adopt();
				shown.resolve(display(step, STEP * quarter / 4));
				for (phase, handle) in handles.iter().enumerate() {
					if step < second_sample(phase as i64) {
						continue;
					}
					let x = shown
						.changed()
						.iter()
						.find(|(changed, _)| changed == handle)
						.map(|(_, transform)| transform.get_position().x());
					let expected = (step - 4) as f32 + quarter as f32 / 4.0;
					assert!(close(x, expected), "phase {phase}, step {step}, quarter {quarter}: {x:?}");
				}
			}
		}
	}

	#[test]
	fn a_placement_shows_at_once_and_ends_the_motion_in_progress() {
		let mut rig = Rig::new();
		rig.sample(1, 0.0);
		rig.sample(2, 2.0);
		rig.place(10.0);

		assert!(close(rig.frame(display(2, STEP / 2)), 10.0));
		assert_eq!(rig.frame(display(2, STEP)), None);
		// The sample after a placement spans one step.
		rig.sample(3, 12.0);
		assert!(close(rig.frame(display(3, STEP / 2)), 11.0));
	}

	#[test]
	fn the_first_sample_after_a_rest_spans_at_most_the_limit_then_catches_up() {
		let mut rig = Rig::new();
		rig.sample(1, 0.0);
		// A minute at rest, then two samples one step apart.
		let resumed = 1 + 60 * 60;
		rig.sample(resumed, 30.0);
		let quarter_of_limit = SPAN_LIMIT / 4;
		assert!(close(rig.frame(steps(resumed) - STEP + quarter_of_limit), 7.5));
		rig.sample(resumed + 1, 31.0);

		// The next segment starts from where the first stood at its stamp less one step and lasts one step.
		let shown_at_start = 30.0 * STEP.as_seconds_f32() / SPAN_LIMIT.as_seconds_f32();
		assert!(close(rig.frame(display(resumed + 1, MediaTime::ZERO)), shown_at_start));
		assert!(close(rig.frame(display(resumed + 1, STEP)), 31.0));
	}

	#[test]
	fn a_later_sample_in_the_same_step_replaces_the_earlier_ones_target() {
		let mut rig = Rig::new();
		rig.sample(1, 0.0);
		rig.publish(1.0);
		rig.publish(4.0);
		rig.shown.step(Time::new(steps(2), STEP));

		assert!(close(rig.frame(display(2, MediaTime::ZERO)), 0.0));
		assert!(close(rig.frame(display(2, STEP / 2)), 2.0));
	}

	#[test]
	fn a_handle_is_shown_at_its_first_update_before_any_frame_and_forgotten_once_deleted() {
		let mut rig = Rig::new();
		rig.place(3.0);
		assert_eq!(
			rig.shown.get(rig.handle).map(|transform| transform.get_position().x()),
			Some(3.0)
		);

		rig.world.channel::<DeleteMessage>().send(DeleteMessage::new(rig.handle));
		assert_eq!(rig.frame(display(1, MediaTime::ZERO)), None);
		assert!(rig.shown.get(rig.handle).is_none());
	}
}
