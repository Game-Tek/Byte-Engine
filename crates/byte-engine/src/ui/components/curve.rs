use std::alloc::Allocator;

use crate::ui::{flow::Size, layout::Sizing, style::ConcreteStyle, transform::Transform, visual::Visual};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurvePoint {
	pub x: f32,
	pub y: f32,
}

impl CurvePoint {
	pub fn new(x: f32, y: f32) -> Self {
		Self { x, y }
	}

	pub(crate) fn is_finite(self) -> bool {
		self.x.is_finite() && self.y.is_finite()
	}
}

impl From<(f32, f32)> for CurvePoint {
	fn from(value: (f32, f32)) -> Self {
		Self::new(value.0, value.1)
	}
}

#[derive(Debug, Clone, PartialEq)]
pub enum CurveSegment {
	Line {
		from: CurvePoint,
		to: CurvePoint,
	},
	Quadratic {
		from: CurvePoint,
		control: CurvePoint,
		to: CurvePoint,
	},
	Cubic {
		from: CurvePoint,
		control0: CurvePoint,
		control1: CurvePoint,
		to: CurvePoint,
	},
}

impl CurveSegment {
	/// Appends the segment as a polyline, mapping each control point first and
	/// subdividing until the curve stays within `tolerance` of its spans.
	///
	/// Rendering maps points into pixels and hit testing into layout units, so
	/// both flatten the same way. Non-finite points are skipped.
	pub(crate) fn flatten<A: Allocator>(
		&self,
		map: impl Fn(CurvePoint) -> CurvePoint,
		tolerance: f32,
		points: &mut Vec<CurvePoint, A>,
	) {
		match *self {
			CurveSegment::Line { from, to } => {
				for point in [map(from), map(to)] {
					if point.is_finite() {
						points.push(point);
					}
				}
			}
			CurveSegment::Quadratic { from, control, to } => {
				let (from, control, to) = (map(from), map(control), map(to));
				if from.is_finite() && control.is_finite() && to.is_finite() {
					points.push(from);
					flatten_quadratic(from, control, to, tolerance, 0, points);
				}
			}
			CurveSegment::Cubic {
				from,
				control0,
				control1,
				to,
			} => {
				let (from, control0, control1, to) = (map(from), map(control0), map(control1), map(to));
				if from.is_finite() && control0.is_finite() && control1.is_finite() && to.is_finite() {
					points.push(from);
					flatten_cubic(from, control0, control1, to, tolerance, 0, points);
				}
			}
		}
	}
}

fn flatten_quadratic<A: Allocator>(
	from: CurvePoint,
	control: CurvePoint,
	to: CurvePoint,
	tolerance: f32,
	depth: u32,
	points: &mut Vec<CurvePoint, A>,
) {
	if depth >= 12 || point_line_distance(control, from, to) <= tolerance {
		points.push(to);
		return;
	}

	let from_control = midpoint(from, control);
	let control_to = midpoint(control, to);
	let mid = midpoint(from_control, control_to);
	flatten_quadratic(from, from_control, mid, tolerance, depth + 1, points);
	flatten_quadratic(mid, control_to, to, tolerance, depth + 1, points);
}

fn flatten_cubic<A: Allocator>(
	from: CurvePoint,
	control0: CurvePoint,
	control1: CurvePoint,
	to: CurvePoint,
	tolerance: f32,
	depth: u32,
	points: &mut Vec<CurvePoint, A>,
) {
	if depth >= 12 || point_line_distance(control0, from, to).max(point_line_distance(control1, from, to)) <= tolerance {
		points.push(to);
		return;
	}

	let p01 = midpoint(from, control0);
	let p12 = midpoint(control0, control1);
	let p23 = midpoint(control1, to);
	let p012 = midpoint(p01, p12);
	let p123 = midpoint(p12, p23);
	let mid = midpoint(p012, p123);
	flatten_cubic(from, p01, p012, mid, tolerance, depth + 1, points);
	flatten_cubic(mid, p123, p23, to, tolerance, depth + 1, points);
}

fn midpoint(a: CurvePoint, b: CurvePoint) -> CurvePoint {
	CurvePoint::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)
}

/// Distance from `point` to the infinite line through `from` and `to`.
fn point_line_distance(point: CurvePoint, from: CurvePoint, to: CurvePoint) -> f32 {
	let dx = to.x - from.x;
	let dy = to.y - from.y;
	let length = dx.hypot(dy);
	if length <= 0.0001 {
		return (point.x - from.x).hypot(point.y - from.y);
	}
	((point.x - from.x) * dy - (point.y - from.y) * dx).abs() / length
}

#[derive(Debug, Clone, PartialEq)]
pub struct CurvePath {
	pub(crate) segments: Vec<CurveSegment>,
	pub(crate) width: Sizing,
	pub(crate) height: Sizing,
}

