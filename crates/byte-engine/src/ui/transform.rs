use super::{Location3, Size, UiPoint, layout::Geometry};

/// The `Transform` struct supports visual placement, scaling, and rotation after layout.
///
/// Transform edits update visual bounds, clipping, and pointer hits without
/// changing measured sizes or flow placement.
///
/// Scaling uses the element's center by default. Choose [`Self::origin`] when
/// an edge or corner must stay anchored, then pass the transform to
/// [`super::Container::transform`] or another UI primitive.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
	/// The horizontal displacement in layout units, applied after scaling.
	pub translate_x: f32,
	/// The vertical displacement in layout units, applied after scaling.
	pub translate_y: f32,
	/// The horizontal scale around `origin`.
	pub scale_x: f32,
	/// The vertical scale around `origin`.
	pub scale_y: f32,
	/// The clockwise turn around `origin` in radians, applied after scaling.
	///
	/// A turn is visual only: the element and its descendants draw rotated, while
	/// bounds, clipping, and pointer hits keep their unrotated placement.
	pub rotation: f32,
	/// The scaling and rotation pivot as a fraction of the element's laid-out width and height.
	///
	/// `(0, 0)` is the top left and `(1, 1)` is the bottom right. Values outside
	/// that range place the pivot outside the element. Nonfinite coordinates use
	/// `0.5`, the center, during visual evaluation.
	pub origin: UiPoint,
}

impl Transform {
	pub const IDENTITY: Self = Self {
		translate_x: 0.0,
		translate_y: 0.0,
		scale_x: 1.0,
		scale_y: 1.0,
		rotation: 0.0,
		origin: UiPoint::new(0.5, 0.5),
	};

	pub fn identity() -> Self {
		Self::default()
	}

	pub fn translate(mut self, x: f32, y: f32) -> Self {
		self.translate_x = x;
		self.translate_y = y;
		self
	}

	pub fn translate_x(mut self, x: f32) -> Self {
		self.translate_x = x;
		self
	}

	pub fn translate_y(mut self, y: f32) -> Self {
		self.translate_y = y;
		self
	}

	pub fn scale(mut self, scale: f32) -> Self {
		self.scale_x = scale;
		self.scale_y = scale;
		self
	}

	pub fn scale_xy(mut self, x: f32, y: f32) -> Self {
		self.scale_x = x;
		self.scale_y = y;
		self
	}

	/// Turns this element and its descendants clockwise around [`Self::origin`].
	pub fn rotate(mut self, radians: f32) -> Self {
		self.rotation = radians;
		self
	}

	/// Chooses the fractional pivot for scaling and rotating this element and its descendants.
	///
	/// Use [`UiPoint::zero`] to preserve the top-left corner while scaling. Next,
	/// set the scale with [`Self::scale`] or [`Self::scale_xy`].
	pub fn origin(mut self, origin: UiPoint) -> Self {
		self.origin = origin;
		self
	}
}

impl Default for Transform {
	fn default() -> Self {
		Self::IDENTITY
	}
}

/// The `Rotation` struct is the rigid turn that carries a subtree's unrotated placement to the screen.
///
/// Layout, clipping, and hits stay unrotated. A point is drawn at `R * point + (x, y)`, in layout units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Rotation {
	pub(crate) cos: f32,
	pub(crate) sin: f32,
	pub(crate) x: f32,
	pub(crate) y: f32,
}

impl Rotation {
	pub(crate) const IDENTITY: Self = Self {
		cos: 1.0,
		sin: 0.0,
		x: 0.0,
		y: 0.0,
	};

	/// A clockwise turn around a pivot.
	pub(crate) fn about(radians: f32, pivot_x: f32, pivot_y: f32) -> Self {
		if !radians.is_finite() || radians == 0.0 {
			return Self::IDENTITY;
		}
		let (sin, cos) = radians.sin_cos();
		Self {
			cos,
			sin,
			x: pivot_x - (cos * pivot_x - sin * pivot_y),
			y: pivot_y - (sin * pivot_x + cos * pivot_y),
		}
	}

	pub(crate) fn is_identity(&self) -> bool {
		*self == Self::IDENTITY
	}

	pub(crate) fn apply(&self, x: f32, y: f32) -> (f32, f32) {
		(self.cos * x - self.sin * y + self.x, self.sin * x + self.cos * y + self.y)
	}

	/// The turn that applies `inner` first and this one after it.
	pub(crate) fn after(&self, inner: Self) -> Self {
		let (x, y) = self.apply(inner.x, inner.y);
		Self {
			cos: self.cos * inner.cos - self.sin * inner.sin,
			sin: self.sin * inner.cos + self.cos * inner.sin,
			x,
			y,
		}
	}

	/// The same turn in a space whose axes are scaled, as layout units are to pixels.
	pub(crate) fn scaled(self, sx: f32, sy: f32) -> Self {
		Self {
			x: self.x * sx,
			y: self.y * sy,
			..self
		}
	}

	/// The axis-aligned bounds of a turned rectangle, as `[x0, y0, x1, y1]`.
	pub(crate) fn bounds(&self, [x0, y0, x1, y1]: [f32; 4]) -> [f32; 4] {
		let corners = [self.apply(x0, y0), self.apply(x1, y0), self.apply(x1, y1), self.apply(x0, y1)];
		corners.iter().fold(
			[f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY],
			|bounds, &(x, y)| [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)],
		)
	}

	/// The axis-aligned bounds of a turned geometry, at its depth.
	pub(crate) fn geometry(&self, geometry: Geometry) -> Geometry {
		let [x0, y0, x1, y1] = self.bounds([
			geometry.x(),
			geometry.y(),
			geometry.x() + geometry.width(),
			geometry.y() + geometry.height(),
		]);
		Geometry::new(Location3::new(x0, y0, geometry.position.z()), Size::new(x1 - x0, y1 - y0))
	}
}
