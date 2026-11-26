//! Bakes SVG files into [`Vector`] resources: the filled shapes the UI can draw, with everything else either
//! converted or refused. [SVG vector images](/docs/reference/svg) is the specification this module implements.

use i_overlay::core::{fill_rule::FillRule as OverlayFillRule, overlay_rule::OverlayRule};
use i_overlay::float::single::SingleFloatOverlay;
use tiny_skia_path::{Path as SkiaPath, PathSegment, Point, StrokeDash, Transform};
use usvg::{Group, Node, Paint};

use super::{
	ResourceId,
	handler::{AssetHandler, BakeContext, LoadErrors},
};
use crate::{
	ProcessedAsset, online_docs_url,
	resources::vector::{FLATTENING_TOLERANCE, Ink, Vector, VectorLayer, VectorPaint, VectorSegment, ink, optical_shift},
};

/// The `SvgAssetHandler` struct lets an asset manager bake `.svg` files into [`Vector`] resources, so an application
/// ships its icons and other vector images in the resource store with the rest of its assets.
///
/// Register it with [`crate::asset::manager::AssetManager::add_asset_handler`]; the standard graphics setup does.
#[derive(Default)]
pub struct SvgAssetHandler;

impl AssetHandler for SvgAssetHandler {
	fn can_handle(&self, r#type: &str) -> bool {
		matches!(r#type, "svg" | "image/svg+xml")
	}

	async fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
		let (data, _) = context.resolve(url).await?;

		match vector(&data) {
			Ok(vector) => context.store_primary(ProcessedAsset::new(url, vector), &[]).await,
			Err(problem) => {
				context.error(format_args!("Vector image '{}' could not be baked. {problem}", url.as_ref()));
				Err(LoadErrors::FailedToProcess)
			}
		}
	}
}

/// The reference page that lists what a vector image keeps, which every refusal links to at its section.
const SVG_DOCS_PATH: &str = "reference/svg";

/// Explains a part of the file the UI can't draw: what it is, and what to use instead.
fn unsupported(what: &str, instead: &str, section: &str) -> String {
	format!(
		"It uses {what}, which vector images don't draw. {instead} See {}.",
		online_docs_url(&format!("{SVG_DOCS_PATH}#{section}"))
	)
}

/// Reads an SVG file into the layers the UI draws, or explains the first part it can't draw.
pub(crate) fn vector(data: &[u8]) -> Result<Vector, String> {
	refuse_undrawn_elements(data)?;
	let tree = usvg::Tree::from_data(data, &usvg::Options::default()).map_err(|error| {
		format!("The file isn't valid SVG ({error}). The most likely cause is a truncated or hand-edited file. Re-export it from your editor.")
	})?;
	let size = tree.size();
	let mut baker = Baker {
		layers: Vec::new(),
		glass: Vec::new(),
		// Curves cut by a clip or mask become lines this close to the curve, in box units.
		tolerance: size.width().max(size.height()) * FLATTENING_TOLERANCE,
	};
	baker.group(tree.root(), Transform::identity(), 1.0, &[])?;
	if !baker.glass.is_empty() {
		return Err(unsupported(
			"a blurred copy with no shape drawn after it in the shape of its clip",
			"Draw the glass shape after the blurred copy, or remove the filter.",
			"glass",
		));
	}
	let optical_shift = optical_shift(baker.layers.iter().map(|layer| (&layer.segments[..], layer.even_odd)));
	Ok(Vector {
		view_box: [size.width(), size.height()],
		layers: baker.layers,
		optical_shift,
	})
}

