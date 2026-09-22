//! UI filled paths drawn from outlines on the GPU with the Slug algorithm.
//!
//! This is a copy of [`super::slug`] for arbitrary outlines: paths are keyed by content instead of
//! glyph index, packed in their own units with y pointing up, and read from their own two storage
//! buffers. Text and paths stay apart until both are merged.

use std::collections::HashMap;

use super::*;
use crate::ui::components::{
	curve::{CurvePoint, CurveSegment},
	path::FillRule,
};

/// Elements in the GPU path curve buffer. An icon takes a few hundred.
pub(super) const UI_PATH_CURVE_CAPACITY: usize = 1 << 16;
/// Values in the GPU path band buffer. A path takes a few per curve.
pub(super) const UI_PATH_BAND_CAPACITY: usize = 1 << 18;

/// Most bands per axis.
const MAX_BANDS: usize = 16;
/// Overlap between neighboring bands as a fraction of the path's extent.
const BAND_OVERLAP: f32 = 1.0 / 1024.0;
/// The shader's coverage reaches half a pixel past the outline, so every quad grows by this much.
const DILATION_PIXELS: f32 = 0.5;
/// Largest distance between a cubic segment and its quadratic replacements, as a fraction of the path's extent.
const CUBIC_TOLERANCE: f32 = 1.0 / 4096.0;

pub(super) type PathKey = (u64, u64, FillRule);

/// Where the shader finds one packed path, and its bounds in path units with y pointing up.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct PackedPath {
	pub(super) location: u32,
	pub(super) last_band: [u32; 2],
	pub(super) banding: [f32; 4],
	pub(super) bounds: [f32; 4],
}

/// One path's outline as quadratic curves in path units, y pointing up.
pub(super) struct PathOutline {
	pub(super) curves: Vec<[[f32; 2]; 3]>,
	pub(super) bounds: [f32; 4],
}

struct CurveSpan {
	location: u32,
	min: [f32; 2],
	max: [f32; 2],
}

/// The `UiPathCurves` struct packs path outlines into curve and band data for the ubershader.
pub(super) struct UiPathCurves {
	curves: Vec<[f32; 4]>,
	bands: Vec<u32>,
	packed: HashMap<PathKey, PackedPath>,
	uploaded: (usize, usize),
	capacity: (usize, usize),
	generation: u64,
	spans: Vec<CurveSpan>,
}

impl UiPathCurves {
	pub(super) fn new(curve_capacity: usize, band_capacity: usize) -> Self {
		Self {
			curves: Vec::new(),
			bands: Vec::new(),
			packed: HashMap::new(),
			uploaded: (0, 0),
			capacity: (curve_capacity, band_capacity),
			generation: 0,
			spans: Vec::new(),
		}
	}

	/// Identifies the current packing; geometry built against an older generation is stale.
	pub(super) fn generation(&self) -> u64 {
		self.generation
	}

	/// Returns where the shader finds the path, packing its segments on first use.
	///
	/// Returns `None` when the path does not fit in the remaining buffer space.
	pub(super) fn ensure(&mut self, key: PathKey, segments: &[CurveSegment]) -> Option<PackedPath> {
		if let Some(packed) = self.packed.get(&key) {
			return Some(*packed);
		}
		let outline = path_outline(segments, key.2);
		let packed = self.pack(&outline)?;
		self.packed.insert(key, packed);
		Some(packed)
	}

	/// Forgets every path so that a full buffer can hold the paths of the current frame.
	pub(super) fn reset(&mut self) {
		self.curves.clear();
		self.bands.clear();
		self.packed.clear();
		self.uploaded = (0, 0);
		self.generation += 1;
	}

