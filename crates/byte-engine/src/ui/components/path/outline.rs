//! The geometry of a path's outline: its closed quadratic contours with the fill rule applied, which the renderer packs,
//! and its optical center, which centering flows line up. See [Visual center](/docs/reference/gui#visual-center).

use super::FillRule;
use crate::ui::{
	components::curve::{CurvePoint, CurveSegment},
	font::{CUBIC_TOLERANCE, QuadraticCollector, curve_bounds},
};

/// One path's outline as quadratic curves in path units, y pointing up.
pub(crate) struct PathOutline {
	pub(crate) curves: Vec<[[f32; 2]; 3]>,
	pub(crate) bounds: [f32; 4],
}

/// Turns segments into quadratic contours, closes them, applies the fill rule, and flips y up.
pub(crate) fn path_outline(segments: &[CurveSegment], fill_rule: FillRule) -> PathOutline {
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

/// The `Ink` type holds a filled area as polygons that never overlap: outer boundaries run one way and holes the other,
/// so every covered point is covered once. Points are in the units of the outline it came from, y pointing down.
///
/// Build one from a path with [`ink`], combine several with [`union`], and measure it with [`optical_shift`].
pub(crate) type Ink = Vec<Vec<[f32; 2]>>;

/// How far a flattened curve may stray from the true one, as a fraction of the outline's larger extent.
const INK_TOLERANCE: f32 = 1.0 / 1024.0;

/// How far the optical center moves from the midpoint of the bounds toward the area centroid. The specification fixes
/// it at one half; see [Visual center](/docs/reference/gui#visual-center).
const CENTROID_WEIGHT: f32 = 0.5;

/// Returns the area `segments` fill under `fill_rule`, with crossings and overlaps resolved so each point counts once.
///
/// Curves are flattened into polygons within [`INK_TOLERANCE`] of the outline's size, then the fill rule is applied by
/// polygon boolean operations, so how the outline was drawn (overlapping contours, crossings, either winding) doesn't
/// change the result.
pub(crate) fn ink(segments: &[CurveSegment], fill_rule: FillRule) -> Ink {
	let mut extent = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
	for point in segments.iter().flat_map(segment_points).filter(|point| point.is_finite()) {
		extent = [
			extent[0].min(point.x),
			extent[1].min(point.y),
			extent[2].max(point.x),
			extent[3].max(point.y),
		];
	}
	let span = ((extent[2] - extent[0]).max(extent[3] - extent[1])).max(0.0);
	let tolerance = (span * INK_TOLERANCE).max(1e-5);

	// A segment that doesn't start where the previous one ended starts a new contour, as the renderer reads them.
	let mut contours: Ink = Vec::new();
	let mut points = Vec::new();
	let mut end: Option<CurvePoint> = None;
	for segment in segments {
		let start = segment_points(segment)[0];
		if end != Some(start) && !points.is_empty() {
			contours.push(points.drain(..).map(|point: CurvePoint| [point.x, point.y]).collect());
		}
		let before = points.len();
		segment.flatten(|point| point, tolerance, &mut points);
		if points.len() > before {
			end = points.last().copied();
		}
	}
	if !points.is_empty() {
		contours.push(points.drain(..).map(|point| [point.x, point.y]).collect());
	}
	contours.retain(|contour| contour.len() >= 3);
	let rule = match fill_rule {
		FillRule::NonZero => i_overlay::core::fill_rule::FillRule::NonZero,
		FillRule::EvenOdd => i_overlay::core::fill_rule::FillRule::EvenOdd,
	};
	resolve(&contours, rule)
}

/// Returns the area any of `parts` covers, each part already resolved by [`ink`] or [`union`].
pub(crate) fn union(parts: &Ink) -> Ink {
	// Each part covers its points once, so a point is inside the union where the summed winding isn't zero.
	resolve(parts, i_overlay::core::fill_rule::FillRule::NonZero)
}

/// Applies `rule` to `contours` with polygon boolean operations, returning non-overlapping polygons.
fn resolve(contours: &Ink, rule: i_overlay::core::fill_rule::FillRule) -> Ink {
	use i_overlay::float::simplify::SimplifyShape;
	if contours.is_empty() {
		return Vec::new();
	}
	contours.simplify_shape(rule).into_iter().flatten().collect()
}

/// Returns how far the optical center of `ink` sits from the midpoint of its bounds, in the ink's units, or `None` when
/// it covers no area.
///
/// The area and its first moments are the polygon forms of Green's theorem, and the bounds are the polygons' extent,
/// so both are exact for the polygons.
pub(crate) fn optical_shift(ink: &Ink) -> Option<[f32; 2]> {
	let (mut area, mut moment_x, mut moment_y) = (0.0f64, 0.0f64, 0.0f64);
	let mut bounds = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
	for contour in ink {
		for (index, &a) in contour.iter().enumerate() {
			let b = contour[(index + 1) % contour.len()];
			// Accumulated in f64, since the moments of a polygon far from the origin cancel to small differences.
			let (ax, ay, bx, by) = (a[0] as f64, a[1] as f64, b[0] as f64, b[1] as f64);
			let cross = ax * by - bx * ay;
			area += cross / 2.0;
			moment_x += (ax + bx) * cross / 6.0;
			moment_y += (ay + by) * cross / 6.0;
			bounds = [
				bounds[0].min(a[0]),
				bounds[1].min(a[1]),
				bounds[2].max(a[0]),
				bounds[3].max(a[1]),
			];
		}
	}
	// An area this small against the bounds is rounding, such as a contour traced forward and back.
	let extent = ((bounds[2] - bounds[0]) * (bounds[3] - bounds[1])) as f64;
	if !(area.abs() > extent * 1e-6) {
		return None;
	}
	let centroid = [(moment_x / area) as f32, (moment_y / area) as f32];
	let middle = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
	Some([
		CENTROID_WEIGHT * (centroid[0] - middle[0]),
		CENTROID_WEIGHT * (centroid[1] - middle[1]),
	])
}

#[cfg(test)]
mod tests {
	use super::{ink, optical_shift, orient_even_odd, path_outline, sample_polygon, signed_area, union};
	use crate::ui::components::{curve::CurveSegment, path::FillRule};

	/// A closed contour of lines through `points`, which the fill closes back to the first point.
	fn polygon(points: &[(f32, f32)]) -> Vec<CurveSegment> {
		points
			.windows(2)
			.map(|pair| CurveSegment::Line {
				from: pair[0].into(),
				to: pair[1].into(),
			})
			.collect()
	}

	fn square(x: f32, y: f32, size: f32) -> Vec<CurveSegment> {
		polygon(&[(x, y), (x + size, y), (x + size, y + size), (x, y + size)])
	}

	fn assert_near(actual: [f32; 2], expected: [f32; 2]) {
		assert!(
			(actual[0] - expected[0]).abs() < 1e-3 && (actual[1] - expected[1]).abs() < 1e-3,
			"{actual:?} is not {expected:?}"
		);
	}

	/// The optical shift of the area `segments` fill.
	fn shift(segments: &[CurveSegment], fill_rule: FillRule) -> Option<[f32; 2]> {
		optical_shift(&ink(segments, fill_rule))
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

	#[test]
	fn a_symmetric_outline_looks_centered_at_its_middle() {
		assert_near(shift(&square(3.0, 5.0, 8.0), FillRule::NonZero).unwrap(), [0.0, 0.0]);
	}

	#[test]
	fn a_triangle_looks_centered_halfway_to_its_centroid() {
		// The right triangle's bounds are centered at (6, 6) and its centroid is at (4, 4), a third of the way in.
		let triangle = polygon(&[(0.0, 0.0), (12.0, 0.0), (0.0, 12.0)]);
		assert_near(shift(&triangle, FillRule::NonZero).unwrap(), [-1.0, -1.0]);
	}

	#[test]
	fn a_play_triangle_looks_centered_toward_its_flat_side() {
		// Pointing right, its centroid sits a third of the way from the flat side at x = 0.
		let play = polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 12.0)]);
		assert_near(shift(&play, FillRule::NonZero).unwrap(), [-0.75, 0.0]);
	}

	#[test]
	fn curves_count_by_their_area_and_extent() {
		// A parabolic segment on the line y = 0, bulging up to its peak at y = -2. Its control point sits at -4, so
		// control-point bounds would be twice as tall. The area's centroid sits 2/5 of the way up, at y = -0.8.
		let segments = vec![
			CurveSegment::Quadratic {
				from: (-2.0, 0.0).into(),
				control: (0.0, -4.0).into(),
				to: (2.0, 0.0).into(),
			},
			CurveSegment::Line {
				from: (2.0, 0.0).into(),
				to: (-2.0, 0.0).into(),
			},
		];
		assert_near(shift(&segments, FillRule::NonZero).unwrap(), [0.0, 0.5 * (-0.8 + 1.0)]);
	}

	#[test]
	fn holes_follow_the_fill_rule() {
		// A 12 unit square with a 4 unit hole in its top-left quarter; even-odd cuts the hole out, so the mass moves
		// toward the bottom right. The full square has area 144 and centroid (6, 6); the hole has area 16 and
		// centroid (4, 4). What's left has centroid (6 * 144 - 4 * 16) / 128 = 6.25 on each axis.
		let mut segments = square(0.0, 0.0, 12.0);
		segments.extend(square(2.0, 2.0, 4.0));
		assert_near(shift(&segments, FillRule::EvenOdd).unwrap(), [0.125, 0.125]);
	}

	#[test]
	fn an_outline_without_area_has_no_optical_center() {
		assert_eq!(shift(&[], FillRule::NonZero), None);
		let line = polygon(&[(0.0, 0.0), (5.0, 5.0)]);
		assert_eq!(shift(&line, FillRule::NonZero), None);
	}

	#[test]
	fn overlapping_contours_count_once() {
		// A 12 unit square with a smaller square drawn over its top-left corner covers just the 12 unit square.
		let mut segments = square(0.0, 0.0, 12.0);
		segments.extend(square(0.0, 0.0, 6.0));
		assert_near(shift(&segments, FillRule::NonZero).unwrap(), [0.0, 0.0]);
	}

	#[test]
	fn either_winding_fills_the_same_area() {
		let clockwise = polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 12.0)]);
		let counterclockwise = polygon(&[(0.0, 12.0), (9.0, 6.0), (0.0, 0.0)]);
		assert_near(shift(&counterclockwise, FillRule::NonZero).unwrap(), shift(&clockwise, FillRule::NonZero).unwrap());
	}

	#[test]
	fn crossing_contours_follow_the_fill_rule() {
		// Two 12 by 4 bars crossing at their middles: even-odd leaves the 4 unit square where they cross empty,
		// non-zero fills it. Each bar covers 48 square units and they share 16, so the non-zero ink covers 80 and the
		// even-odd ink 64.
		let mut segments = polygon(&[(0.0, 4.0), (12.0, 4.0), (12.0, 8.0), (0.0, 8.0)]);
		segments.extend(polygon(&[(4.0, 0.0), (8.0, 0.0), (8.0, 12.0), (4.0, 12.0)]));
		let area = |ink: &super::Ink| {
			ink.iter()
				.map(|contour| super::signed_area(contour))
				.sum::<f32>()
				.abs()
		};
		assert!((area(&ink(&segments, FillRule::EvenOdd)) - 64.0).abs() < 1e-3);
		assert!((area(&ink(&segments, FillRule::NonZero)) - 80.0).abs() < 1e-3);
	}

	#[test]
	fn a_union_counts_shared_area_once() {
		// The play triangle split into two layers that overlap along the middle covers the whole triangle once.
		let top = ink(&polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 8.0)]), FillRule::NonZero);
		let bottom = ink(&polygon(&[(0.0, 4.0), (9.0, 6.0), (0.0, 12.0)]), FillRule::NonZero);
		let both: super::Ink = top.into_iter().chain(bottom).collect();
		assert_near(optical_shift(&union(&both)).unwrap(), [-0.75, 0.0]);
	}
}
