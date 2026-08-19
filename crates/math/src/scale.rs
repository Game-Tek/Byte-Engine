use maths_rs::Vec3f;

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

	/// Creates scale factors that preserve an object's size.
	pub fn identity() -> Self {
		Self::new(1.0, 1.0, 1.0)
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

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	use super::Scale;

	#[test]
	fn identity_has_unit_factors() {
		assert_eq!(Scale::identity(), Scale::new(1.0, 1.0, 1.0));
	}

	#[test]
	fn maths_conversion_is_explicit_and_lossless() {
		let scale = Scale::from_maths(Vec3f::new(2.0, 3.0, 4.0));

		assert_eq!((scale.x(), scale.y(), scale.z()), (2.0, 3.0, 4.0));
		assert_eq!(scale.into_maths(), Vec3f::new(2.0, 3.0, 4.0));
	}
}
