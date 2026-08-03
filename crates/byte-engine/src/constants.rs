//! Common directions in the engine's default coordinate system.

use math::UnitVector;

/// Returns the direction that points right along the positive x-axis.
pub fn right() -> UnitVector {
	UnitVector::x_axis()
}

/// Returns the direction that points up along the positive y-axis.
pub fn up() -> UnitVector {
	UnitVector::y_axis()
}

/// Returns the direction that points forward along the positive z-axis.
pub fn forward() -> UnitVector {
	UnitVector::z_axis()
}
