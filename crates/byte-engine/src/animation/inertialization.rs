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
	active: bool,
}

impl PoseInertializer {
	/// Creates cleared transition state for a fixed node count.
	pub fn new(node_count: usize) -> Self {
		Self {
			nodes: vec![InertializedTransform::default(); node_count],
			elapsed_seconds: 0.0,
			duration_seconds: 0.0,
			active: false,
		}
	}

	/// Returns the node count required for poses passed to [`Self::begin`] and [`Self::apply`].
	pub fn node_count(&self) -> usize {
		self.nodes.len()
	}

	/// Returns whether [`Self::apply`] is still smoothing a transition.
	pub fn is_active(&self) -> bool {
		self.active
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
		let sample_delta_seconds = sample_delta.as_seconds_f32();
		let duration_seconds = duration.as_seconds_f32();
		if !sample_delta_seconds.is_finite() || sample_delta_seconds <= 0.0 {
			return Err(InertializationError::InvalidSampleDelta);
		}
		if !duration_seconds.is_finite() || duration_seconds < 0.0 {
			return Err(InertializationError::InvalidDuration);
		}

		self.elapsed_seconds = 0.0;
		self.duration_seconds = duration_seconds;
		self.active = duration_seconds > 0.0;
		if !self.active {
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
			state.translation_velocity = velocity(source_previous.translation, source.translation, sample_delta_seconds)
				- velocity(
					destination_previous.translation,
					destination.translation,
					sample_delta_seconds,
				);
			state.scale_offset = source.scale - destination.scale;
			state.scale_velocity = velocity(source_previous.scale, source.scale, sample_delta_seconds)
				- velocity(destination_previous.scale, destination.scale, sample_delta_seconds);
			state.rotation_offset = source.rotation.compose(destination.rotation.inverse()).to_rotation_vector();
			state.rotation_velocity = angular_velocity(source_previous.rotation, source.rotation, sample_delta_seconds)
				- angular_velocity(destination_previous.rotation, destination.rotation, sample_delta_seconds);
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
		if !self.active {
			output.copy_from_slice(destination);
			return Ok(());
		}

		self.elapsed_seconds = (self.elapsed_seconds + delta_seconds).min(self.duration_seconds);
		if self.elapsed_seconds >= self.duration_seconds {
			self.active = false;
			output.copy_from_slice(destination);
			return Ok(());
		}

		let decay_rate = DECAY_TO_ONE_THOUSANDTH / self.duration_seconds;
		for ((destination, state), output) in destination.iter().zip(&self.nodes).zip(output) {
			let translation_offset = decay(
				state.translation_offset,
				state.translation_velocity,
				decay_rate,
				self.elapsed_seconds,
			);
			let scale_offset = decay(state.scale_offset, state.scale_velocity, decay_rate, self.elapsed_seconds);
			let rotation_offset = decay(
				state.rotation_offset,
				state.rotation_velocity,
				decay_rate,
				self.elapsed_seconds,
			);
			*output = LocalTransform {
				translation: destination.translation + translation_offset,
				rotation: Orientation::try_from_rotation_vector(rotation_offset)
					.expect("decayed rotation offsets stay finite")
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
		self.active = false;
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

/// The `InertializedTransform` struct holds one node's decaying offsets from its destination pose.
///
/// Rotation offsets and velocities are rotation vectors, which decay linearly unlike quaternions.
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

/// Evaluates an exact critically damped offset with the supplied initial velocity.
fn decay<T: Copy + Add<Output = T> + Mul<f32, Output = T>>(offset: T, velocity: T, rate: f32, time: f32) -> T {
	debug_assert!(
		rate.is_finite() && rate >= 0.0 && time.is_finite() && time >= 0.0,
		"Inertial decay inputs are invalid. The most likely cause is bypassing transition time validation."
	);
	let decay = (-rate * time).exp();
	(offset + (velocity + offset * rate) * time) * decay
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

	use resource_management::resources::skeleton::LocalTransform;

	use super::PoseInertializer;
	use crate::MediaTime;

	fn transform(position: f32, angle: f32) -> LocalTransform {
		LocalTransform {
			translation: math::Vector::new(position, 0.0, 0.0),
			rotation: math::Orientation::try_from_rotation_vector(
				math::Vector::<resource_management::resources::ParentSpace>::new(0.0, angle, 0.0),
			)
			.unwrap(),
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