/// Refuses text, images, and foreign objects before parsing, since the parser drops some of them silently.
fn refuse_undrawn_elements(data: &[u8]) -> Result<(), String> {
	// A file the XML parser can't read is left for the SVG parser to report.
	let Ok(text) = std::str::from_utf8(data) else {
		return Ok(());
	};
	let Ok(document) = usvg::roxmltree::Document::parse(text) else {
		return Ok(());
	};
	for name in ["text", "image", "foreignObject"] {
		if document.descendants().any(|node| node.tag_name().name() == name) {
			return Err(unsupported(
				&format!("a <{name}> element"),
				"Convert it to paths in your editor, such as with Object to Path.",
				"not-drawn",
			));
		}
	}
	Ok(())
}

/// One region that cuts every layer inside a clipped or masked group, as filled contours by the non-zero rule.
type Region = Ink;

/// A blurred copy waiting for the glass drawn after it: the clip's outline, and the blur in box units.
struct Glass {
	outline: Vec<VectorSegment>,
	sigma: f32,
}

/// The `Baker` struct collects layers while it walks the tree, in the order the file draws them.
struct Baker {
	layers: Vec<VectorLayer>,
	glass: Vec<Glass>,
	tolerance: f32,
}

impl Baker {
	/// Walks a group's children with `transform` from their coordinates to the box, the group's accumulated
	/// `opacity`, and the `regions` every layer inside is cut to.
	fn group(&mut self, group: &Group, transform: Transform, opacity: f32, regions: &[Region]) -> Result<(), String> {
		for node in group.children() {
			match node {
				Node::Group(child) => self.child_group(child, transform, opacity, regions)?,
				Node::Path(path) => self.path(path, transform, opacity, regions)?,
				Node::Image(_) => {
					return Err(unsupported(
						"an image",
						"Trace it into paths, or draw it as a UI image instead.",
						"not-drawn",
					));
				}
				Node::Text(_) => {
					return Err(unsupported(
						"text",
						"Convert it to paths in your editor, such as with Object to Path.",
						"not-drawn",
					));
				}
			}
		}
		Ok(())
	}

	/// Applies a group's transform, opacity, clip, mask, and filter, then walks its children.
	fn child_group(&mut self, group: &Group, transform: Transform, opacity: f32, regions: &[Region]) -> Result<(), String> {
		let transform = transform.pre_concat(group.transform());
		let opacity = opacity * group.opacity().get();
		if group.blend_mode() != usvg::BlendMode::Normal {
			return Err(unsupported(
				"a blend mode",
				"Use the normal blend mode, and flatten the effect into colors.",
				"groups",
			));
		}
		if !group.filters().is_empty() {
			return self.blurred_copy(group, transform);
		}
		let mut regions = regions.to_vec();
		if let Some(clip) = group.clip_path() {
			regions.push(self.clip_region(clip, transform)?);
		}
		if let Some(mask) = group.mask() {
			regions.push(self.mask_region(mask, transform)?);
		}
		self.group(group, transform, opacity, &regions)
	}

	/// Takes a group with a filter as glass's blurred copy, which the glass drawn after it replaces with a blur of
	/// what's behind it; any other filter is refused.
	fn blurred_copy(&mut self, group: &Group, transform: Transform) -> Result<(), String> {
		let refuse = || {
			unsupported(
				"a filter",
				"Remove it, or flatten the effect into the shapes and colors.",
				"groups",
			)
		};
		let [filter] = group.filters() else {
			return Err(refuse());
		};
		let [primitive] = filter.primitives() else {
			return Err(refuse());
		};
		let usvg::filter::Kind::GaussianBlur(blur) = primitive.kind() else {
			return Err(refuse());
		};
		let sigma = blur.std_dev_x().get();
		if sigma != blur.std_dev_y().get() || !matches!(blur.input(), usvg::filter::Input::SourceGraphic) {
			return Err(refuse());
		}
		let Some(clip) = group.clip_path() else {
			return Err(refuse());
		};
		let clip_transform = transform.pre_concat(clip.transform());
		let [Node::Path(shape)] = clip.root().children() else {
			return Err(refuse());
		};
		let outline = segments(shape.data(), clip_transform).ok_or_else(refuse)?;
		// The blur is in the group's units, which the transform scales into the box's.
		let (scale_x, scale_y) = transform.get_scale();
		self.glass.push(Glass {
			outline,
			sigma: sigma * (scale_x * scale_y).sqrt(),
		});
		Ok(())
	}

