//! UI filled paths drawn from outlines on the GPU with the Slug algorithm.
//!
//! Paths share the glyph packer, [`super::slug::SlugCurves`], but are keyed by content instead of
//! glyph index, packed in their own units with y pointing up, and read from their own two storage
//! buffers. Text and paths stay apart until both are merged.

use super::*;
use crate::ui::{
	components::{
		curve::{CurvePoint, CurveSegment},
		path::FillRule,
	},
	font::{CUBIC_TOLERANCE, QuadraticCollector, curve_bounds},
};

pub(super) type PathKey = (u64, u64, FillRule);

/// Packed paths by content key. Build their quads with [`build_ui_path_geometry_damaged`].
pub(super) type UiPathCurves = SlugCurves<utils::hash::HashMap<PathKey, PackedOutline>>;

/// One path's outline as quadratic curves in path units, y pointing up.
pub(super) struct PathOutline {
	pub(super) curves: Vec<[[f32; 2]; 3]>,
	pub(super) bounds: [f32; 4],
}

impl UiPathCurves {
	/// Returns where the shader finds the path, packing its segments on first use.
	///
	/// Returns `None` when the path does not fit in the remaining buffer space.
	pub(super) fn ensure(&mut self, key: PathKey, segments: &[CurveSegment]) -> Option<PackedOutline> {
		if let Some(packed) = self.packed.get(&key) {
			return Some(*packed);
		}
		let outline = path_outline(segments, key.2);
		// Path units are arbitrary, so the band overlap follows the path's own extent.
		let [x0, y0, x1, y1] = outline.bounds;
		let packed = self.pack(&outline.curves, outline.bounds, (x1 - x0).max(y1 - y0) * BAND_OVERLAP)?;
		self.packed.insert(key, packed);
		Some(packed)
	}
}

/// Turns segments into quadratic contours, closes them, applies the fill rule, and flips y up.
pub(super) fn path_outline(segments: &[CurveSegment], fill_rule: FillRule) -> PathOutline {
	// The cubic split tolerance follows the path's own extent, whatever its units are.
	let mut extent = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
	for segment in segments {
		for point in segment_points(segment) {
			if point.is_finite() {
				extent = [
					extent[0].min(point.x),
					extent[1].min(point.y),
					extent[2].max(point.x),
					extent[3].max(point.y),
				];
			}
		}
	}
	let span = ((extent[2] - extent[0]).max(extent[3] - extent[1])).max(0.0);
	let mut collector = QuadraticCollector::new(1.0, (span * CUBIC_TOLERANCE).max(1e-5));
	// Only the even-odd fill reorients contours, so only it records where each contour ends. The nonzero fill
	// never allocates this list.
	let even_odd = fill_rule == FillRule::EvenOdd;
	let mut contour_ends = Vec::new();
	let mut close_contour = |collector: &mut QuadraticCollector| {
		collector.close_contour();
		let end = collector.curve_count();
		if even_odd && end > contour_ends.last().copied().unwrap_or(0) {
			contour_ends.push(end);
		}
	};

	for segment in segments {
		let points = segment_points(segment);
		if !points.iter().all(|point| point.is_finite()) {
			continue;
		}
		let from = points[0];
		if collector.last().is_none_or(|last| last != [from.x, from.y]) {
			close_contour(&mut collector);
			collector.start_contour([from.x, from.y]);
		}
		match *segment {
			CurveSegment::Line { to, .. } => collector.line([to.x, to.y]),
			CurveSegment::Quadratic { control, to, .. } => collector.quad([control.x, control.y], [to.x, to.y]),
			CurveSegment::Cubic {
				control0, control1, to, ..
			} => collector.cubic([control0.x, control0.y], [control1.x, control1.y], [to.x, to.y]),
		}
	}
	close_contour(&mut collector);
	let mut curves = collector.finish();
	if even_odd {
		orient_even_odd(&mut curves, &contour_ends);
	}

	// The shader shares the glyph convention of y pointing up.
	for point in curves.iter_mut().flatten() {
		point[1] = -point[1];
	}
	let bounds = curve_bounds(&curves);
	PathOutline { curves, bounds }
}

fn segment_points(segment: &CurveSegment) -> [CurvePoint; 4] {
	match *segment {
		CurveSegment::Line { from, to } => [from, to, to, to],
		CurveSegment::Quadratic { from, control, to } => [from, control, to, to],
		CurveSegment::Cubic {
			from,
			control0,
			control1,
			to,
		} => [from, control0, control1, to],
	}
}

