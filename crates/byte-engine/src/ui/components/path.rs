use std::sync::atomic::{AtomicU64, Ordering};

use super::curve::CurvePath;
use crate::ui::{Transform, Visual, layout::Sizing, style::ConcreteStyle};

static NEXT_PATH_ID: AtomicU64 = AtomicU64::new(1);

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
/// Points are in the path's own units. With a [`Self::view_box`], the element's box maps those
/// units onto its size; without one, they are layout units like a [`super::curve::Curve`].
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
	pub fn new(path: CurvePath) -> Self {
		Self {
			id: NEXT_PATH_ID.fetch_add(1, Ordering::Relaxed), // TODO: remove, have the engine emit these internally, same for images
			version: 0,
			path,
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

	/// Maps `width` by `height` path units onto the element's box.
	pub fn view_box(mut self, width: f32, height: f32) -> Self {
		self.view_box = Some([width, height]);
		self
	}

	pub fn fill_rule(mut self, fill_rule: FillRule) -> Self {
		self.fill_rule = fill_rule;
		self
	}

	pub fn size(self, sizing: Sizing) -> Self {
		self.width(sizing).height(sizing)
	}

	pub fn width(mut self, width: Sizing) -> Self {
		self.path.width = width;
		self
	}

	pub fn height(mut self, height: Sizing) -> Self {
		self.path.height = height;
		self
	}

	pub fn style(mut self, style: impl Into<ConcreteStyle>) -> Self {
		self.style = style.into();
		self
	}

	pub fn transform(mut self, transform: impl Into<Transform>) -> Self {
		self.transform = transform.into();
		self
	}

	pub fn opacity(mut self, opacity: f32) -> Self {
		self.visual.opacity = opacity;
		self
	}

	/// Replaces the outline. The path is packed again on its next draw.
	pub fn set_path(&mut self, path: CurvePath) {
		self.path = path;
		self.version = self.version.wrapping_add(1);
	}

	pub fn set_view_box(&mut self, view_box: Option<[f32; 2]>) {
		self.view_box = view_box;
	}

	pub fn set_fill_rule(&mut self, fill_rule: FillRule) {
		self.fill_rule = fill_rule;
	}

	/// Replaces the style in place; see [`ConcreteStyle::set_layers`].
	pub fn set_style(&mut self, style: impl AsRef<[crate::ui::style::ConcreteLayer]>) {
		self.style.set_layers(style);
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn set_opacity(&mut self, opacity: f32) {
		self.visual.opacity = opacity;
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
