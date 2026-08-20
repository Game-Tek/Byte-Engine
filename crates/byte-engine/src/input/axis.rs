//! Input-domain axis values used by device triggers and actions.

use std::ops::{Add, Mul};

use math::{NormalizationError, UnitVector, Vector};

/// The `Axis2` struct carries a two-channel input value such as a mouse delta or gamepad stick.
///
/// Use `Axis2` for device-relative values passed through [`super::Value`]. It
/// deliberately has no world-space meaning. Next, use it in a
/// [`super::input_trigger::TriggerDescription`] or an action [`super::ValueMapping`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axis2 {
	/// The horizontal input channel.
	pub x: f32,
	/// The vertical input channel.
	pub y: f32,
}

impl Axis2 {
	/// Creates a two-channel input value from its horizontal and vertical components.
	pub const fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	/// Returns the neutral two-channel input value.
	pub const fn zero() -> Self {
		Self::new(0.0, 0.0)
	}

	/// Returns the lowest finite components accepted by input trigger descriptions.
	pub const fn min_value() -> Self {
		Self::new(f32::MIN, f32::MIN)
	}

	/// Returns the highest finite components accepted by input trigger descriptions.
	pub const fn max_value() -> Self {
		Self::new(f32::MAX, f32::MAX)
	}

	/// Scales a non-zero axis to unit length while preserving a neutral axis.
	pub fn normalized(self) -> Self {
		let length_squared = self.x * self.x + self.y * self.y;
		if length_squared == 0.0 {
			return self;
		}

		let inverse_length = length_squared.sqrt().recip();
		Self::new(self.x * inverse_length, self.y * inverse_length)
	}
}

impl Add for Axis2 {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self::new(self.x + rhs.x, self.y + rhs.y)
	}
}

/// The `Axis3` struct carries a three-channel input value such as a headset position or directional command.
///
/// Use `Axis3` for device-relative values passed through [`super::Value`]. It
/// deliberately has no world-space meaning. Next, use it in a
/// [`super::input_trigger::TriggerDescription`] or an action [`super::ValueMapping`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Axis3 {
	/// The first input channel, conventionally horizontal.
	pub x: f32,
	/// The second input channel, conventionally vertical.
	pub y: f32,
	/// The third input channel, conventionally depth.
	pub z: f32,
}

impl Axis3 {
	/// Creates a three-channel input value from its components.
	pub const fn new(x: f32, y: f32, z: f32) -> Self {
		Self { x, y, z }
	}

	/// Returns the neutral three-channel input value.
	pub const fn zero() -> Self {
		Self::new(0.0, 0.0, 0.0)
	}

	/// Returns the lowest finite components accepted by input trigger descriptions.
	pub const fn min_value() -> Self {
		Self::new(f32::MIN, f32::MIN, f32::MIN)
	}

	/// Returns the highest finite components accepted by input trigger descriptions.
	pub const fn max_value() -> Self {
		Self::new(f32::MAX, f32::MAX, f32::MAX)
	}

	/// Scales a non-zero axis to unit length while preserving a neutral axis.
	pub fn normalized(self) -> Self {
		let length_squared = self.x * self.x + self.y * self.y + self.z * self.z;
		if length_squared == 0.0 {
			return self;
		}

		let inverse_length = length_squared.sqrt().recip();
		Self::new(self.x * inverse_length, self.y * inverse_length, self.z * inverse_length)
	}
}

impl Add for Axis3 {
	type Output = Self;

	fn add(self, rhs: Self) -> Self::Output {
		Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
	}
}

impl Mul for Axis3 {
	type Output = Self;

	fn mul(self, rhs: Self) -> Self::Output {
		Self::new(self.x * rhs.x, self.y * rhs.y, self.z * rhs.z)
	}
}

impl From<UnitVector> for Axis3 {
	fn from(value: UnitVector) -> Self {
		Self::new(value.x(), value.y(), value.z())
	}
}

impl TryFrom<Axis3> for UnitVector {
	type Error = NormalizationError;

	fn try_from(value: Axis3) -> Result<Self, NormalizationError> {
		UnitVector::try_from_vector(Vector::new(value.x, value.y, value.z))
	}
}

#[cfg(test)]
mod tests {
	use super::{Axis2, Axis3};

	#[test]
	fn normalization_preserves_neutral_axes() {
		assert_eq!(Axis2::zero().normalized(), Axis2::zero());
		assert_eq!(Axis3::zero().normalized(), Axis3::zero());
	}

	#[test]
	fn normalization_preserves_axis_direction() {
		assert_eq!(Axis2::new(3.0, 4.0).normalized(), Axis2::new(0.6, 0.8));
		assert_eq!(Axis3::new(0.0, 3.0, 4.0).normalized(), Axis3::new(0.0, 0.6, 0.8));
	}
}