/// Orients contours so that nonzero winding reproduces the even-odd fill: contours at an even
/// nesting depth run one way and contours inside them run the other. Exact when no contour
/// crosses itself or another one.
fn orient_even_odd(curves: &mut [[[f32; 2]; 3]], contour_ends: &[usize]) {
	let contours = || {
		contour_ends
			.iter()
			.scan(0, |start, &end| Some(std::mem::replace(start, end)..end))
	};
	let polygons: Vec<Vec<[f32; 2]>> = contours().map(|contour| sample_polygon(&curves[contour])).collect();
	let areas: Vec<f32> = polygons.iter().map(|polygon| signed_area(polygon)).collect();
	for (index, contour) in contours().enumerate() {
		let Some(&probe) = polygons[index].first() else { continue };
		let depth = polygons
			.iter()
			.enumerate()
			.filter(|(other, polygon)| *other != index && contains(polygon, probe))
			.count();
		let outward = depth % 2 == 0;
		if (areas[index] > 0.0) != outward {
			let contour = &mut curves[contour];
			contour.reverse();
			for curve in contour.iter_mut() {
				curve.swap(0, 2);
			}
		}
	}
}

/// Each curve's start and midpoint; enough to tell orientation and containment.
fn sample_polygon(contour: &[[[f32; 2]; 3]]) -> Vec<[f32; 2]> {
	let mut polygon = Vec::with_capacity(contour.len() * 2);
	for [p0, p1, p2] in contour {
		polygon.push(*p0);
		polygon.push([0, 1].map(|axis| (p0[axis] + 2.0 * p1[axis] + p2[axis]) * 0.25));
	}
	polygon
}

fn signed_area(polygon: &[[f32; 2]]) -> f32 {
	let mut area = 0.0;
	for (index, a) in polygon.iter().enumerate() {
		let b = polygon[(index + 1) % polygon.len()];
		area += a[0] * b[1] - b[0] * a[1];
	}
	area * 0.5
}

fn contains(polygon: &[[f32; 2]], point: [f32; 2]) -> bool {
	let mut inside = false;
	for (index, a) in polygon.iter().enumerate() {
		let b = polygon[(index + 1) % polygon.len()];
		if (a[1] > point[1]) != (b[1] > point[1]) {
			let x = a[0] + (point[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);
			if point[0] < x {
				inside = !inside;
			}
		}
	}
	inside
}

/// The primitives of every path fill of one frame, by draw-list path index.
pub(super) struct UiPathGeometry<'a> {
	pub(super) primitives: Vec<UiPrimitive, &'a bumpalo::Bump>,
	pub(super) ranges: Vec<std::ops::Range<usize>, &'a bumpalo::Bump>,
	/// One prepared primitive per draw-list blur shaped by a path; `None` where the blur is not a path or was dropped.
	pub(super) blurs: Vec<Option<UiPrimitive>, &'a bumpalo::Bump>,
	pub(super) truncated: bool,
	pub(super) dropped_paths: usize,
}

