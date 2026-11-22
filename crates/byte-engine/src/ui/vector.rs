//! Vector images: pictures made of filled shapes, such as icons, which the UI draws sharp at any size.
//!
//! Load one baked from an SVG file with [`load_vector`], or build a [`Vector`] in code, and draw it with
//! [`draw_vector`]. See [Draw vector images](/docs/use/gui#draw-vector-images).

use resource_management::resources::vector::VectorSegment;

use super::layout::{
	context::{Context, ElementContext},
	engine::EvaluationContext,
};
use super::{Color, ConcreteLayer, CurvePoint, CurveSegment, FillRule, FlowInput, FlowOutput, LinearGradient};
use crate::utils::RGBA;

/// The `Vector` struct holds a picture made of filled shapes, such as an icon, that the UI draws sharp at any size.
///
/// Load one baked from an SVG file with [`load_vector`], or build one in code with [`Vector::new`], and draw it with
/// [`draw_vector`]. Its layers are drawn in a view box of their own units, as an SVG is drawn in its `viewBox`, which
/// the drawing maps onto the size it's given. It knows where it looks centered, so centering flows line it up by how
/// it looks rather than by its box; see [Visual center](/docs/reference/gui#visual-center).
#[derive(Clone, PartialEq)]
pub struct Vector {
	view_box: [f32; 2],
	layers: Vec<VectorLayer>,
	/// How far the optical center of the area the layers cover sits from the midpoint of that area's bounds, in view
	/// box units; `None` when the layers cover no area.
	optical_shift: Option<[f32; 2]>,
}

impl Vector {
	/// Builds a vector image from `layers`, back to front, drawn in a box `view_box` units wide and tall, and measures
	/// where it looks centered.
	pub fn new(view_box: [f32; 2], layers: Vec<VectorLayer>) -> Self {
		let outlines: Vec<(Vec<VectorSegment>, bool)> = layers
			.iter()
			.map(|layer| {
				let segments = layer.segments.iter().map(vector_segment).collect();
				(segments, layer.fill_rule == FillRule::EvenOdd)
			})
			.collect();
		let optical_shift = resource_management::resources::vector::optical_shift(
			outlines.iter().map(|(segments, even_odd)| (&segments[..], *even_odd)),
		);
		Self {
			view_box,
			layers,
			optical_shift,
		}
	}

	/// The width and height of the box the layers are drawn in.
	pub fn view_box(&self) -> [f32; 2] {
		self.view_box
	}

	/// The shapes, from back to front.
	pub fn layers(&self) -> &[VectorLayer] {
		&self.layers
	}

	/// How far the point where the picture looks centered sits from the midpoint of what it covers, in view box units;
	/// `None` when it covers no area.
	pub fn optical_shift(&self) -> Option<[f32; 2]> {
		self.optical_shift
	}
}

/// A UI curve segment as the resource store writes it, to measure with the bake's own geometry.
fn vector_segment(segment: &CurveSegment) -> VectorSegment {
	let point = |point: CurvePoint| [point.x, point.y];
	match *segment {
		CurveSegment::Line { from, to } => VectorSegment::Line(point(from), point(to)),
		CurveSegment::Quadratic { from, control, to } => VectorSegment::Quadratic(point(from), point(control), point(to)),
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		} => VectorSegment::Cubic(point(from), point(control0), point(control1), point(to)),
	}
}

/// The `VectorLayer` struct is one filled shape of a [`Vector`], such as the body or the glass front of a glass icon.
///
/// Build its outline with [`super::CurvePath`] and take its [`super::CurvePath::segments`], or list
/// [`CurveSegment`]s directly.
#[derive(Clone, PartialEq)]
pub struct VectorLayer {
	/// The outline, as closed contours of joined segments in view box units.
	pub segments: Vec<CurveSegment>,
	/// Which parts of crossing or nested contours are filled; see [Path](/docs/reference/gui#path).
	pub fill_rule: FillRule,
	/// The layer's own paint, such as a gradient whose ends are in view box units. `None` fills it with the color the
	/// drawing gives, which can change with the drawing's state, such as a button under the pointer.
	pub paint: Option<Color>,
	/// The standard deviation, in view box units, of the blur this layer lays over what's drawn behind it, as a
	/// frosted glass front does; 0 for none.
	pub backdrop_blur: f32,
}

impl VectorLayer {
	/// A layer in the drawing's color, without a blur.
	pub fn plain(segments: Vec<CurveSegment>, fill_rule: FillRule) -> Self {
		Self {
			segments,
			fill_rule,
			paint: None,
			backdrop_blur: 0.0,
		}
	}
}

