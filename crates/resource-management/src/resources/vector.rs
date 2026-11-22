//! Vector images: filled outlines the UI draws as paths at any size, such as icons.

/// The `Vector` struct lets an application draw a picture made of filled shapes, such as an icon, sharp at any size.
///
/// The model holds the whole picture; the payload is empty. Bake one from an SVG file with
/// [`crate::asset::handler::implementations::svg::SvgAssetHandler`], request it through
/// [`crate::ResourceManager::request`], and give it to the UI to draw. See
/// [SVG vector images](/docs/reference/svg) for what a bake keeps.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Vector {
	/// The width and height of the box the layers are drawn in.
	pub view_box: [f32; 2],
	/// The shapes, from back to front.
	pub layers: Vec<VectorLayer>,
	/// How far the optical center of the area the layers cover sits from the midpoint of that area's bounds, in box
	/// units, which centering flows line up; `None` when the layers cover no area. Measure it with [`optical_shift`].
	pub optical_shift: Option<[f32; 2]>,
}

/// The `VectorLayer` struct is one filled shape of a [`Vector`], which the UI draws as one path.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct VectorLayer {
	/// The outline, as closed contours of joined segments in box units, with y pointing down.
	pub segments: Vec<VectorSegment>,
	/// Whether crossing and nested contours fill by the even-odd rule instead of the non-zero rule.
	pub even_odd: bool,
	/// What fills the outline.
	pub paint: VectorPaint,
	/// The standard deviation, in box units, of the blur the layer lays over what's drawn behind it; 0 for none.
	pub backdrop_blur: f32,
}

/// The `VectorSegment` enum is one piece of a [`VectorLayer`]'s outline, with its points in box units.
#[derive(
	Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum VectorSegment {
	Line([f32; 2], [f32; 2]),
	Quadratic([f32; 2], [f32; 2], [f32; 2]),
	Cubic([f32; 2], [f32; 2], [f32; 2], [f32; 2]),
}

/// The `VectorPaint` enum is what fills a [`VectorLayer`], with colors as red, green, blue, and alpha from 0 to 1, as
/// the UI draws them.
#[derive(
	Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub enum VectorPaint {
	Color([f32; 4]),
	/// A gradient from the first color at `from` to the second at `to`, in box units.
	LinearGradient {
		from: [f32; 2],
		to: [f32; 2],
		colors: [[f32; 4]; 2],
	},
}

super::impl_direct_resource!(Vector, "Vector");

/// How far a flattened curve may stray from the true one, as a fraction of the outline's larger extent. See
/// [Visual center](/docs/reference/gui#visual-center).
pub const FLATTENING_TOLERANCE: f32 = 1.0 / 1024.0;

/// How far the optical center moves from the midpoint of the bounds toward the area centroid. The specification fixes
/// it at one half; see [Visual center](/docs/reference/gui#visual-center).
const CENTROID_WEIGHT: f64 = 0.5;

/// The `Ink` type holds a filled area as polygons that never overlap: outer boundaries run one way and holes the other,
/// so every covered point is covered once. Points are in box units, y pointing down.
pub(crate) type Ink = Vec<Vec<[f64; 2]>>;

/// Returns how far the optical center of the area `layers` cover sits from the midpoint of that area's bounds, in box
/// units, or `None` when they cover no area. Each layer is its outline and whether it fills by the even-odd rule.
///
/// Bakes store it in [`Vector`]; the UI measures vector images built in code with it. Every covered point counts once,
/// however many layers or contours cover it. See [Visual center](/docs/reference/gui#visual-center).
pub fn optical_shift<'a>(layers: impl IntoIterator<Item = (&'a [VectorSegment], bool)>) -> Option<[f32; 2]> {
	let layers: Vec<_> = layers.into_iter().collect();
	let mut extent = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
	for point in layers
		.iter()
		.flat_map(|(segments, _)| segments.iter())
		.flat_map(segment_points)
	{
		extent = [
			extent[0].min(point[0]),
			extent[1].min(point[1]),
			extent[2].max(point[0]),
			extent[3].max(point[1]),
		];
	}
	let span = (extent[2] - extent[0]).max(extent[3] - extent[1]);
	if !(span > 0.0) {
		return None;
	}
	let tolerance = span * FLATTENING_TOLERANCE;
	let mut parts = Ink::new();
	for (segments, even_odd) in layers {
		parts.extend(ink(segments, even_odd, tolerance));
	}
	// Each part covers its points once, so a point is inside the union where the summed winding isn't zero.
	shift_of(&resolve(parts, false))
}