/// Builds one primitive per path fill touching `damage`; `None` builds everything.
///
/// When the buffers fill up partway through a frame, they are reset and the frame is built
/// again, so no primitive keeps a location from before the reset.
pub(super) fn build_ui_path_geometry_damaged<'a>(
	draw_list: &UiDrawList,
	viewport: Extent,
	paths: &mut UiPathCurves,
	masks: &mut UiMaskTable,
	frame_allocator: &'a bumpalo::Bump,
	damage: Option<&[UiPixelRegion]>,
) -> UiPathGeometry<'a> {
	let width = viewport.width().max(1) as f32;
	let height = viewport.height().max(1) as f32;
	let sx = width / draw_list.layout_size[0].max(1.0);
	let sy = height / draw_list.layout_size[1].max(1.0);
	let viewport_clip = PixelClip {
		x0: 0.0,
		y0: 0.0,
		x1: width,
		y1: height,
	};

	let mut geometry = UiPathGeometry {
		primitives: Vec::with_capacity_in(draw_list.paths.len().min(MAX_UI_PRIMITIVES), frame_allocator),
		ranges: Vec::with_capacity_in(draw_list.paths.len(), frame_allocator),
		blurs: Vec::with_capacity_in(draw_list.blurs.len(), frame_allocator),
		truncated: false,
		dropped_paths: 0,
	};

	let radius_scale = sx.min(sy);
	let mut reset = false;
	loop {
		// Blurs are shaped by an outline the same way a fill is, but the merge places them itself.
		for blur in &draw_list.blurs {
			// A shadow's caster is the whole outline in white, moved by the offset. The composite
			// quad carries the clip, so the caster ignores it and a clipped part still casts.
			if let (Some(shape), Some(shadow)) = (&blur.path, blur.shadow) {
				let offset = [shadow.offset[0] * sx, shadow.offset[1] * sy];
				let caster = match place_outline(paths, shape, blur.position, None, sx, sy, viewport_clip) {
					Some(Ok(mut primitive)) => {
						primitive.bounds = [
							primitive.bounds[0] + offset[0],
							primitive.bounds[1] + offset[1],
							primitive.bounds[2] + offset[0],
							primitive.bounds[3] + offset[1],
						];
						primitive.a[0] += offset[0];
						primitive.a[1] += offset[1];
						primitive.kind = UI_KIND_PATH;
						primitive.color = [1.0; 4];
						primitive.color_end = [1.0; 4];
						// The caster turns with its element, so the composite reads it where it was turned to.
						primitive.mask = masks.index(None, blur.clip_mask, sx, sy);
						Some(primitive)
					}
					Some(Err(Dropped)) => {
						geometry.dropped_paths += 1;
						None
					}
					None => None,
				};
				geometry.blurs.push(caster);
				continue;
			}
			let placed = blur.path.as_ref().and_then(|shape| {
				(blur.radius > 0.0)
					.then(|| place_outline(paths, shape, blur.position, blur.clip, sx, sy, viewport_clip))
					.flatten()
			});
			let primitive = placed.map(|placed| match placed {
				Ok(mut primitive) => {
					let sigma = blur_sigma((blur.radius * radius_scale).clamp(0.0, 64.0));
					primitive.kind = UI_KIND_PATH_BLUR;
					primitive.color = [0.0, 0.0, 0.0, blur_resolution_mix(sigma)];
					primitive.mask = masks.index(None, blur.clip_mask, sx, sy);
					primitive
				}
				Err(Dropped) => {
					geometry.dropped_paths += 1;
					UiPrimitive::default()
				}
			});
			geometry
				.blurs
				.push(primitive.filter(|primitive| primitive.kind == UI_KIND_PATH_BLUR));
		}

		for path in &draw_list.paths {
			let start = geometry.primitives.len();
			geometry.ranges.push(start..start);
			if geometry.truncated || path.paint.alpha() <= 0.0 {
				continue;
			}
			let mut primitive = match place_outline(paths, &path.shape, path.position, path.clip, sx, sy, viewport_clip) {
				Some(Ok(primitive)) => primitive,
				Some(Err(Dropped)) => {
					geometry.dropped_paths += 1;
					continue;
				}
				None => continue,
			};
			// Every damaged blur is drawn, but a fill is only built where the frame is redrawn.
			let bounds = primitive.bounds;
			if !damage_intersects(
				damage,
				turned_bounds(
					[
						bounds[0] - UI_DAMAGE_MARGIN_PIXELS,
						bounds[1] - UI_DAMAGE_MARGIN_PIXELS,
						bounds[2] + UI_DAMAGE_MARGIN_PIXELS,
						bounds[3] + UI_DAMAGE_MARGIN_PIXELS,
					],
					path.clip_mask,
					sx,
					sy,
				),
			) {
				continue;
			}
			if geometry.primitives.len() == MAX_UI_PRIMITIVES {
				geometry.truncated = true;
				continue;
			}
			primitive.kind = UI_KIND_PATH;
			primitive.mask = masks.index(None, path.clip_mask, sx, sy);
			let [ox, oy, scale_x, scale_y] = primitive.a;
			// The axis is in path units with y down, so it maps like the quad and not like the packed curves.
			path.paint.placed([ox, oy], [scale_x, scale_y]).apply(&mut primitive);
			geometry.primitives.push(primitive);
			geometry.ranges.last_mut().unwrap().end = geometry.primitives.len();
		}

		if geometry.dropped_paths == 0 || reset {
			break;
		}
		paths.reset();
		reset = true;
		geometry.primitives.clear();
		geometry.ranges.clear();
		geometry.blurs.clear();
		geometry.truncated = false;
		geometry.dropped_paths = 0;
	}

	geometry
}

/// A path that did not fit the curve buffers.
struct Dropped;

