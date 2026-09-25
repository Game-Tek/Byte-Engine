use super::curve::CurvePath;
use crate::ui::{Transform, Visual, layout::Sizing, style::ConcreteStyle};

/// How a point's winding number decides whether it lies inside a [`Path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FillRule {
	/// Inside where the winding number is not zero.
	#[default]
	NonZero,
	/// Inside where the winding number is odd. Resolved when the path is packed by
	/// orienting nested contours against their parents, so it is exact for contours
	/// that do not cross themselves or each other.
	EvenOdd,
}

/// A filled outline. Every run of joined segments of its [`CurvePath`] is one contour, and a
/// contour is always closed. The GPU computes coverage per pixel from the packed curves, so a
/// path stays sharp at any size and a size change uploads nothing.
///
/// Points are in the path's own units. With a view box, set by [`crate::ui::Properties::view_box`], the element's
/// box maps those units onto its size; without one, they are layout units like a [`super::curve::Curve`].
///
/// The engine owns every path. Declare one with [`crate::ui::ElementContext::path`] and edit it with
/// [`crate::ui::EvaluationContext::update_path`].
pub struct Path {
	id: u64,
	version: u64,
	pub(crate) path: CurvePath,
	pub(crate) view_box: Option<[f32; 2]>,
	pub(crate) fill_rule: FillRule,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Path {
	/// Creates a full-size path with no contours. `id` must differ from every other path's, so render caches can
	/// tell outlines apart without comparing segments.
	pub(crate) fn new(id: u64) -> Self {
		Self {
			id,
			version: 0,
			path: CurvePath::new(Sizing::full(), Sizing::full()),
			view_box: None,
			fill_rule: FillRule::NonZero,
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
	}

	/// Identifies the packed outline without comparing segments.
	pub(crate) fn content_key(&self) -> (u64, u64, FillRule) {
		(self.id, self.version, self.fill_rule)
	}

	/// Records that the segments changed, so the path is packed again on its next draw.
	pub(crate) fn outline_changed(&mut self) {
		self.version = self.version.wrapping_add(1);
	}

	pub fn id(&self) -> u64 {
		self.id
	}

	pub fn version(&self) -> u64 {
		self.version
	}

	pub fn path(&self) -> &CurvePath {
		&self.path
	}

	pub fn style_ref(&self) -> &ConcreteStyle {
		&self.style
	}

	pub fn transform_ref(&self) -> &Transform {
		&self.transform
	}

	pub fn visual_ref(&self) -> &Visual {
		&self.visual
	}
}