	/// The region a clip path keeps: the union of its shapes, each by its own clip rule.
	fn clip_region(&self, clip: &usvg::ClipPath, transform: Transform) -> Result<Region, String> {
		if clip.clip_path().is_some() {
			return Err(unsupported(
				"a clip path that is clipped itself",
				"Combine the two clips into one shape in your editor.",
				"groups",
			));
		}
		let transform = transform.pre_concat(clip.transform());
		let mut region = Region::new();
		for node in clip.root().children() {
			let Node::Path(path) = node else {
				return Err(unsupported(
					"a clip path with groups or text inside",
					"Make the clip a plain shape.",
					"groups",
				));
			};
			let even_odd = path.fill().is_some_and(|fill| fill.rule() == usvg::FillRule::EvenOdd);
			let shape = self.flattened(path.data(), transform, even_odd);
			region = region.overlay(&shape, OverlayRule::Union, OverlayFillRule::NonZero).concat();
		}
		Ok(region)
	}

	/// The region a mask keeps: its box, less everything but its white shapes, which later black shapes cut back.
	fn mask_region(&self, mask: &usvg::Mask, transform: Transform) -> Result<Region, String> {
		let refuse = || {
			unsupported(
				"a mask with grays, colors, gradients, or partial opacity",
				"Fill the mask's shapes with plain black or white, or use a clip path.",
				"groups",
			)
		};
		if mask.mask().is_some() {
			return Err(refuse());
		}
		let mut region = Region::new();
		for node in mask.root().children() {
			let Node::Path(path) = node else {
				return Err(refuse());
			};
			if path.stroke().is_some() {
				return Err(refuse());
			}
			let Some(fill) = path.fill() else {
				continue;
			};
			let Paint::Color(color) = fill.paint() else {
				return Err(refuse());
			};
			if fill.opacity().get() < 1.0 {
				return Err(refuse());
			}
			let white = match (mask.kind(), (color.red, color.green, color.blue)) {
				(usvg::MaskType::Alpha, _) | (_, (255, 255, 255)) => true,
				(_, (0, 0, 0)) => false,
				_ => return Err(refuse()),
			};
			let shape = self.flattened(path.data(), transform, fill.rule() == usvg::FillRule::EvenOdd);
			let rule = if white { OverlayRule::Union } else { OverlayRule::Difference };
			region = region.overlay(&shape, rule, OverlayFillRule::NonZero).concat();
		}
		// Nothing outside the mask's box shows.
		let rect = mask.rect();
		let corners = [
			(rect.left(), rect.top()),
			(rect.right(), rect.top()),
			(rect.right(), rect.bottom()),
			(rect.left(), rect.bottom()),
		];
		let bounds: Region = vec![
			corners
				.iter()
				.map(|&(x, y)| {
					let mut point = Point::from_xy(x, y);
					transform.map_point(&mut point);
					[point.x as f64, point.y as f64]
				})
				.collect(),
		];
		Ok(region
			.overlay(&bounds, OverlayRule::Intersect, OverlayFillRule::NonZero)
			.concat())
	}

