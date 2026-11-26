//! The geometry of a path's outline: its closed quadratic contours with the fill rule applied, which the renderer packs.

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

#[cfg(test)]
mod tests {
	use super::{path_outline, sample_polygon, signed_area};
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
}
