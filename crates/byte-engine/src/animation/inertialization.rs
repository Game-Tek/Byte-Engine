//! Retained local-pose inertialization for discontinuity-free transitions.

use std::ops::{Add, Div, Mul, Sub};

use math::{Orientation, Scale, Vector};
use resource_management::resources::{ParentSpace, skeleton::LocalTransform};

use crate::MediaTime;

const DECAY_TO_ONE_THOUSANDTH: f32 = 6.907_755_4;

/// The `PoseInertializer` struct retains per-node offsets and velocities for a smooth pose transition.
///
/// Allocate it once for a skeleton, call [`Self::begin`] when a transition
/// starts, then call [`Self::apply`] with the destination pose each frame.
#[derive(Clone, Debug)]
pub struct PoseInertializer {
	nodes: Vec<InertializedTransform>,
	elapsed_seconds: f32,
	duration_seconds: f32,
}

impl PoseInertializer {
	/// Creates cleared transition state for a fixed node count.
	pub fn new(node_count: usize) -> Self {
		Self {
			nodes: vec![InertializedTransform::default(); node_count],
			elapsed_seconds: 0.0,
			duration_seconds: 0.0,
		}
	}

	/// Returns the node count required for poses passed to [`Self::begin`] and [`Self::apply`].
	pub fn node_count(&self) -> usize {
		self.nodes.len()
	}

	/// Returns whether [`Self::apply`] is still smoothing a transition.
	pub fn is_active(&self) -> bool {
		self.elapsed_seconds < self.duration_seconds
	}

	/// Captures the positional and rotational discontinuity between two moving poses.
	///
	/// `sample_delta` is the time between each previous/current pose pair. A
	/// zero transition duration clears the inertializer so the destination pose
	/// takes effect immediately.
	pub fn begin(
		&mut self,
		source_previous: &[LocalTransform],
		source: &[LocalTransform],
		destination_previous: &[LocalTransform],
		destination: &[LocalTransform],
		sample_delta: MediaTime,
		duration: MediaTime,
	) -> Result<(), InertializationError> {
		self.validate_pose_lengths(&[source_previous, source, destination_previous, destination])?;
		let delta = sample_delta.as_seconds_f32();
		let duration_seconds = duration.as_seconds_f32();
		if !delta.is_finite() || delta <= 0.0 {
			return Err(InertializationError::InvalidSampleDelta);
		}
		if !duration_seconds.is_finite() || duration_seconds < 0.0 {
			return Err(InertializationError::InvalidDuration);
		}

		self.elapsed_seconds = 0.0;
		self.duration_seconds = duration_seconds;
		if !self.is_active() {
			self.nodes.fill(InertializedTransform::default());
			return Ok(());
		}

		for ((((source_previous, source), destination_previous), destination), state) in source_previous
			.iter()
			.zip(source)
			.zip(destination_previous)
			.zip(destination)
			.zip(&mut self.nodes)
		{
			state.translation_offset = source.translation - destination.translation;
			state.translation_velocity = velocity(source_previous.translation, source.translation, delta)
				- velocity(destination_previous.translation, destination.translation, delta);
			state.scale_offset = source.scale - destination.scale;
			state.scale_velocity = velocity(source_previous.scale, source.scale, delta)
				- velocity(destination_previous.scale, destination.scale, delta);
			state.rotation_offset = source.rotation.compose(destination.rotation.inverse()).to_rotation_vector();
			state.rotation_velocity = angular_velocity(source_previous.rotation, source.rotation, delta)
				- angular_velocity(destination_previous.rotation, destination.rotation, delta);
		}
		Ok(())
	}