	/// Adds a path's fill and stroke layers, in the order it paints them.
	fn path(&mut self, path: &usvg::Path, transform: Transform, opacity: f32, regions: &[Region]) -> Result<(), String> {
		if !path.is_visible() {
			return Ok(());
		}
		// Each shape in the path's own units: its outline, whether it fills by the even-odd rule, its paint, and its
		// opacity.
		let mut shapes: Vec<(SkiaPath, bool, &Paint, f32)> = Vec::with_capacity(2);
		if let Some(fill) = path.fill() {
			let even_odd = fill.rule() == usvg::FillRule::EvenOdd;
			shapes.push((path.data().clone(), even_odd, fill.paint(), fill.opacity().get()));
		}
		if let Some(stroke) = path.stroke()
			&& let Some(outline) = stroked(path.data(), stroke, transform)
		{
			shapes.push((outline, false, stroke.paint(), stroke.opacity().get()));
		}
		if path.paint_order() == usvg::PaintOrder::StrokeAndFill {
			shapes.reverse();
		}
		for (source, even_odd, paint, alpha) in shapes {
			let Some(outline) = segments(&source, transform) else {
				continue;
			};
			let paint = vector_paint(paint, transform, alpha * opacity)?;
			let backdrop_blur = self.take_glass(&outline);
			let (segments, even_odd) = if regions.is_empty() {
				(outline, even_odd)
			} else {
				(self.cut(&source, transform, even_odd, regions), false)
			};
			self.layers.push(VectorLayer {
				segments,
				even_odd,
				paint,
				backdrop_blur,
			});
		}
		Ok(())
	}

	/// The blur of the blurred copy whose clip has `outline`'s shape, which that copy hands to this layer; 0 when no
	/// copy waits for it.
	fn take_glass(&mut self, outline: &[VectorSegment]) -> f32 {
		let Some(index) = self.glass.iter().position(|glass| same_outline(&glass.outline, outline)) else {
			return 0.0;
		};
		self.glass.remove(index).sigma
	}

	/// Cuts a shape to every region, as lines by the non-zero rule.
	fn cut(&self, source: &SkiaPath, transform: Transform, even_odd: bool, regions: &[Region]) -> Vec<VectorSegment> {
		let mut shape = self.flattened(source, transform, even_odd);
		for region in regions {
			shape = shape
				.overlay(region, OverlayRule::Intersect, OverlayFillRule::NonZero)
				.concat();
		}
		let mut segments = Vec::new();
		for contour in &shape {
			for (index, point) in contour.iter().enumerate() {
				let next = contour[(index + 1) % contour.len()];
				segments.push(VectorSegment::Line(
					[point[0] as f32, point[1] as f32],
					[next[0] as f32, next[1] as f32],
				));
			}
		}
		segments
	}

	/// A shape as polygons in box units, resolved by its fill rule so the non-zero rule fills the same area.
	fn flattened(&self, source: &SkiaPath, transform: Transform, even_odd: bool) -> Region {
		ink(&segments(source, transform).unwrap_or_default(), even_odd, self.tolerance)
	}
}

/// A stroke's outline in the path's own units, dashed first when it has dashes; `None` for a stroke that covers
/// nothing.
fn stroked(source: &SkiaPath, stroke: &usvg::Stroke, transform: Transform) -> Option<SkiaPath> {
	let dashed = match stroke.dasharray() {
		Some(dashes) => source.dash(&StrokeDash::new(dashes.to_vec(), stroke.dashoffset())?, 1.0)?,
		None => source.clone(),
	};
	// The stroker splits curves finely enough for the size the transform draws them at.
	let (scale_x, scale_y) = transform.get_scale();
	dashed.stroke(&stroke.to_tiny_skia(), scale_x.max(scale_y).max(1.0))
}

/// A path's segments in box units, with every contour closed; `None` for a path the transform can't map.
fn segments(source: &SkiaPath, transform: Transform) -> Option<Vec<VectorSegment>> {
	let path = source.clone().transform(transform)?;
	let point = |point: Point| [point.x, point.y];
	let mut segments = Vec::new();
	let (mut start, mut last) = ([0.0; 2], [0.0; 2]);
	let close = |segments: &mut Vec<VectorSegment>, last: [f32; 2], start: [f32; 2]| {
		if last != start {
			segments.push(VectorSegment::Line(last, start));
		}
	};
	for segment in path.segments() {
		match segment {
			PathSegment::MoveTo(to) => {
				close(&mut segments, last, start);
				start = point(to);
				last = start;
			}
			PathSegment::LineTo(to) => {
				segments.push(VectorSegment::Line(last, point(to)));
				last = point(to);
			}
			PathSegment::QuadTo(control, to) => {
				segments.push(VectorSegment::Quadratic(last, point(control), point(to)));
				last = point(to);
			}
			PathSegment::CubicTo(c0, c1, to) => {
				segments.push(VectorSegment::Cubic(last, point(c0), point(c1), point(to)));
				last = point(to);
			}
			PathSegment::Close => {
				close(&mut segments, last, start);
				last = start;
			}
		}
	}
	close(&mut segments, last, start);
	Some(segments)
}