impl From<resource_management::resources::vector::Vector> for Vector {
	fn from(vector: resource_management::resources::vector::Vector) -> Self {
		use resource_management::resources::vector::VectorPaint;

		let point = |[x, y]: [f32; 2]| CurvePoint::new(x, y);
		let color = |[r, g, b, a]: [f32; 4]| RGBA::new(r, g, b, a);
		let layers = vector
			.layers
			.into_iter()
			.map(|layer| VectorLayer {
				segments: layer
					.segments
					.into_iter()
					.map(|segment| match segment {
						VectorSegment::Line(from, to) => CurveSegment::Line {
							from: point(from),
							to: point(to),
						},
						VectorSegment::Quadratic(from, control, to) => CurveSegment::Quadratic {
							from: point(from),
							control: point(control),
							to: point(to),
						},
						VectorSegment::Cubic(from, control0, control1, to) => CurveSegment::Cubic {
							from: point(from),
							control0: point(control0),
							control1: point(control1),
							to: point(to),
						},
					})
					.collect(),
				fill_rule: if layer.even_odd {
					FillRule::EvenOdd
				} else {
					FillRule::NonZero
				},
				paint: Some(match layer.paint {
					VectorPaint::Color(value) => color(value).into(),
					VectorPaint::LinearGradient { from, to, colors } => {
						LinearGradient::new(from, to, color(colors[0]), color(colors[1])).into()
					}
				}),
				backdrop_blur: layer.backdrop_blur,
			})
			.collect();
		// The bake measured where the picture looks centered, so loading doesn't measure it again.
		Self {
			view_box: vector.view_box,
			layers,
			optical_shift: vector.optical_shift,
		}
	}
}

/// Loads the vector image baked from the SVG asset `id`, such as `"icons/cube.svg"`, to draw with [`draw_vector`].
///
/// See [SVG vector images](/docs/reference/svg) for what a bake keeps from the file.
pub async fn load_vector(resource_manager: &resource_management::ResourceManager, id: &str) -> Result<Vector, String> {
	let reference: resource_management::Reference<resource_management::resources::vector::Vector> =
		resource_manager.request(id).await.map_err(|error| {
			format!(
				"Vector image '{id}' could not be loaded. The most likely cause is a missing `.svg` asset, or one that failed to bake. Check the asset ID and the bake's errors: {error}"
			)
		})?;
	Ok(reference.resource().clone().into())
}

/// The `VectorView` struct keeps a vector image a component drew, so the component can recolor it as its state
/// changes, such as a button under the pointer. Draw one with [`draw_vector`].
pub struct VectorView<C: 'static> {
	/// The box the layers stack in, which fades the whole picture.
	frame: EvaluationContext<C>,
	/// The layers without a paint of their own, which take the drawing's color.
	plain: Vec<EvaluationContext<C>>,
}

impl<C: 'static> VectorView<C> {
	/// Gives the picture `color`: its plain layers take the color, and the whole picture its alpha, so a painted
	/// picture fades the way a plain one does.
	pub async fn tint(&mut self, color: RGBA) {
		let alpha = color.a;
		self.frame.update_container(move |c| c.opacity(alpha)).await;
		for layer in &mut self.plain {
			layer
				.update_path(move |path| path.style([ConcreteLayer::default().color(RGBA { a: 1.0, ..color }.into())]))
				.await;
		}
	}
}

/// Draws `vector` under `parent` as the element `key`, `width` units wide and as tall as its view box's shape, with
/// its plain layers filled with `fill`. Centering flows line it up by where it looks centered. Returns it to recolor
/// with [`VectorView::tint`].
pub async fn draw_vector<C: 'static, const N: usize>(
	parent: &mut EvaluationContext<C>,
	key: &'static str,
	vector: &Vector,
	width: f32,
	fill: [ConcreteLayer; N],
) -> VectorView<C> {
	let height = width * vector.view_box[1] / vector.view_box[0].max(f32::EPSILON);
	// The layers stack at one origin, so each lines up with the others however its own shape would center.
	// Centering flows line the box up by where the picture looks centered: the middle, moved by the optical shift.
	let center = vector.optical_shift.map(|[x, y]| {
		[
			0.5 + x / vector.view_box[0].max(f32::EPSILON),
			0.5 + y / vector.view_box[1].max(f32::EPSILON),
		]
	});
	let mut frame = parent
		.element(key)
		.container(|c| {
			c.width(width.into())
				.height(height.into())
				.clip(false)
				.flow(stacked)
				.visual_center(center)
		})
		.await;
	// Blurs are stated in view box units, and the engine takes layout units.
	let units = width / vector.view_box[0].max(f32::EPSILON);
	let mut plain = Vec::new();
	for (index, layer) in vector.layers.iter().enumerate() {
		// A blur layer only where there's a blur: without one, the layer would paint its default white under the
		// paint, and that white would show around the paint's anti-aliased edges.
		let blur =
			(layer.backdrop_blur > 0.0).then(|| ConcreteLayer::default().backdrop_blur_sigma(layer.backdrop_blur * units));
		let painted = layer
			.paint
			.clone()
			.map(|paint| blur.into_iter().chain([ConcreteLayer::default().color(paint)]));
		let element = frame
			.element(("layer", index))
			.path(|path| {
				let path = path
					.width(width.into())
					.height(height.into())
					.segments(layer.segments.iter().cloned())
					.view_box(vector.view_box)
					.fill_rule(layer.fill_rule);
				match painted {
					Some(layers) => path.style(layers),
					None => path.style(fill.clone()),
				}
			})
			.await;
		if layer.paint.is_none() {
			plain.push(element);
		}
	}
	VectorView { frame, plain }
}

/// Places every child at the parent's origin, one over the next, as a vector image's layers draw.
fn stacked(input: FlowInput) -> FlowOutput {
	FlowOutput::new(input.cursor(), input.cursor())
}
