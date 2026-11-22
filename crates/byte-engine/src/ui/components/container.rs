use super::super::{
	flow,
	layout::{Depth, Position, Sizing},
};
use crate::ui::flow::{FlowInput, FlowOutput, Size};

/// An annular sector a container is shaped as instead of a rounded rectangle.
///
/// The sector is centered in the container and its outer radius is half the shorter side.
/// Angles are radians; zero points right and they grow clockwise on screen. A `sweep` of a
/// full turn or more is a ring, and `inner` is the hole radius as a ratio of the outer radius,
/// where zero makes a pie slice. `inset` pulls both straight edges inward by that many layout
/// units, so neighboring sectors keep a constant gap from hub to rim instead of an angular one.
/// Painting and pointer hits follow the sector; a clipping sector still masks its
/// descendants to its rectangle and corner radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sector {
	pub start: f32,
	pub sweep: f32,
	pub inner: f32,
	pub inset: f32,
}

impl Sector {
	pub const FULL_TURN: f32 = std::f32::consts::TAU;

	pub fn new(start: f32, sweep: f32, inner: f32) -> Self {
		Self {
			start,
			sweep,
			inner,
			inset: 0.0,
		}
	}

	pub fn inset(self, inset: f32) -> Self {
		Self { inset, ..self }
	}

	/// Reports whether a point, measured from the sector's center with the outer radius given, lies in it.
	pub fn contains(&self, dx: f32, dy: f32, outer: f32) -> bool {
		let radius = dx.hypot(dy);
		if radius > outer || radius < self.inner.clamp(0.0, 1.0) * outer {
			return false;
		}
		if self.sweep >= Self::FULL_TURN {
			return true;
		}
		let sweep = self.sweep.max(0.0);
		let angle = (dy.atan2(dx) - self.start).rem_euclid(Self::FULL_TURN);
		if angle >= sweep {
			return false;
		}
		// Inside the wedge, the point must also clear both straight edges by the inset.
		let inset = self.inset.max(0.0);
		if inset <= 0.0 {
			return true;
		}
		[self.start, self.start + sweep].iter().all(|edge| {
			let (ex, ey) = (edge.cos(), edge.sin());
			let along = (dx * ex + dy * ey).max(0.0);
			(dx - ex * along).hypot(dy - ey * along) >= inset
		})
	}
}

/// The `Container` struct is the retained state of a box that lays out, clips, and paints its children.
///
/// The engine owns every container. Declare one with [`crate::ui::ElementContext::container`] and edit it with
/// [`crate::ui::EvaluationContext::update_container`]; both hand you [`crate::ui::Properties`] setters.
pub struct Container {
	/// The logical width: the space the container takes in its parent's flow and offers its children.
	pub width: Sizing,
	/// The logical height; see [`Self::width`].
	pub height: Sizing,
	/// The display width: the width the container is painted and clips its descendants at, resolved against the
	/// logical width and anchored at its top-left corner. Animate it instead of [`Self::width`] to resize what is seen
	/// without moving the children or the hit box.
	pub display_width: Sizing,
	/// The display height; see [`Self::display_width`].
	pub display_height: Sizing,
	/// The corner radii, top left, top right, bottom right, then bottom left, in layout units; see [`fitted_radii`].
	pub corner_radii: [f32; 4],
	pub corner_exponent: f32,
	pub sector: Option<Sector>,
	pub depth: Depth,
	pub position: Position,
	pub clip: bool,
	pub(crate) hit_testable: bool,
	/// The room kept inside the left and right edges, then inside the top and bottom ones, before the flow children.
	pub padding: Size,
	pub flow: utils::InlineCopyFn<FlowInput, FlowOutput>,
	/// The point of the box that looks like its middle, as fractions of its width and height from its top-left corner,
	/// which centering flows line up; `None` for the middle. See [Visual center](/docs/reference/gui#visual-center).
	pub visual_center: Option<[f32; 2]>,
}

/// Every fixed-size container property, compared to prove an edit changed nothing.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ContainerProperties {
	width: Sizing,
	height: Sizing,
	display_width: Sizing,
	display_height: Sizing,
	corner_radii: [f32; 4],
	corner_exponent: f32,
	sector: Option<Sector>,
	depth: Depth,
	position: Position,
	clip: bool,
	hit_testable: bool,
	padding: Size,
	flow: (std::any::TypeId, FlowOutput),
	visual_center: Option<[f32; 2]>,
}