/// Whether two outlines are the same shape: the same segments with every point within 0.001 box units.
fn same_outline(a: &[VectorSegment], b: &[VectorSegment]) -> bool {
	let points = |segment: &VectorSegment| -> Vec<[f32; 2]> {
		match *segment {
			VectorSegment::Line(a, b) => vec![a, b],
			VectorSegment::Quadratic(a, b, c) => vec![a, b, c],
			VectorSegment::Cubic(a, b, c, d) => vec![a, b, c, d],
		}
	};
	a.len() == b.len()
		&& a.iter().zip(b).all(|(a, b)| {
			let (a, b) = (points(a), points(b));
			a.len() == b.len()
				&& a.iter()
					.zip(&b)
					.all(|(a, b)| (a[0] - b[0]).abs() <= 0.001 && (a[1] - b[1]).abs() <= 0.001)
		})
}

/// A paint in box units, with `alpha` multiplied into its colors.
fn vector_paint(paint: &Paint, transform: Transform, alpha: f32) -> Result<VectorPaint, String> {
	let color = |color: usvg::Color, opacity: f32| {
		[
			color.red as f32 / 255.0,
			color.green as f32 / 255.0,
			color.blue as f32 / 255.0,
			opacity * alpha,
		]
	};
	match paint {
		Paint::Color(value) => Ok(VectorPaint::Color(color(*value, 1.0))),
		Paint::LinearGradient(gradient) => {
			let stops = gradient.stops();
			let (first, last) = match stops {
				[only] => return Ok(VectorPaint::Color(color(only.color(), only.opacity().get()))),
				[first, last] => (first, last),
				_ => {
					return Err(unsupported(
						"a gradient with more than two stops",
						"Use two stops, or split the shape into one shape per pair of stops.",
						"paint",
					));
				}
			};
			let transform = transform.pre_concat(gradient.transform());
			let map = |x: f32, y: f32| {
				let mut point = Point::from_xy(x, y);
				transform.map_point(&mut point);
				[point.x, point.y]
			};
			let (start, end) = (map(gradient.x1(), gradient.y1()), map(gradient.x2(), gradient.y2()));
			// Stop offsets move the gradient's ends along its line.
			let along = |offset: f32| {
				[
					start[0] + (end[0] - start[0]) * offset,
					start[1] + (end[1] - start[1]) * offset,
				]
			};
			Ok(VectorPaint::LinearGradient {
				from: along(first.offset().get()),
				to: along(last.offset().get()),
				colors: [
					color(first.color(), first.opacity().get()),
					color(last.color(), last.opacity().get()),
				],
			})
		}
		Paint::RadialGradient(_) => Err(unsupported(
			"a radial gradient",
			"Use a linear gradient or a color instead.",
			"paint",
		)),
		Paint::Pattern(_) => Err(unsupported("a pattern", "Use a color or a linear gradient instead.", "paint")),
	}
}

#[cfg(test)]
mod tests {
	use super::{SvgAssetHandler, vector};
	use crate::{
		asset::{self, ResourceId, manager::AssetManager},
		r#async, resource,
		resources::vector::{VectorPaint, VectorSegment},
	};

	/// Wraps shapes in an SVG with a 20 by 10 view box drawn at twice that size.
	fn svg(body: &str) -> Vec<u8> {
		format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20" viewBox="0 0 20 10">{body}</svg>"#)
			.into_bytes()
	}