	/// Appends one outline's curves and bands. Same layout as the glyph packer.
	fn pack(&mut self, outline: &PathOutline) -> Option<PackedPath> {
		let Self {
			curves,
			bands,
			spans,
			capacity,
			..
		} = self;
		let (curve_start, band_start) = (curves.len(), bands.len());

		spans.clear();
		let mut contour_end: Option<[f32; 2]> = None;
		for [p1, p2, p3] in &outline.curves {
			if let Some(end) = contour_end.filter(|end| end != p1) {
				curves.push([end[0], end[1], 0.0, 0.0]);
			}
			spans.push(CurveSpan {
				location: curves.len() as u32,
				min: [p1[0].min(p2[0]).min(p3[0]), p1[1].min(p2[1]).min(p3[1])],
				max: [p1[0].max(p2[0]).max(p3[0]), p1[1].max(p2[1]).max(p3[1])],
			});
			curves.push([p1[0], p1[1], p2[0], p2[1]]);
			contour_end = Some(*p3);
		}
		if let Some(end) = contour_end {
			curves.push([end[0], end[1], 0.0, 0.0]);
		}

		let min = [outline.bounds[0], outline.bounds[1]];
		let extent = [outline.bounds[2] - min[0], outline.bounds[3] - min[1]];
		let overlap = extent[0].max(extent[1]) * BAND_OVERLAP;
		let wanted = (outline.curves.len() / 2).clamp(1, MAX_BANDS);
		let sliced = [(1, 0), (0, 1)].map(|(across, along)| (across, along, if extent[across] > 0.0 { wanted } else { 1 }));
		bands.resize(band_start + (sliced[0].2 + sliced[1].2) * 2, 0);

		let mut header = band_start;
		for (across, along, count) in sliced {
			for band in 0..count {
				let low = min[across] + extent[across] * band as f32 / count as f32 - overlap;
				let high = min[across] + extent[across] * (band + 1) as f32 / count as f32 + overlap;
				let list = bands.len();
				bands.extend(spans.iter().enumerate().filter_map(|(index, span)| {
					(span.min[across] != span.max[across] && span.max[across] >= low && span.min[across] <= high)
						.then_some(index as u32)
				}));
				bands[list..].sort_unstable_by(|a, b| spans[*b as usize].max[along].total_cmp(&spans[*a as usize].max[along]));
				for entry in &mut bands[list..] {
					*entry = spans[*entry as usize].location;
				}
				bands[header] = (bands.len() - list) as u32;
				bands[header + 1] = (list - band_start) as u32;
				header += 2;
			}
		}

		if curves.len() > capacity.0 || bands.len() > capacity.1 {
			curves.truncate(curve_start);
			bands.truncate(band_start);
			return None;
		}

		let [horizontal, vertical] = sliced.map(|(across, _, count)| {
			let scale = if extent[across] > 0.0 {
				count as f32 / extent[across]
			} else {
				0.0
			};
			(scale, -min[across] * scale, count as u32 - 1)
		});
		Some(PackedPath {
			location: band_start as u32,
			last_band: [horizontal.2, vertical.2],
			banding: [vertical.0, horizontal.0, vertical.1, horizontal.1],
			bounds: outline.bounds,
		})
	}