impl Container {
	/// Returns the fixed-size properties, or `None` when a custom flow keeps them from being compared.
	pub(crate) fn properties(&self) -> Option<ContainerProperties> {
		Some(ContainerProperties {
			width: self.width,
			height: self.height,
			display_width: self.display_width,
			display_height: self.display_height,
			corner_radii: self.corner_radii,
			corner_exponent: self.corner_exponent,
			sector: self.sector,
			depth: self.depth,
			position: self.position,
			clip: self.clip,
			hit_testable: self.hit_testable,
			padding: self.padding,
			flow: flow::placement_key(&self.flow)?,
			visual_center: self.visual_center,
		})
	}
}

impl Default for Container {
	fn default() -> Self {
		Self {
			width: Sizing::full(),
			height: Sizing::full(),
			display_width: Sizing::full(),
			display_height: Sizing::full(),
			corner_radii: [0.0; 4],
			corner_exponent: 2.0,
			sector: None,
			depth: Depth::default(),
			position: Position::default(),
			clip: true,
			hit_testable: false,
			padding: Size::new(0, 0),
			flow: utils::InlineCopyFn::<FlowInput, FlowOutput>::new(flow::grid),
			visual_center: None,
		}
	}
}

/// Fits corner `radii`, top left, top right, bottom right, bottom left, to a `width` by `height` box as CSS does: a
/// negative or non-finite radius counts as 0, and where two radii on one side add up to more than that side, all four
/// scale down by the same factor until they fit.
pub(crate) fn fitted_radii(radii: [f32; 4], width: f32, height: f32) -> [f32; 4] {
	let radii = radii.map(|radius| if radius.is_finite() { radius.max(0.0) } else { 0.0 });
	let [top_left, top_right, bottom_right, bottom_left] = radii;
	// Each side's length over the radii that share it; a side without radii needs no scaling.
	let fit = |length: f32, sum: f32| if sum > 0.0 { length.max(0.0) / sum } else { f32::INFINITY };
	let factor = fit(width, top_left + top_right)
		.min(fit(width, bottom_left + bottom_right))
		.min(fit(height, top_left + bottom_left))
		.min(fit(height, top_right + bottom_right))
		.min(1.0);
	radii.map(|radius| radius * factor)
}

/// The corners of a border's inner edge, as CSS draws them, for outer corner `radii` and side `widths`, top, right,
/// bottom, left: each corner is elliptical, its outer radius less the adjacent left or right width across and less the
/// adjacent top or bottom width down, each at least 0. Returns (across, down) for top left, top right, bottom right,
/// then bottom left.
pub(crate) fn inner_corners(radii: [f32; 4], widths: [f32; 4]) -> [[f32; 2]; 4] {
	let [top, right, bottom, left] = widths;
	let inner = |radius: f32, across: f32, down: f32| [(radius - across).max(0.0), (radius - down).max(0.0)];
	[
		inner(radii[0], left, top),
		inner(radii[1], right, top),
		inner(radii[2], right, bottom),
		inner(radii[3], left, bottom),
	]
}

#[cfg(test)]
mod corner_tests {
	use super::*;

	#[test]
	fn radii_that_overlap_on_a_side_scale_down_together() {
		assert_eq!(
			fitted_radii([10.0; 4], 100.0, 100.0),
			[10.0; 4],
			"radii that fit keep their size"
		);
		assert_eq!(
			fitted_radii([40.0; 4], 60.0, 100.0),
			[30.0; 4],
			"a uniform radius stops at half the shorter side"
		);
		assert_eq!(fitted_radii([60.0, 40.0, 0.0, 0.0], 50.0, 100.0), [30.0, 20.0, 0.0, 0.0]);
		assert_eq!(fitted_radii([-4.0, f32::NAN, 3.0, 0.0], 10.0, 10.0), [0.0, 0.0, 3.0, 0.0]);
	}

	#[test]
	fn an_inner_corner_loses_each_adjacent_width_on_its_own_axis() {
		assert_eq!(
			inner_corners([10.0; 4], [2.0, 4.0, 6.0, 8.0]),
			[[2.0, 8.0], [6.0, 8.0], [6.0, 4.0], [2.0, 4.0]]
		);
		assert_eq!(
			inner_corners([3.0; 4], [5.0; 4]),
			[[0.0, 0.0]; 4],
			"a wide border squares the inner corner"
		);
	}
}