	/// The smallest and largest x and y of a layer's points.
	fn bounds(segments: &[VectorSegment]) -> [f32; 4] {
		let mut bounds = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
		for segment in segments {
			let points: &[[f32; 2]] = match segment {
				VectorSegment::Line(a, b) => &[*a, *b],
				VectorSegment::Quadratic(a, _, c) => &[*a, *c],
				VectorSegment::Cubic(a, _, _, d) => &[*a, *d],
			};
			for point in points {
				bounds = [
					bounds[0].min(point[0]),
					bounds[1].min(point[1]),
					bounds[2].max(point[0]),
					bounds[3].max(point[1]),
				];
			}
		}
		bounds
	}

	fn assert_near(actual: [f32; 4], expected: [f32; 4]) {
		assert!(
			actual.iter().zip(expected).all(|(a, e)| (a - e).abs() < 0.01),
			"{actual:?} isn't {expected:?}"
		);
	}

	#[test]
	fn a_filled_shape_becomes_one_layer_in_the_box_its_size_gives() {
		let vector = vector(&svg(
			r##"<rect x="2" y="1" width="6" height="4" fill="#ff8000" fill-opacity="0.5"/>"##,
		))
		.unwrap();
		assert_eq!(vector.view_box, [40.0, 20.0]);
		let [layer] = vector.layers.as_slice() else {
			panic!("one shape draws one layer");
		};
		// The view box scales the shape into the box.
		assert_near(bounds(&layer.segments), [4.0, 2.0, 16.0, 10.0]);
		assert_eq!(layer.paint, VectorPaint::Color([1.0, 128.0 / 255.0, 0.0, 0.5]));
		assert_eq!(layer.backdrop_blur, 0.0);
		// A rectangle looks centered at its middle.
		let [x, y] = vector.optical_shift.expect("a filled shape has an optical center");
		assert!(x.abs() < 1e-3 && y.abs() < 1e-3);
	}

