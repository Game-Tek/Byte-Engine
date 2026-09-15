use super::UiPoint;

/// The `Transform` struct supports visual placement and scaling after layout.
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
	/// The scaling pivot as a fraction of the element's laid-out width and height.
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

	/// Chooses the fractional pivot for scaling this element and its descendants.
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