	/// Advances retained offsets and writes an inertialized destination pose.
	///
	/// Once the configured duration elapses, this writes the destination exactly
	/// and marks the transition inactive.
	pub fn apply(
		&mut self,
		destination: &[LocalTransform],
		delta: MediaTime,
		output: &mut [LocalTransform],
	) -> Result<(), InertializationError> {
		self.validate_pose_lengths(&[destination, output])?;
		let delta_seconds = delta.as_seconds_f32();
		if !delta_seconds.is_finite() || delta_seconds < 0.0 {
			return Err(InertializationError::InvalidAdvanceDelta);
		}
		// An inactive transition has `elapsed == duration`, which this update leaves unchanged.
		self.elapsed_seconds = (self.elapsed_seconds + delta_seconds).min(self.duration_seconds);
		if !self.is_active() {
			output.copy_from_slice(destination);
			return Ok(());
		}

		let (duration, elapsed) = (self.duration_seconds, self.elapsed_seconds);
		for ((destination, state), output) in destination.iter().zip(&self.nodes).zip(output) {
			let translation_offset = settle(state.translation_offset, state.translation_velocity, duration, elapsed);
			let scale_offset = settle(state.scale_offset, state.scale_velocity, duration, elapsed);
			let rotation_offset = settle(state.rotation_offset, state.rotation_velocity, duration, elapsed);
			*output = LocalTransform {
				translation: destination.translation + translation_offset,
				rotation: Orientation::try_from_rotation_vector(rotation_offset)
					.expect("settling rotation offsets stay finite")
					.compose(destination.rotation),
				scale: destination.scale + scale_offset,
			};
		}
		Ok(())
	}

	/// Stops the current transition without changing allocated state.
	pub fn clear(&mut self) {
		self.elapsed_seconds = 0.0;
		self.duration_seconds = 0.0;
		self.nodes.fill(InertializedTransform::default());
	}

	fn validate_pose_lengths(&self, poses: &[&[LocalTransform]]) -> Result<(), InertializationError> {
		for pose in poses {
			if pose.len() != self.nodes.len() {
				return Err(InertializationError::PoseLength {
					expected: self.nodes.len(),
					actual: pose.len(),
				});
			}
		}
		Ok(())
	}
}

/// The `InertializedTransform` struct holds one node's settling offsets from its destination pose.
///
/// Rotation offsets and velocities are rotation vectors, which settle linearly unlike quaternions.
#[derive(Clone, Copy, Debug)]
struct InertializedTransform {
	translation_offset: Vector<ParentSpace>,
	translation_velocity: Vector<ParentSpace>,
	rotation_offset: Vector<ParentSpace>,
	rotation_velocity: Vector<ParentSpace>,
	scale_offset: Scale,
	scale_velocity: Scale,
}

impl Default for InertializedTransform {
	fn default() -> Self {
		let no_scale_change = Scale::new(0.0, 0.0, 0.0);
		Self {
			translation_offset: Vector::zero(),
			translation_velocity: Vector::zero(),
			rotation_offset: Vector::zero(),
			rotation_velocity: Vector::zero(),
			scale_offset: no_scale_change,
			scale_velocity: no_scale_change,
		}
	}
}

/// Errors returned when a pose transition cannot be initialized or advanced.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InertializationError {
	/// A pose does not match the inertializer's node count.
	PoseLength {
		/// Node count configured by [`PoseInertializer::new`].
		expected: usize,
		/// Supplied pose node count.
		actual: usize,
	},
	/// The interval between source samples is zero, negative, or non-finite.
	InvalidSampleDelta,
	/// The requested transition duration is negative or non-finite.
	InvalidDuration,
	/// The frame interval used to advance the transition is negative or non-finite.
	InvalidAdvanceDelta,
}

impl std::fmt::Display for InertializationError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			Self::PoseLength { expected, actual } => write!(
				formatter,
				"Inertialization pose has the wrong node count. The most likely cause is using {actual} transforms with state prepared for {expected}."
			),
			Self::InvalidSampleDelta => write!(
				formatter,
				"Inertialization sample delta is invalid. The most likely cause is a zero, negative, or non-finite source frame interval."
			),
			Self::InvalidDuration => write!(
				formatter,
				"Inertialization duration is invalid. The most likely cause is a negative or non-finite transition duration."
			),
			Self::InvalidAdvanceDelta => write!(
				formatter,
				"Inertialization advance delta is invalid. The most likely cause is a negative or non-finite frame interval."
			),
		}
	}
}

impl std::error::Error for InertializationError {}