	#[test]
	fn a_bake_stores_where_its_shapes_look_centered() {
		// A right triangle's bounds are centered at (12, 12) in the box and its centroid at (8, 8), so it looks centered
		// halfway between.
		let vector = vector(&svg(r##"<path d="M0 0H12L0 12Z"/>"##)).unwrap();
		let [x, y] = vector.optical_shift.unwrap();
		assert!((x + 2.0).abs() < 1e-3 && (y + 2.0).abs() < 1e-3, "{x} {y}");
	}

	#[test]
	fn a_stroke_draws_its_outline_over_the_fill() {
		let vector = vector(&svg(
			r##"<rect x="2" y="2" width="6" height="6" fill="white" stroke="black" stroke-width="2"/>"##,
		))
		.unwrap();
		let [fill, stroke] = vector.layers.as_slice() else {
			panic!("a filled, stroked shape draws two layers");
		};
		assert_eq!(fill.paint, VectorPaint::Color([1.0, 1.0, 1.0, 1.0]));
		assert_eq!(stroke.paint, VectorPaint::Color([0.0, 0.0, 0.0, 1.0]));
		// The stroke reaches half its width past the shape, in box units.
		assert_near(bounds(&stroke.segments), [2.0, 2.0, 18.0, 18.0]);
	}

	#[test]
	fn a_two_stop_gradient_keeps_its_ends_and_colors() {
		let vector = vector(&svg(
			r##"<linearGradient id="g" x1="0" y1="0" x2="10" y2="0" gradientUnits="userSpaceOnUse">
				<stop offset="0" stop-color="#000"/><stop offset="1" stop-color="#fff" stop-opacity="0.5"/>
			</linearGradient><rect width="10" height="10" fill="url(#g)"/>"##,
		))
		.unwrap();
		assert_eq!(
			vector.layers[0].paint,
			VectorPaint::LinearGradient {
				from: [0.0, 0.0],
				to: [20.0, 0.0],
				colors: [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 0.5]],
			}
		);
	}

	#[test]
	fn what_the_ui_cant_draw_fails_with_its_section() {
		for (body, section) in [
			(
				r##"<radialGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></radialGradient><rect width="5" height="5" fill="url(#g)"/>"##,
				"#paint",
			),
			(r##"<text x="1" y="5">Hi</text>"##, "#not-drawn"),
			(
				r##"<filter id="f"><feOffset dx="1"/></filter><rect width="5" height="5" filter="url(#f)"/>"##,
				"#groups",
			),
		] {
			let error = vector(&svg(body)).unwrap_err();
			assert!(error.contains(section), "{error}");
		}
	}

	#[test]
	fn a_black_and_white_mask_cuts_the_layers_it_masks() {
		let vector = vector(&svg(
			r##"<mask id="m" maskUnits="userSpaceOnUse" x="0" y="0" width="20" height="10">
				<rect width="20" height="10" fill="white"/><rect width="5" height="10" fill="black"/>
			</mask><rect width="10" height="10" fill="red" mask="url(#m)"/>"##,
		))
		.unwrap();
		// The black rectangle takes the left half of the shape away.
		assert_near(bounds(&vector.layers[0].segments), [10.0, 0.0, 20.0, 20.0]);
	}

	#[test]
	fn a_blurred_copy_turns_into_the_glass_drawn_after_it_blurring_its_backdrop() {
		let glass = r##"M6 2H14V8H6Z"##;
		let vector = vector(&svg(&format!(
			r##"<filter id="f"><feGaussianBlur stdDeviation="1.5"/></filter>
			<clipPath id="c"><path d="{glass}"/></clipPath>
			<rect width="10" height="10" fill="black"/>
			<rect width="10" height="10" fill="black" filter="url(#f)" clip-path="url(#c)"/>
			<path d="{glass}" fill="white" fill-opacity="0.6"/>"##
		)))
		.unwrap();
		let [body, front] = vector.layers.as_slice() else {
			panic!("the blurred copy draws no layer of its own");
		};
		assert_eq!(body.backdrop_blur, 0.0);
		// The box is twice the view box, so the blur doubles too.
		assert!((front.backdrop_blur - 3.0).abs() < 1e-4, "{}", front.backdrop_blur);
	}

	#[test]
	fn a_blurred_copy_without_its_glass_fails() {
		let error = vector(&svg(r##"<filter id="f"><feGaussianBlur stdDeviation="1"/></filter>
			<clipPath id="c"><rect width="4" height="4"/></clipPath>
			<rect width="10" height="10" filter="url(#f)" clip-path="url(#c)"/>"##))
		.unwrap_err();
		assert!(error.contains("#glass"), "{error}");
	}

	#[r#async::test]
	async fn svg_files_bake_into_vector_resources() {
		let asset_storage_backend = asset::storage_backend::tests::TestStorageBackend::new();
		let resource_storage_backend = resource::storage_backend::MemoryStorageBackend::new();
		asset_storage_backend.add_file("icons/square.svg", &svg(r##"<rect width="5" height="5"/>"##));
		asset_storage_backend.add_file("icons/broken.svg", b"<svg");

		let mut asset_manager = AssetManager::new(asset_storage_backend, resource_storage_backend);
		asset_manager.add_asset_handler(SvgAssetHandler);

		asset_manager.bake("icons/square.svg").await.expect("a valid SVG should bake");
		let resource = asset_manager
			.memory_storage()
			.get_resource(ResourceId::new("icons/square.svg"))
			.expect("the baked vector image should be stored");
		assert_eq!(resource.class, "Vector");
		assert!(
			asset_manager.bake("icons/broken.svg").await.is_err(),
			"a file that isn't SVG must fail the bake"
		);
	}
}

use crate::asset::handler::PrimaryStored;
