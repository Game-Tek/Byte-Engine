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

pub struct Curve {
	pub(crate) path: CurvePath,
	pub(crate) style: ConcreteStyle,
	pub(crate) transform: Transform,
	pub(crate) visual: Visual,
}

impl Curve {
	pub fn new(path: CurvePath) -> Self {
		Self {
			path,
			style: ConcreteStyle::default(),
			transform: Transform::default(),
			visual: Visual::default(),
		}
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

	pub fn set_style(&mut self, style: impl Into<ConcreteStyle>) {
		self.style = style.into();
	}

	pub fn set_transform(&mut self, transform: impl Into<Transform>) {
		self.transform = transform.into();
	}

	pub fn set_opacity(&mut self, opacity: f32) {
		self.visual.opacity = opacity;
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
