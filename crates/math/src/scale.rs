use std::{
	convert::Infallible,
	ops::{Add, Div, Mul, Sub},
};

use maths_rs::Vec3f;

use crate::Vector;
use crate::serialization::{ArrayForm, serialize_as_array};

/// The `Scale` struct represents non-spatial scale factors for transforms.
///
/// Use [`Self::into_maths`] only when passing scale to a maths or rendering boundary.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
	value: Vec3f,
}

impl Scale {
	/// Creates scale factors for the x, y, and z axes.
	pub fn new(x: f32, y: f32, z: f32) -> Self {
		Self::from_maths(Vec3f::new(x, y, z))
	}

	/// Creates scale factors from their `[x, y, z]` components.
	pub fn from_array([x, y, z]: [f32; 3]) -> Self {
		Self::new(x, y, z)
	}

	/// Returns the `[x, y, z]` scale factors.
	pub fn to_array(self) -> [f32; 3] {
		[self.x(), self.y(), self.z()]
	}

	/// Creates scale factors that preserve an object's size.
	pub const fn identity() -> Self {
		Self {
			value: Vec3f { x: 1.0, y: 1.0, z: 1.0 },
		}
	}

	/// Moves `factor` of the way from these scale factors to `other`, axis by axis.
	pub fn lerp(self, other: Self, factor: f32) -> Self {
		self + (other - self) * factor
	}

	/// Creates scale factors from an explicit `maths-rs` value at an integration boundary.
	pub fn from_maths(value: Vec3f) -> Self {
		Self { value }
	}

	/// Returns these scale factors as an explicit `maths-rs` value for an integration boundary.
	pub fn into_maths(self) -> Vec3f {
		self.value
	}

	/// Returns the x-axis scale factor.
	pub fn x(self) -> f32 {
		self.value.x
	}

	/// Returns the y-axis scale factor.
	pub fn y(self) -> f32 {
		self.value.y
	}

	/// Returns the z-axis scale factor.
	pub fn z(self) -> f32 {
		self.value.z
	}
}

impl Default for Scale {
	fn default() -> Self {
		Self::identity()
	}
}

impl ArrayForm<3> for Scale {
	type Array = [f32; 3];
	type Error = Infallible;

	fn to_array(&self) -> Self::Array {
		Scale::to_array(*self)
	}

	fn try_from_array(array: Self::Array) -> Result<Self, Self::Error> {
		Ok(Self::from_array(array))
	}
}

serialize_as_array!(Scale, 3);

/// Adds scale factors axis by axis, such as an offset that eases one scale toward another.
impl Add for Scale {
	type Output = Self;

	fn add(self, other: Self) -> Self {
		Self::from_maths(self.value + other.value)
	}
}

/// Subtracts scale factors axis by axis.
impl Sub for Scale {
	type Output = Self;

	fn sub(self, other: Self) -> Self {
		Self::from_maths(self.value - other.value)
	}
}

/// Multiplies scale factors axis by axis, which composes two scales.
impl Mul for Scale {
	type Output = Self;

	fn mul(self, other: Self) -> Self {
		Self::from_maths(self.value * other.value)
	}
}

impl Mul<f32> for Scale {
	type Output = Self;

	fn mul(self, factor: f32) -> Self {
		Self::from_maths(self.value * factor)
	}
}

impl Div<f32> for Scale {
	type Output = Self;

	fn div(self, divisor: f32) -> Self {
		Self::from_maths(self.value / divisor)
	}
}

/// Scales a displacement axis by axis while preserving its coordinate-space brand.
impl<Space> Mul<Vector<Space>> for Scale {
	type Output = Vector<Space>;

	fn mul(self, vector: Vector<Space>) -> Vector<Space> {
		Vector::from_maths(self.value * vector.into_maths())
	}
}
