use super::super::{
	flow::{self, FlowFunction},
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

pub struct Container {
	min_width: Option<Sizing>,
	min_height: Option<Sizing>,
	pub width: Sizing,
	pub height: Sizing,
	pub corner_radius: f32,
	pub corner_exponent: f32,
	pub sector: Option<Sector>,
	max_width: Option<Sizing>,
	max_height: Option<Sizing>,
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

	/// Selects whether this surface participates in pointer hit testing.
	/// Disable this for decorative roots; children retain their own policy.
	pub fn hit_testable(mut self, enabled: bool) -> Self {
		self.hit_testable = enabled;
		self
	}

	pub fn size(self, sizing: Sizing) -> Self {
		Self {
			width: sizing,
			height: sizing,
			..self
		}
	}

	pub fn width(self, width: Sizing) -> Self {
		Self { width, ..self }
	}

	pub fn height(self, height: Sizing) -> Self {
		Self { height, ..self }
	}

	pub fn corner_radius(self, corner_radius: f32) -> Self {
		Self { corner_radius, ..self }
	}

	pub fn corner_exponent(self, corner_exponent: f32) -> Self {
		Self { corner_exponent, ..self }
	}

	/// Shapes this container as an annular sector; see [`Sector`] for the parameters.
	pub fn sector(self, sector: Sector) -> Self {
		Self {
			sector: Some(sector),
			..self
		}
	}

	pub fn min_width(self, min_width: Sizing) -> Self {
		Self {
			min_width: Some(min_width),
			..self
		}
	}

	pub fn min_height(self, min_height: Sizing) -> Self {
		Self {
			min_height: Some(min_height),
			..self
		}
	}

	pub fn max_width(self, max_width: Sizing) -> Self {
		Self {
			max_width: Some(max_width),
			..self
		}
	}

	pub fn max_height(self, max_height: Sizing) -> Self {
		Self {
			max_height: Some(max_height),
			..self
		}
	}

	pub fn depth(self, depth: impl Into<Depth>) -> Self {
		Self {
			depth: depth.into(),
			..self
		}
	}

	pub fn position(self, position: impl Into<Position>) -> Self {
		Self {
			position: position.into(),
			..self
		}
	}

	/// Places this container at an offset from its parent's top-left corner
	/// instead of in the parent's flow. A container with [`Depth::absolute`]
	/// is placed from the viewport's corner instead.
	pub fn absolute_position(self, x: impl Into<f64>, y: impl Into<f64>) -> Self {
		self.position(Position::absolute(x, y))
	}

	pub fn clip(self, enabled: bool) -> Self {
		Self { clip: enabled, ..self }
	}

	pub fn flow(self, flow: impl FlowFunction + 'static) -> Self {
		Self {
			flow: utils::InlineCopyFn::<fn(FlowInput) -> FlowOutput>::new(flow),
			..self
		}
	}

	pub fn style(self, style: impl Into<ConcreteStyle>) -> Self {
		Self {
			style: style.into(),
			..self
		}
	}

	pub fn transform(self, transform: impl Into<Transform>) -> Self {
		Self {
			transform: transform.into(),
			..self
		}
	}

	pub fn opacity(self, opacity: f32) -> Self {
		Self {
			visual: Visual::opacity(opacity),
			..self
		}
	}

	/// Replaces the style in place; see [`ConcreteStyle::set_layers`].
	pub fn set_style(&mut self, style: impl AsRef<[crate::ui::style::ConcreteLayer]>) {
		self.style.set_layers(style);
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn set_position(&mut self, position: impl Into<Position>) {
		self.position = position.into();
	}

	pub fn set_clip(&mut self, enabled: bool) {
		self.clip = enabled;
	}

	pub fn set_hit_testable(&mut self, enabled: bool) {
		self.hit_testable = enabled;
	}

	pub fn set_opacity(&mut self, opacity: f32) {
		self.visual.opacity = opacity;
	}

	pub fn set_sector(&mut self, sector: Option<Sector>) {
		self.sector = sector;
	}

	pub fn set_corner_exponent(&mut self, corner_exponent: f32) {
		self.corner_exponent = corner_exponent;
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