impl CurvePath {
	pub fn new(width: Sizing, height: Sizing) -> Self {
		Self {
			segments: Vec::new(),
			width,
			height,
		}
	}

	pub fn line(mut self, from: impl Into<CurvePoint>, to: impl Into<CurvePoint>) -> Self {
		self.segments.push(CurveSegment::Line {
			from: from.into(),
			to: to.into(),
		});
		self
	}

	pub fn quadratic(mut self, from: impl Into<CurvePoint>, control: impl Into<CurvePoint>, to: impl Into<CurvePoint>) -> Self {
		self.segments.push(CurveSegment::Quadratic {
			from: from.into(),
			control: control.into(),
			to: to.into(),
		});
		self
	}

	pub fn cubic(
		mut self,
		from: impl Into<CurvePoint>,
		control0: impl Into<CurvePoint>,
		control1: impl Into<CurvePoint>,
		to: impl Into<CurvePoint>,
	) -> Self {
		self.segments.push(CurveSegment::Cubic {
			from: from.into(),
			control0: control0.into(),
			control1: control1.into(),
			to: to.into(),
		});
		self
	}

	pub fn from_segments(width: Sizing, height: Sizing, segments: impl IntoIterator<Item = CurveSegment>) -> Self {
		Self {
			segments: segments.into_iter().collect(),
			width,
			height,
		}
	}

	/// Drops every segment while keeping the buffer, so a re-routed path allocates nothing.
	pub fn clear(&mut self) {
		self.segments.clear();
	}

	pub fn push(&mut self, segment: CurveSegment) {
		self.segments.push(segment);
	}

	pub fn push_line(&mut self, from: impl Into<CurvePoint>, to: impl Into<CurvePoint>) {
		self.push(CurveSegment::Line {
			from: from.into(),
			to: to.into(),
		});
	}

	pub fn push_cubic(
		&mut self,
		from: impl Into<CurvePoint>,
		control0: impl Into<CurvePoint>,
		control1: impl Into<CurvePoint>,
		to: impl Into<CurvePoint>,
	) {
		self.push(CurveSegment::Cubic {
			from: from.into(),
			control0: control0.into(),
			control1: control1.into(),
			to: to.into(),
		});
	}

	pub fn set_size(&mut self, width: Sizing, height: Sizing) {
		self.width = width;
		self.height = height;
	}

	pub fn size(&self, available_space: Size) -> Size {
		Size::new(
			self.width.calculate(available_space.x()),
			self.height.calculate(available_space.y()),
		)
	}

	pub fn segments(&self) -> &[CurveSegment] {
		&self.segments
	}
}

/// The `Curve` struct is the retained state of a stroked open outline, such as a wire between two nodes.
///
/// The engine owns every curve. Declare one with [`crate::ui::ElementContext::curve`] and edit it with
/// [`crate::ui::EvaluationContext::update_curve`].
pub struct Curve {
	pub(crate) path: CurvePath,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
	pub(crate) hit_width: Option<f32>,
}

impl Curve {
	/// Creates a full-size curve with no segments.
	pub(crate) fn new() -> Self {
		Self {
			path: CurvePath::new(Sizing::full(), Sizing::full()),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
			hit_width: None,
		}
	}

	/// Curves repaint from their segments on every change, so there is nothing to record; see
	/// [`super::path::Path::outline_changed`], which a path needs to repack.
	pub(crate) fn outline_changed(&mut self) {}

	pub fn hit_width(&self) -> Option<f32> {
		self.hit_width
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

/// The `FlattenedCurve` struct retains local points for translated curves.
/// A scale or path edit refreshes tessellation at the caller's tolerance.
#[derive(Default)]
pub(crate) struct FlattenedCurve {
	segments: Vec<CurveSegment>,
	scale: [f32; 2],
	tolerance: f32,
	pub(crate) points: Vec<CurvePoint>,
	pub(crate) ranges: Vec<std::ops::Range<usize>>,
}

impl FlattenedCurve {
	/// Refreshes local points only when the path or its effective scale changes.
	pub(crate) fn update(&mut self, segments: &[CurveSegment], scale: [f32; 2], tolerance: f32) {
		if self.segments == segments && self.scale == scale && self.tolerance == tolerance {
			return;
		}
		self.segments.clear();
		self.segments.extend_from_slice(segments);
		self.scale = scale;
		self.tolerance = tolerance;
		self.points.clear();
		self.ranges.clear();
		for segment in segments {
			let start = self.points.len();
			segment.flatten(
				|point| CurvePoint::new(point.x * scale[0], point.y * scale[1]),
				tolerance,
				&mut self.points,
			);
			self.ranges.push(start..self.points.len());
		}
	}
}