/// Returns the area `segments` fill by their fill rule, with curves flattened within `tolerance` box units.
pub(crate) fn ink(segments: &[VectorSegment], even_odd: bool, tolerance: f32) -> Ink {
	resolve(contours(segments, tolerance), even_odd)
}

/// Applies a fill rule to `contours` with polygon boolean operations, returning non-overlapping polygons.
pub(crate) fn resolve(mut contours: Ink, even_odd: bool) -> Ink {
	use i_overlay::{core::fill_rule::FillRule, float::simplify::SimplifyShape};
	contours.retain(|contour| contour.len() >= 3);
	if contours.is_empty() {
		return Ink::new();
	}
	let rule = if even_odd { FillRule::EvenOdd } else { FillRule::NonZero };
	contours.simplify_shape(rule).concat()
}

/// Flattens `segments` into polygons whose edges stay within about `tolerance` of the curves. A segment that doesn't
/// start where the one before ended starts a new contour, as the renderer reads them.
fn contours(segments: &[VectorSegment], tolerance: f32) -> Ink {
	let mut contours = Ink::new();
	let mut end: Option<[f32; 2]> = None;
	for segment in segments {
		let points = segment_points(segment);
		if end != Some(points[0]) {
			contours.push(vec![[points[0][0] as f64, points[0][1] as f64]]);
		}
		let contour = contours.last_mut().expect("A contour was just started.");
		let length: f32 = points
			.windows(2)
			.map(|pair| (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]))
			.sum();
		// Lines need no steps between their ends; curves take enough that each step stays close to the curve.
		let steps = match segment {
			VectorSegment::Line(..) => 1,
			_ => ((length / tolerance.max(f32::EPSILON)).sqrt().ceil() as usize).clamp(1, 256),
		};
		for step in 1..=steps {
			let point = segment_point(segment, step as f32 / steps as f32);
			contour.push([point[0] as f64, point[1] as f64]);
		}
		end = Some(points[points.len() - 1]);
	}
	contours
}

/// The optical shift of `ink`: half the distance from the midpoint of its bounds to its centroid.
///
/// The area and its first moments are the polygon forms of Green's theorem, and the bounds are the polygons' extent,
/// so both are exact for the polygons.
fn shift_of(ink: &Ink) -> Option<[f32; 2]> {
	let (mut area, mut moment_x, mut moment_y) = (0.0f64, 0.0f64, 0.0f64);
	let mut bounds = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
	for contour in ink {
		for (index, &[ax, ay]) in contour.iter().enumerate() {
			let [bx, by] = contour[(index + 1) % contour.len()];
			let cross = ax * by - bx * ay;
			area += cross / 2.0;
			moment_x += (ax + bx) * cross / 6.0;
			moment_y += (ay + by) * cross / 6.0;
			bounds = [bounds[0].min(ax), bounds[1].min(ay), bounds[2].max(ax), bounds[3].max(ay)];
		}
	}
	// An area this small against the bounds is rounding, such as a contour traced forward and back.
	let extent = (bounds[2] - bounds[0]) * (bounds[3] - bounds[1]);
	if !(area.abs() > extent * 1e-6) {
		return None;
	}
	let centroid = [moment_x / area, moment_y / area];
	let middle = [(bounds[0] + bounds[2]) * 0.5, (bounds[1] + bounds[3]) * 0.5];
	Some([
		(CENTROID_WEIGHT * (centroid[0] - middle[0])) as f32,
		(CENTROID_WEIGHT * (centroid[1] - middle[1])) as f32,
	])
}

/// A segment's points: its ends and control points, in order.
fn segment_points(segment: &VectorSegment) -> Vec<[f32; 2]> {
	match *segment {
		VectorSegment::Line(a, b) => vec![a, b],
		VectorSegment::Quadratic(a, b, c) => vec![a, b, c],
		VectorSegment::Cubic(a, b, c, d) => vec![a, b, c, d],
	}
}