/// Places one outline: packs it if needed and returns its [`slug_quad`]. The caller sets the kind, the paint, and the
/// mask. `None` means nothing is visible.
fn place_outline(
	paths: &mut UiPathCurves,
	shape: &UiPathShape,
	position: [f32; 2],
	clip: Option<DrawClip>,
	sx: f32,
	sy: f32,
	viewport_clip: PixelClip,
) -> Option<Result<UiPrimitive, Dropped>> {
	let scale = [shape.scale[0] * sx, shape.scale[1] * sy];
	let clip = pixel_clip(clip, sx, sy, viewport_clip);
	if scale[0] <= 0.0 || scale[1] <= 0.0 || clip.is_empty() {
		return None;
	}
	// Packing does not depend on damage, so an undamaged path still becomes resident.
	let Some(packed) = paths.ensure(shape.key(), &shape.segments) else {
		return Some(Err(Dropped));
	};
	slug_quad(&packed, [position[0] * sx, position[1] * sy], scale, clip).map(Ok)
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{
		UI_SLUG_BAND_CAPACITY, UI_SLUG_CURVE_CAPACITY, UiBlurDrawElement, UiDrawList, UiMaskTable, UiPaint, UiPathCurves,
		UiPathDrawElement, UiPathShape, build_ui_path_geometry_damaged, orient_even_odd, path_outline, sample_polygon,
		signed_area,
	};
	use crate::ui::{
		components::{curve::CurveSegment, path::FillRule},
		render_pass::{
			UI_KIND_PATH, UI_KIND_PATH_BLUR, UI_KIND_SAMPLED_SHADOW, UiBlurSource, UiShapeShadow, UiStep, build_ui_primitives,
		},
	};

	fn shape(id: u64, segments: Vec<CurveSegment>) -> UiPathShape {
		UiPathShape {
			path_id: id,
			version: 0,
			fill_rule: FillRule::NonZero,
			scale: [1.0, 1.0],
			segments: segments.into(),
		}
	}

	fn path_fill(order: u32, segments: Vec<CurveSegment>) -> UiPathDrawElement {
		UiPathDrawElement {
			depth: 0,
			order,
			position: [10.0, 10.0],
			size: [20.0, 20.0],
			clip: None,
			clip_mask: None,
			paint: UiPaint::flat([1.0; 4]),
			shape: shape(order as u64, segments),
		}
	}

	/// A glass element blurs its backdrop under its outline, then tints it, and a highlight draws over both.
	#[test]
	fn path_blur_precedes_its_fill_and_later_paths_follow() {
		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			blurs: vec![UiBlurDrawElement {
				depth: 0,
				order: 2,
				position: [10.0, 10.0],
				size: [20.0, 20.0],
				clip: None,
				clip_mask: None,
				color: [0.0; 4],
				corner_radius: 0.0,
				corner_exponent: 2.0,
				sector: None,
				radius: 8.0,
				path: Some(shape(2, square(0.0, 0.0, 10.0))),
				shadow: None,
			}],
			paths: vec![path_fill(2, square(0.0, 0.0, 10.0)), path_fill(3, square(2.0, 2.0, 4.0))],
			..UiDrawList::default()
		};
		let arena = bumpalo::Bump::new();
		let mut masks = UiMaskTable::default();
		let mut curves = UiPathCurves::new(UI_SLUG_CURVE_CAPACITY, UI_SLUG_BAND_CAPACITY);
		let viewport = Extent::square(100);
		let paths = build_ui_path_geometry_damaged(&draw_list, viewport, &mut curves, &mut masks, &arena, None);
		assert_eq!(paths.blurs.len(), 1);
		assert_eq!(paths.blurs[0].map(|primitive| primitive.kind), Some(UI_KIND_PATH_BLUR));
		assert_eq!(paths.ranges.len(), 2);

		let output = build_ui_primitives(
			&draw_list,
			viewport,
			&arena,
			Vec::new(),
			None,
			&mut masks,
			None,
			Some(&paths),
			None,
		);
		let kinds: Vec<u32> = output.primitives[1..].iter().map(|primitive| primitive.kind).collect();
		assert_eq!(kinds, [UI_KIND_PATH_BLUR, UI_KIND_PATH, UI_KIND_PATH]);
		assert!(matches!(
			output.steps.as_slice(),
			[UiStep::Draw { .. }, UiStep::Blur(_), UiStep::Draw { count: 3, .. }]
		));
	}

	/// A path's shadow draws its moved outline in white outside every draw, blurs it, and composites
	/// the result under the path's fill.
	#[test]
	fn path_shadow_blurs_a_moved_white_caster_under_its_fill() {
		let draw_list = UiDrawList {
			layout_size: [100.0, 100.0],
			blurs: vec![UiBlurDrawElement {
				depth: 0,
				order: 2,
				position: [10.0, 10.0],
				size: [20.0, 20.0],
				clip: None,
				clip_mask: None,
				color: [0.0, 0.0, 0.0, 0.5],
				corner_radius: 0.0,
				corner_exponent: 2.0,
				sector: None,
				radius: 0.0,
				path: Some(shape(2, square(0.0, 0.0, 10.0))),
				shadow: Some(UiShapeShadow {
					offset: [3.0, 4.0],
					sigma: 2.0,
				}),
			}],
			paths: vec![path_fill(2, square(0.0, 0.0, 10.0))],
			..UiDrawList::default()
		};
		let arena = bumpalo::Bump::new();
		let mut masks = UiMaskTable::default();
		let mut curves = UiPathCurves::new(UI_SLUG_CURVE_CAPACITY, UI_SLUG_BAND_CAPACITY);
		let viewport = Extent::square(100);
		let paths = build_ui_path_geometry_damaged(&draw_list, viewport, &mut curves, &mut masks, &arena, None);
		let fill = paths.primitives[0];
		let caster = paths.blurs[0].expect("A path shadow should have a caster");
		assert_eq!(caster.kind, UI_KIND_PATH);
		assert_eq!(caster.color, [1.0; 4]);
		assert_eq!(caster.a, [fill.a[0] + 3.0, fill.a[1] + 4.0, fill.a[2], fill.a[3]]);
		assert_eq!(
			caster.bounds,
			[
				fill.bounds[0] + 3.0,
				fill.bounds[1] + 4.0,
				fill.bounds[2] + 3.0,
				fill.bounds[3] + 4.0
			]
		);

		let output = build_ui_primitives(
			&draw_list,
			viewport,
			&arena,
			Vec::new(),
			None,
			&mut masks,
			None,
			Some(&paths),
			None,
		);
		let kinds: Vec<u32> = output.primitives[1..].iter().map(|primitive| primitive.kind).collect();
		assert_eq!(kinds, [UI_KIND_PATH, UI_KIND_SAMPLED_SHADOW, UI_KIND_PATH]);
		let [
			UiStep::Draw { first: 1, count: 0 },
			UiStep::Blur(blur),
			UiStep::Draw { first: 2, count: 2 },
		] = output.steps.as_slice()
		else {
			panic!("Unexpected steps {:?}", output.steps);
		};
		assert_eq!(blur.source, UiBlurSource::Shape { first: 1, count: 1 });
		let composite = output.primitives[2];
		let reach = 2.0 * 3.0 + 1.0;
		assert_eq!(
			composite.bounds,
			[
				caster.bounds[0] - reach,
				caster.bounds[1] - reach,
				caster.bounds[2] + reach,
				caster.bounds[3] + reach
			]
		);
		assert_eq!(composite.color, [0.0, 0.0, 0.0, 0.5]);
	}

	fn square(x: f32, y: f32, size: f32) -> Vec<CurveSegment> {
		let corner = |dx: f32, dy: f32| (x + dx * size, y + dy * size);
		vec![
			CurveSegment::Line {
				from: corner(0.0, 0.0).into(),
				to: corner(1.0, 0.0).into(),
			},
			CurveSegment::Line {
				from: corner(1.0, 0.0).into(),
				to: corner(1.0, 1.0).into(),
			},
			CurveSegment::Line {
				from: corner(1.0, 1.0).into(),
				to: corner(0.0, 1.0).into(),
			},
		]
	}

	#[test]
	fn open_contours_close_and_flip_y_up() {
		let outline = path_outline(&square(0.0, 0.0, 2.0), FillRule::NonZero);
		assert_eq!(outline.curves.len(), 4, "a fill closes its contour with a fourth edge");
		assert_eq!(outline.bounds, [0.0, -2.0, 2.0, 0.0]);
	}

	#[test]
	fn even_odd_orients_holes_against_their_parent() {
		let mut segments = square(0.0, 0.0, 10.0);
		segments.extend(square(2.0, 2.0, 4.0));
		let outline = path_outline(&segments, FillRule::EvenOdd);
		let (outer, inner) = outline.curves.split_at(4);
		let orientation = |curves: &[[[f32; 2]; 3]]| signed_area(&sample_polygon(curves)).signum();
		assert_ne!(orientation(outer), orientation(inner));

		let nonzero = path_outline(&segments, FillRule::NonZero);
		let (outer, inner) = nonzero.curves.split_at(4);
		assert_eq!(orientation(outer), orientation(inner));
	}

	#[test]
	fn reversing_keeps_contours_closed() {
		let mut curves = path_outline(&square(0.0, 0.0, 1.0), FillRule::NonZero).curves;
		let before: Vec<_> = curves.iter().map(|curve| curve[0]).collect();
		let contour_ends = [curves.len()];
		orient_even_odd(&mut curves, &contour_ends);
		for pair in curves.windows(2) {
			assert_eq!(pair[0][2], pair[1][0]);
		}
		assert_eq!(curves.len(), before.len());
	}
}