	/// Mirrors the data appended since the last upload to the GPU; a no-op when nothing was added.
	pub(super) fn upload(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		curve_buffer: ghi::BufferHandle<[[f32; 4]; UI_PATH_CURVE_CAPACITY]>,
		band_buffer: ghi::BufferHandle<[u32; UI_PATH_BAND_CAPACITY]>,
	) {
		let (curves, bands) = self.uploaded;
		if curves != self.curves.len() {
			frame.get_mut_buffer_slice(curve_buffer)[curves..self.curves.len()].copy_from_slice(&self.curves[curves..]);
			frame.sync_buffer(curve_buffer);
		}
		if bands != self.bands.len() {
			frame.get_mut_buffer_slice(band_buffer)[bands..self.bands.len()].copy_from_slice(&self.bands[bands..]);
			frame.sync_buffer(band_buffer);
		}
		self.uploaded = (self.curves.len(), self.bands.len());
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
	let mut collector = PathCollector::new((span * CUBIC_TOLERANCE).max(1e-5));

	for segment in segments {
		let points = segment_points(segment);
		if !points.iter().all(|point| point.is_finite()) {
			continue;
		}
		let from = points[0];
		if collector.last().is_none_or(|last| last != [from.x, from.y]) {
			collector.move_to([from.x, from.y]);
		}
		match *segment {
			CurveSegment::Line { to, .. } => collector.line([to.x, to.y]),
			CurveSegment::Quadratic { control, to, .. } => collector.quad([control.x, control.y], [to.x, to.y]),
			CurveSegment::Cubic {
				control0, control1, to, ..
			} => collector.cubic([control0.x, control0.y], [control1.x, control1.y], [to.x, to.y]),
		}
	}
	let mut contours = collector.finish();
	if fill_rule == FillRule::EvenOdd {
		orient_even_odd(&mut contours);
	}

	// The shader shares the glyph convention of y pointing up.
	let mut curves = Vec::with_capacity(contours.iter().map(Vec::len).sum());
	let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
	for contour in contours {
		for curve in contour {
			let curve = curve.map(|[x, y]| [x, -y]);
			for point in curve {
				bounds = [
					bounds[0].min(point[0]),
					bounds[1].min(point[1]),
					bounds[2].max(point[0]),
					bounds[3].max(point[1]),
				];
			}
			curves.push(curve);
		}
	}
	if curves.is_empty() {
		bounds = [0.0; 4];
	}
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

/// Collects segments into closed quadratic contours. A copy of the font outline collector
/// without the font unit scale, plus contour bookkeeping.
struct PathCollector {
	contours: Vec<Vec<[[f32; 2]; 3]>>,
	current: Vec<[[f32; 2]; 3]>,
	tolerance: f32,
	start: [f32; 2],
	last: Option<[f32; 2]>,
}

impl PathCollector {
	fn new(tolerance: f32) -> Self {
		Self {
			contours: Vec::new(),
			current: Vec::new(),
			tolerance,
			start: [0.0; 2],
			last: None,
		}
	}

	fn last(&self) -> Option<[f32; 2]> {
		self.last
	}

	fn move_to(&mut self, point: [f32; 2]) {
		self.close();
		self.start = point;
		self.last = Some(point);
	}

	/// A line repeats its end point as the control point, which keeps the curve's polynomial quadratic.
	fn line(&mut self, end: [f32; 2]) {
		let Some(last) = self.last else { return };
		if end != last {
			self.current.push([last, end, end]);
			self.last = Some(end);
		}
	}

	fn quad(&mut self, control: [f32; 2], end: [f32; 2]) {
		let Some(last) = self.last else { return };
		if control != last || end != last {
			self.current.push([last, control, end]);
			self.last = Some(end);
		}
	}

	/// Splits a cubic into enough quadratics to stay within the tolerance.
	fn cubic(&mut self, p1: [f32; 2], p2: [f32; 2], p3: [f32; 2]) {
		let Some(p0) = self.last else { return };
		// The cubic and its closest quadratic differ by the third difference times t (t - 1/2) (t - 1),
		// which peaks at sqrt(3) / 36. Splitting into n pieces divides the third difference by n cubed.
		let third_difference = (0..2)
			.map(|axis| (p3[axis] - 3.0 * p2[axis] + 3.0 * p1[axis] - p0[axis]).powi(2))
			.sum::<f32>()
			.sqrt();
		let pieces = (third_difference * 3f32.sqrt() / 36.0 / self.tolerance)
			.cbrt()
			.ceil()
			.clamp(1.0, 16.0);
		let point = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				s * s * s * p0[axis] + 3.0 * s * s * t * p1[axis] + 3.0 * s * t * t * p2[axis] + t * t * t * p3[axis]
			})
		};
		let derivative = |t: f32| {
			let s = 1.0 - t;
			[0, 1].map(|axis| {
				3.0 * s * s * (p1[axis] - p0[axis]) + 6.0 * s * t * (p2[axis] - p1[axis]) + 3.0 * t * t * (p3[axis] - p2[axis])
			})
		};
		let mut last = p0;
		for piece in 0..pieces as usize {
			let (t0, t1) = (piece as f32 / pieces, (piece + 1) as f32 / pieces);
			// The last piece ends on the segment's own end point so that contours stay closed.
			let end = if t1 >= 1.0 { p3 } else { point(t1) };
			let (from, to) = (derivative(t0), derivative(t1));
			// The piece's cubic controls are start + from * h and end - to * h with h = (t1 - t0) / 3.
			// The closest quadratic control is (3 * (c1 + c2) - (start + end)) / 4.
			let h = (t1 - t0) / 3.0;
			let control = [0, 1]
				.map(|axis| (3.0 * (last[axis] + from[axis] * h + end[axis] - to[axis] * h) - (last[axis] + end[axis])) / 4.0);
			self.current.push([last, control, end]);
			last = end;
		}
		self.last = Some(last);
	}

	/// Ends the current contour with a line back to its start, as a fill closes every subpath.
	fn close(&mut self) {
		if self.last.is_some() {
			self.line(self.start);
		}
		if !self.current.is_empty() {
			self.contours.push(std::mem::take(&mut self.current));
		}
		self.last = None;
	}

	fn finish(mut self) -> Vec<Vec<[[f32; 2]; 3]>> {
		self.close();
		self.contours
	}
}

