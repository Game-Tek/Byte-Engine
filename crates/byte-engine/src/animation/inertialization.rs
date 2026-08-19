//! Retained local-pose inertialization for discontinuity-free transitions.

use resource_management::resources::skeleton::LocalTransform;

use super::math::{conjugate_quaternion, multiply_quaternion, quaternion_exp, quaternion_log};
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

	pub fn node_count(&self) -> usize {
		self.nodes.len()
	}

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
			state.translation_offset = subtract3(source.translation, destination.translation);
			state.translation_velocity = subtract3(
				velocity3(source_previous.translation, source.translation, sample_delta_seconds),
				velocity3(
					destination_previous.translation,
					destination.translation,
					sample_delta_seconds,
				),
			);
			state.scale_offset = subtract3(source.scale, destination.scale);
			state.scale_velocity = subtract3(
				velocity3(source_previous.scale, source.scale, sample_delta_seconds),
				velocity3(destination_previous.scale, destination.scale, sample_delta_seconds),
			);

			let rotation_offset = multiply_quaternion(source.rotation, conjugate_quaternion(destination.rotation));
			state.rotation_offset = quaternion_log(rotation_offset);
			state.rotation_velocity = subtract3(
				angular_velocity(source_previous.rotation, source.rotation, sample_delta_seconds),
				angular_velocity(destination_previous.rotation, destination.rotation, sample_delta_seconds),
			);
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
			let translation_offset = decay_vector(
				state.translation_offset,
				state.translation_velocity,
				decay_rate,
				self.elapsed_seconds,
			);
			let scale_offset = decay_vector(state.scale_offset, state.scale_velocity, decay_rate, self.elapsed_seconds);
			let rotation_offset = decay_vector(
				state.rotation_offset,
				state.rotation_velocity,
				decay_rate,
				self.elapsed_seconds,
			);
			*output = LocalTransform {
				translation: add3(destination.translation, translation_offset),
				rotation: multiply_quaternion(quaternion_exp(rotation_offset), destination.rotation),
				scale: add3(destination.scale, scale_offset),
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

#[derive(Clone, Copy, Debug, Default)]
struct InertializedTransform {
	translation_offset: [f32; 3],
	translation_velocity: [f32; 3],
	rotation_offset: [f32; 3],
	rotation_velocity: [f32; 3],
	scale_offset: [f32; 3],
	scale_velocity: [f32; 3],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InertializationError {
	PoseLength { expected: usize, actual: usize },
	InvalidSampleDelta,
	InvalidDuration,
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
fn decay_vector(offset: [f32; 3], velocity: [f32; 3], rate: f32, time: f32) -> [f32; 3] {
	debug_assert!(
		rate.is_finite() && rate >= 0.0 && time.is_finite() && time >= 0.0,
		"Inertial decay inputs are invalid. The most likely cause is bypassing transition time validation."
	);
	let decay = (-rate * time).exp();
	std::array::from_fn(|component| (offset[component] + (velocity[component] + rate * offset[component]) * time) * decay)
}

fn velocity3(previous: [f32; 3], current: [f32; 3], delta: f32) -> [f32; 3] {
	debug_assert!(
		delta.is_finite() && delta > 0.0,
		"Velocity delta is invalid. The most likely cause is bypassing sample interval validation."
	);
	std::array::from_fn(|component| (current[component] - previous[component]) / delta)
}

fn angular_velocity(previous: [f32; 4], current: [f32; 4], delta: f32) -> [f32; 3] {
	debug_assert!(
		delta.is_finite() && delta > 0.0,
		"Angular velocity delta is invalid. The most likely cause is bypassing sample interval validation."
	);
	let delta_rotation = multiply_quaternion(current, conjugate_quaternion(previous));
	let rotation_vector = quaternion_log(delta_rotation);
	std::array::from_fn(|component| rotation_vector[component] / delta)
}

fn add3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] + right[component])
}

fn subtract3(left: [f32; 3], right: [f32; 3]) -> [f32; 3] {
	std::array::from_fn(|component| left[component] - right[component])
}

#[cfg(test)]
mod tests {
	use std::f32::consts::FRAC_PI_2;

	use resource_management::resources::skeleton::LocalTransform;

	use super::PoseInertializer;
	use crate::animation::math::quaternion_exp;
	use crate::MediaTime;

	fn transform(position: f32, angle: f32) -> LocalTransform {
		LocalTransform {
			translation: [position, 0.0, 0.0],
			rotation: quaternion_exp([0.0, angle, 0.0]),
			scale: [1.0; 3],
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

		assert!((output[0].translation[0] - 1.0).abs() < 1.0e-4);
		assert!((output[0].rotation[1] - source[0].rotation[1]).abs() < 1.0e-4);

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
