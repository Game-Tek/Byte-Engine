use super::super::{
	flow,
	layout::{Depth, Position, Sizing},
};
use crate::ui::{
	Transform, Visual,
	flow::{FlowInput, FlowOutput},
	style::ConcreteStyle,
};

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
	pub(crate) min_width: Option<Sizing>,
	pub(crate) min_height: Option<Sizing>,
	pub width: Sizing,
	pub height: Sizing,
	pub corner_radius: f32,
	pub corner_exponent: f32,
	pub sector: Option<Sector>,
	pub(crate) max_width: Option<Sizing>,
	pub(crate) max_height: Option<Sizing>,
	pub depth: Depth,
	pub position: Position,
	pub clip: bool,
	pub(crate) hit_testable: bool,
	pub flow: utils::InlineCopyFn<fn(FlowInput) -> FlowOutput>,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

/// Every fixed-size container property, compared to prove an edit changed nothing.
#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ContainerProperties {
	min_width: Option<Sizing>,
	min_height: Option<Sizing>,
	width: Sizing,
	height: Sizing,
	corner_radius: f32,
	corner_exponent: f32,
	sector: Option<Sector>,
	max_width: Option<Sizing>,
	max_height: Option<Sizing>,
	depth: Depth,
	position: Position,
	clip: bool,
	hit_testable: bool,
	flow: (std::any::TypeId, FlowOutput),
}

impl Container {
	/// Returns the fixed-size properties, or `None` when a custom flow keeps them from being compared.
	pub(crate) fn properties(&self) -> Option<ContainerProperties> {
		Some(ContainerProperties {
			min_width: self.min_width,
			min_height: self.min_height,
			width: self.width,
			height: self.height,
			corner_radius: self.corner_radius,
			corner_exponent: self.corner_exponent,
			sector: self.sector,
			max_width: self.max_width,
			max_height: self.max_height,
			depth: self.depth,
			position: self.position,
			clip: self.clip,
			hit_testable: self.hit_testable,
			flow: flow::placement_key(&self.flow)?,
		})
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

impl Default for Container {
	fn default() -> Self {
		Self {
			width: Sizing::full(),
			height: Sizing::full(),
			corner_radius: 0.0,
			corner_exponent: 2.0,
			sector: None,
			min_width: None,
			min_height: None,
			max_width: None,
			max_height: None,
			depth: Depth::default(),
			position: Position::default(),
			clip: true,
			hit_testable: true,
			flow: utils::InlineCopyFn::<fn(FlowInput) -> FlowOutput>::new(flow::grid),
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
	}
}