/// Orients contours so that nonzero winding reproduces the even-odd fill: contours at an even
/// nesting depth run one way and contours inside them run the other. Exact when no contour
/// crosses itself or another one.
fn orient_even_odd(contours: &mut [Vec<[[f32; 2]; 3]>]) {
	let polygons: Vec<Vec<[f32; 2]>> = contours.iter().map(|contour| sample_polygon(contour)).collect();
	let areas: Vec<f32> = polygons.iter().map(|polygon| signed_area(polygon)).collect();
	for (index, contour) in contours.iter_mut().enumerate() {
		let Some(&probe) = polygons[index].first() else { continue };
		let depth = polygons
			.iter()
			.enumerate()
			.filter(|(other, polygon)| *other != index && contains(polygon, probe))
			.count();
		let outward = depth % 2 == 0;
		if (areas[index] > 0.0) != outward {
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

/// Places one outline: packs it if needed and returns a record holding its clipped quad, its
/// origin and pixel scale in `a`, and its band data. The caller sets the kind, the paint, and the
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
	let origin = [position[0] * sx, position[1] * sy];
	let [min_x, min_y, max_x, max_y] = packed.bounds;
	// The packed bounds are y up like a glyph's, so they flip back around the origin here.
	let quad = PixelClip {
		x0: origin[0] + min_x * scale[0] - DILATION_PIXELS,
		y0: origin[1] - max_y * scale[1] - DILATION_PIXELS,
		x1: origin[0] + max_x * scale[0] + DILATION_PIXELS,
		y1: origin[1] - min_y * scale[1] + DILATION_PIXELS,
	}
	.intersect(clip);
	if quad.is_empty() {
		return None;
	}
	Some(Ok(UiPrimitive {
		bounds: [quad.x0, quad.y0, quad.x1, quad.y1],
		a: [origin[0], origin[1], scale[0], scale[1]],
		b: packed.banding,
		data0: packed.location,
		data1: packed.last_band[0] | packed.last_band[1] << 16,
		..UiPrimitive::default()
	}))
}

#[cfg(test)]
mod tests {
	use utils::Extent;

	use super::{
		UI_PATH_BAND_CAPACITY, UI_PATH_CURVE_CAPACITY, UiBlurDrawElement, UiDrawList, UiMaskTable, UiPaint, UiPathCurves,
		UiPathDrawElement, UiPathShape, build_ui_path_geometry_damaged, orient_even_odd, path_outline, sample_polygon,
		signed_area,
	};
	use crate::ui::{
		components::{curve::CurveSegment, path::FillRule},
		render_pass::{UI_KIND_PATH, UI_KIND_PATH_BLUR, UiStep, build_ui_primitives},
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
			}],
			paths: vec![path_fill(2, square(0.0, 0.0, 10.0)), path_fill(3, square(2.0, 2.0, 4.0))],
			..UiDrawList::default()
		};
		let arena = bumpalo::Bump::new();
		let mut masks = UiMaskTable::default();
		let mut curves = UiPathCurves::new(UI_PATH_CURVE_CAPACITY, UI_PATH_BAND_CAPACITY);
		let viewport = Extent::square(100);
		let paths = build_ui_path_geometry_damaged(&draw_list, viewport, &mut curves, &mut masks, &arena, None);
		assert_eq!(paths.blurs.len(), 1);
		assert_eq!(paths.blurs[0].map(|primitive| primitive.kind), Some(UI_KIND_PATH_BLUR));
		assert_eq!(paths.ranges.len(), 2);

		let output = build_ui_primitives(&draw_list, viewport, &arena, None, &mut masks, None, Some(&paths), None);
		let kinds: Vec<u32> = output.primitives[1..].iter().map(|primitive| primitive.kind).collect();
		assert_eq!(kinds, [UI_KIND_PATH_BLUR, UI_KIND_PATH, UI_KIND_PATH]);
		assert!(matches!(
			output.steps.as_slice(),
			[UiStep::Draw { .. }, UiStep::Blur(_), UiStep::Draw { count: 3, .. }]
		));
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
		let mut contours = vec![path_outline(&square(0.0, 0.0, 1.0), FillRule::NonZero).curves];
		let before: Vec<_> = contours[0].iter().map(|curve| curve[0]).collect();
		orient_even_odd(&mut contours);
		for pair in contours[0].windows(2) {
			assert_eq!(pair[0][2], pair[1][0]);
		}
		assert_eq!(contours[0].len(), before.len());
	}
}