/// Evaluates an offset settling to the destination with the supplied initial velocity.
///
/// The offset follows a quintic, `1 - 10t^3 + 15t^4 - 6t^5` of the transition, that
/// leaves the source pose without acceleration and reaches the destination with no
/// velocity or acceleration, so a pose mismatch blends away without a first-frame jolt.
/// The velocity decays like a critically damped spring, `v t e^(-rt)`: easing a
/// difference in speed out without acceleration instead swings bones far past the destination.
fn settle<T: Copy + Add<Output = T> + Mul<f32, Output = T>>(offset: T, velocity: T, duration: f32, time: f32) -> T {
	debug_assert!(
		duration.is_finite() && duration > 0.0 && time.is_finite() && time >= 0.0,
		"Inertial settling inputs are invalid. The most likely cause is bypassing transition time validation."
	);
	let progress = (time / duration).min(1.0);
	let remaining = 1.0 - progress;
	let offset_share = remaining * remaining * remaining * (1.0 + progress * (3.0 + 6.0 * progress));
	let rate = DECAY_TO_ONE_THOUSANDTH / duration;
	offset * offset_share + velocity * (time * (-rate * time).exp())
}

fn velocity<T: Sub<Output = T> + Div<f32, Output = T>>(previous: T, current: T, delta: f32) -> T {
	debug_assert!(
		delta.is_finite() && delta > 0.0,
		"Velocity delta is invalid. The most likely cause is bypassing sample interval validation."
	);
	(current - previous) / delta
}

fn angular_velocity(previous: Orientation, current: Orientation, delta: f32) -> Vector<ParentSpace> {
	debug_assert!(
		delta.is_finite() && delta > 0.0,
		"Angular velocity delta is invalid. The most likely cause is bypassing sample interval validation."
	);
	current.compose(previous.inverse()).to_rotation_vector() / delta
}

#[cfg(test)]
mod tests {
	use std::f32::consts::FRAC_PI_2;

	use resource_management::resources::{ParentSpace, skeleton::LocalTransform};

	use super::{PoseInertializer, settle};
	use crate::MediaTime;

	fn transform(position: f32, angle: f32) -> LocalTransform {
		LocalTransform {
			translation: math::Vector::new(position, 0.0, 0.0),
			rotation: math::Orientation::try_from_rotation_vector(math::Vector::<ParentSpace>::new(0.0, angle, 0.0)).unwrap(),
			scale: math::Scale::identity(),
		}
	}

	#[test]
	fn inertialization_starts_at_the_source_pose_and_finishes_at_destination() {
		let source_previous = [transform(0.0, 0.0)];
		let source = [transform(1.0, FRAC_PI_2)];
		let destination_previous = [transform(10.0, 0.0)];
		let destination = [transform(10.0, 0.0)];
		let mut output = [LocalTransform::identity()];
		let mut inertializer = PoseInertializer::new(1);
		inertializer
			.begin(
				&source_previous,
				&source,
				&destination_previous,
				&destination,
				MediaTime::from_millis(16),
				MediaTime::from_millis(200),
			)
			.expect("expected test value");

		inertializer
			.apply(&destination, MediaTime::ZERO, &mut output)
			.expect("expected test value");

		assert!((output[0].translation.x() - 1.0).abs() < 1.0e-4);
		assert!((output[0].rotation.to_array()[1] - source[0].rotation.to_array()[1]).abs() < 1.0e-4);

		inertializer
			.apply(&destination, MediaTime::from_millis(200), &mut output)
			.expect("expected test value");

		assert_eq!(output, destination);
		assert!(!inertializer.is_active());
	}

	const DURATION: f32 = 0.25;

	/// Plays a point that moves at `velocity` into a still destination at the origin, 50 frames a second, and returns the largest change in its step.
	fn peak_second_difference(offset: f32, velocity: f32, curve: impl Fn(f32, f32, f32) -> f32) -> f32 {
		const FRAME: f32 = 0.02;
		let mut positions = vec![offset - velocity * FRAME];
		positions.extend((0..20).map(|frame| curve(offset, velocity, frame as f32 * FRAME)));
		positions
			.windows(3)
			.map(|window| (window[2] - 2.0 * window[1] + window[0]).abs())
			.fold(0.0, f32::max)
	}

