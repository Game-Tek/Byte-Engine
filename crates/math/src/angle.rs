/// The `Degrees` struct provides an angular value measured in degrees at degree-based API boundaries.
///
/// Create a value with [`Self::new`]. Convert it to [`Radians`] before using trigonometric methods.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Degrees(f32);

impl Degrees {
	/// Creates an angular value measured in degrees.
	pub const fn new(value: f32) -> Self {
		Self(value)
	}

	/// Returns the numeric value in degrees.
	pub const fn value(self) -> f32 {
		self.0
	}

	/// Converts this angle to radians.
	pub fn to_radians(self) -> Radians {
		Radians(self.0.to_radians())
	}

	/// Returns whether the angle is neither infinite nor NaN.
	pub fn is_finite(self) -> bool {
		self.0.is_finite()
	}
}

/// The `Radians` struct provides an angular value measured in radians for rotation and trigonometry.
///
/// Create a value with [`Self::new`]. Use [`Self::sin`], [`Self::cos`], or [`Self::tan`] without
/// unwrapping it, or convert it to [`Degrees`] for a degree-based boundary.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, PartialOrd)]
pub struct Radians(f32);

impl Radians {
	/// Creates an angular value measured in radians.
	pub const fn new(value: f32) -> Self {
		Self(value)
	}

	/// Returns the numeric value in radians.
	pub const fn value(self) -> f32 {
		self.0
	}

	/// Converts this angle to degrees.
	pub fn to_degrees(self) -> Degrees {
		Degrees(self.0.to_degrees())
	}

	/// Returns the sine of this angle.
	pub fn sin(self) -> f32 {
		self.0.sin()
	}

	/// Returns the cosine of this angle.
	pub fn cos(self) -> f32 {
		self.0.cos()
	}

	/// Returns the tangent of this angle.
	pub fn tan(self) -> f32 {
		self.0.tan()
	}

	/// Returns whether the angle is neither infinite nor NaN.
	pub fn is_finite(self) -> bool {
		self.0.is_finite()
	}
}

impl From<Degrees> for Radians {
	fn from(value: Degrees) -> Self {
		value.to_radians()
	}
}

impl From<Radians> for Degrees {
	fn from(value: Radians) -> Self {
		value.to_degrees()
	}
}

impl std::ops::Mul<f32> for Radians {
	type Output = Self;

	fn mul(self, scale: f32) -> Self::Output {
		Self(self.0 * scale)
	}
}

impl std::ops::Mul<f32> for Degrees {
	type Output = Self;

	fn mul(self, scale: f32) -> Self::Output {
		Self(self.0 * scale)
	}
}

#[cfg(test)]
mod tests {
	use super::{Degrees, Radians};

	#[test]
	fn degree_and_radian_conversions_preserve_a_quarter_turn() {
		let radians = Degrees::new(90.0).to_radians();

		crate::assert_float_eq!(radians.value(), std::f32::consts::FRAC_PI_2);
		crate::assert_float_eq!(radians.to_degrees().value(), 90.0);
		crate::assert_float_eq!(Radians::new(std::f32::consts::FRAC_PI_2).sin(), 1.0);
	}
}