/// The point at `t`, from 0 at the segment's start to 1 at its end.
fn segment_point(segment: &VectorSegment, t: f32) -> [f32; 2] {
	let u = 1.0 - t;
	match *segment {
		VectorSegment::Line(a, b) => [0, 1].map(|axis| u * a[axis] + t * b[axis]),
		VectorSegment::Quadratic(a, b, c) => [0, 1].map(|axis| u * u * a[axis] + 2.0 * u * t * b[axis] + t * t * c[axis]),
		VectorSegment::Cubic(a, b, c, d) => {
			[0, 1].map(|axis| u * u * u * a[axis] + 3.0 * u * u * t * b[axis] + 3.0 * u * t * t * c[axis] + t * t * t * d[axis])
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{VectorSegment, optical_shift};

	/// A closed contour of lines through `points`, which the fill closes back to the first point.
	fn polygon(points: &[(f32, f32)]) -> Vec<VectorSegment> {
		points
			.windows(2)
			.map(|pair| VectorSegment::Line([pair[0].0, pair[0].1], [pair[1].0, pair[1].1]))
			.collect()
	}

	fn square(x: f32, y: f32, size: f32) -> Vec<VectorSegment> {
		polygon(&[(x, y), (x + size, y), (x + size, y + size), (x, y + size)])
	}

	fn assert_near(actual: [f32; 2], expected: [f32; 2]) {
		assert!(
			(actual[0] - expected[0]).abs() < 1e-3 && (actual[1] - expected[1]).abs() < 1e-3,
			"{actual:?} is not {expected:?}"
		);
	}

	/// The optical shift of the area one layer fills.
	fn shift(segments: &[VectorSegment], even_odd: bool) -> Option<[f32; 2]> {
		optical_shift([(segments, even_odd)])
	}

	#[test]
	fn a_symmetric_outline_looks_centered_at_its_middle() {
		assert_near(shift(&square(3.0, 5.0, 8.0), false).unwrap(), [0.0, 0.0]);
	}

	#[test]
	fn a_triangle_looks_centered_halfway_to_its_centroid() {
		// The right triangle's bounds are centered at (6, 6) and its centroid is at (4, 4), a third of the way in.
		let triangle = polygon(&[(0.0, 0.0), (12.0, 0.0), (0.0, 12.0)]);
		assert_near(shift(&triangle, false).unwrap(), [-1.0, -1.0]);
	}

	#[test]
	fn a_play_triangle_looks_centered_toward_its_flat_side() {
		// Pointing right, its centroid sits a third of the way from the flat side at x = 0.
		let play = polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 12.0)]);
		assert_near(shift(&play, false).unwrap(), [-0.75, 0.0]);
	}

	#[test]
	fn curves_count_by_their_area_and_extent() {
		// A parabolic segment on the line y = 0, bulging up to its peak at y = -2. Its control point sits at -4, so
		// control-point bounds would be twice as tall. The area's centroid sits 2/5 of the way up, at y = -0.8.
		let segments = [
			VectorSegment::Quadratic([-2.0, 0.0], [0.0, -4.0], [2.0, 0.0]),
			VectorSegment::Line([2.0, 0.0], [-2.0, 0.0]),
		];
		assert_near(shift(&segments, false).unwrap(), [0.0, 0.5 * (-0.8 + 1.0)]);
	}

	#[test]
	fn holes_follow_the_fill_rule() {
		// A 12 unit square with a 4 unit hole in its top-left quarter; even-odd cuts the hole out, so the mass moves
		// toward the bottom right. What's left has centroid (6 * 144 - 4 * 16) / 128 = 6.25 on each axis.
		let mut segments = square(0.0, 0.0, 12.0);
		segments.extend(square(2.0, 2.0, 4.0));
		assert_near(shift(&segments, true).unwrap(), [0.125, 0.125]);
	}

	#[test]
	fn an_outline_without_area_has_no_optical_center() {
		assert_eq!(shift(&[], false), None);
		assert_eq!(shift(&polygon(&[(0.0, 0.0), (5.0, 5.0)]), false), None);
	}

	#[test]
	fn overlapping_contours_count_once() {
		// A 12 unit square with a smaller square drawn over its top-left corner covers just the 12 unit square.
		let mut segments = square(0.0, 0.0, 12.0);
		segments.extend(square(0.0, 0.0, 6.0));
		assert_near(shift(&segments, false).unwrap(), [0.0, 0.0]);
	}

	#[test]
	fn either_winding_fills_the_same_area() {
		let clockwise = polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 12.0)]);
		let counterclockwise = polygon(&[(0.0, 12.0), (9.0, 6.0), (0.0, 0.0)]);
		assert_near(shift(&counterclockwise, false).unwrap(), shift(&clockwise, false).unwrap());
	}

	#[test]
	fn layers_that_overlap_count_their_shared_area_once() {
		// The play triangle split into two layers that overlap along the middle covers the whole triangle once.
		let top = polygon(&[(0.0, 0.0), (9.0, 6.0), (0.0, 8.0)]);
		let bottom = polygon(&[(0.0, 4.0), (9.0, 6.0), (0.0, 12.0)]);
		assert_near(
			optical_shift([(&top[..], false), (&bottom[..], false)]).unwrap(),
			[-0.75, 0.0],
		);
	}
}