	/// The critically damped spring settling replaced, decaying offsets to a thousandth by the transition's end.
	fn spring(offset: f32, velocity: f32, elapsed: f32) -> f32 {
		let rate = 6.907_755_4 / DURATION;
		(offset + (velocity + offset * rate) * elapsed) * (-rate * elapsed).exp()
	}

	fn settled(offset: f32, velocity: f32, elapsed: f32) -> f32 {
		settle(offset, velocity, DURATION, elapsed)
	}

	#[test]
	fn settling_leaves_the_source_motion_without_a_jolt_and_ends_at_rest() {
		const STEP: f32 = 1.0e-3;
		let at = |velocity: f32, elapsed: f32| settled(0.1, velocity, elapsed);

		assert_eq!(at(0.0, 0.0), 0.1);
		assert!(((at(0.5, STEP) - at(0.5, 0.0)) / STEP - 0.5).abs() < 0.02);
		let pose_acceleration = (at(0.0, 2.0 * STEP) - 2.0 * at(0.0, STEP) + at(0.0, 0.0)) / (STEP * STEP);
		// A spring starts this offset accelerating at 76 m/s^2; what is left here is the step times the jerk.
		assert!(
			pose_acceleration.abs() < 1.0,
			"a pose offset starts accelerating at {pose_acceleration}"
		);

		assert_eq!(at(0.0, DURATION), 0.0);
		let end_velocity = (at(0.0, DURATION) - at(0.0, DURATION - STEP)) / STEP;
		assert!(end_velocity.abs() < 1.0e-3, "a pose offset ends moving at {end_velocity}");
		assert!(at(0.5, DURATION).abs() < 1.0e-3 * 0.5 * DURATION);
	}

	#[test]
	fn settling_jolts_bones_less_than_a_critically_damped_spring() {
		let pose = peak_second_difference(0.3, 0.0, settled);
		let sprung_pose = peak_second_difference(0.3, 0.0, spring);
		assert!(pose < 0.5 * sprung_pose, "pose offset: {pose} vs spring {sprung_pose}");

		// A bone moving away from a nearby destination, like a swinging foot meeting a planted one.
		let moving = peak_second_difference(0.05, 3.0, settled);
		let sprung_moving = peak_second_difference(0.05, 3.0, spring);
		assert!(moving <= sprung_moving, "moving offset: {moving} vs spring {sprung_moving}");
	}

	#[test]
	fn rotation_offsets_settle_without_passing_the_destination() {
		let frame = MediaTime::from_millis(10);
		let source_previous = [transform(0.0, 0.34)];
		let source = [transform(0.0, 0.3)];
		let destination = [transform(0.0, 0.0)];
		let mut output = [LocalTransform::identity()];
		let mut inertializer = PoseInertializer::new(1);
		inertializer
			.begin(
				&source_previous,
				&source,
				&destination,
				&destination,
				frame,
				MediaTime::from_millis(250),
			)
			.expect("expected test value");

		let mut angles = Vec::new();
		while inertializer.is_active() {
			inertializer
				.apply(&destination, frame, &mut output)
				.expect("expected test value");
			angles.push(output[0].rotation.to_rotation_vector::<ParentSpace>().y());
		}

		// Only the velocity's spring tail, which outlasts the offset, may cross, by under a tenth of a degree.
		assert!(angles.iter().all(|angle| *angle >= -2.0e-3), "{angles:?}");
		assert!(
			angles[0] < 0.28,
			"the first frame keeps the source's 4 rad/s turn: {}",
			angles[0]
		);
		assert_eq!(output, destination);
	}

	#[test]
	fn zero_duration_transition_uses_destination_immediately() {
		let source = [transform(1.0, 0.0)];
		let destination = [transform(2.0, 0.0)];
		let mut output = [LocalTransform::identity()];
		let mut inertializer = PoseInertializer::new(1);
		inertializer
			.begin(
				&source,
				&source,
				&destination,
				&destination,
				MediaTime::from_millis(16),
				MediaTime::ZERO,
			)
			.expect("expected test value");
		inertializer
			.apply(&destination, MediaTime::ZERO, &mut output)
			.expect("expected test value");

		assert_eq!(output, destination);
	}
}
