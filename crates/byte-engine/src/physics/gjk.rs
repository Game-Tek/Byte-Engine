//! The parts of the GJK algorithm, which finds how far apart two convex shapes are, or that they overlap.
//!
//! GJK works on the Minkowski difference of the two shapes: every point of shape A minus every point of shape B,
//! which is the Minkowski sum of shape A and the mirror image of shape B. The shapes overlap exactly when the
//! difference holds the origin, and otherwise its point closest to the origin is as far from the origin as the shapes
//! are from each other. The difference is never built. [`minkowski_support`] finds its furthest point along one
//! direction from the furthest point of each shape, and GJK closes in on the origin with a few of those.
//!
//! Call [`shapes_overlap`] to test two shapes, [`shapes_contact`] to also find where and how deep they overlap, or
//! [`shapes_closest_points`] to find how far apart they are. All three run one search, and each takes from it what
//! it needs. [`rounded_shapes_contact`] is for shapes that are a simpler shape made thicker all around, as a ball
//! and a capsule are. [`rounded_shape_ray_cast`] finds where a ray meets such a shape.

use math::{Point, Ray, UnitVector, Vector, WorldSpace, collision::Intersection};

use super::epa::{self, ContactScratch};

/// The `MinkowskiPoint` struct keeps a point of the Minkowski difference of two shapes with the point of each shape
/// that it came from, so that a result found in the difference can be turned back into points on the shapes.
///
/// Get one from [`minkowski_support`].
#[derive(Debug, PartialEq)]
pub struct MinkowskiPoint<Space = WorldSpace> {
	difference: Vector<Space>,
	point_on_a: Point<Space>,
	point_on_b: Point<Space>,
}

// `derive` would make the point copyable only in a space that is copyable itself, and spaces are bare markers.
impl<Space> Copy for MinkowskiPoint<Space> {}

impl<Space> Clone for MinkowskiPoint<Space> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<Space> MinkowskiPoint<Space> {
	/// Returns the point of the difference, as its offset from the origin: the point on shape A minus the point on
	/// shape B.
	pub fn difference(&self) -> Vector<Space> {
		self.difference
	}

	/// Returns the point of shape A that the difference was taken from.
	pub fn point_on_a(&self) -> Point<Space> {
		self.point_on_a
	}

	/// Returns the point of shape B that the difference was taken from.
	pub fn point_on_b(&self) -> Point<Space> {
		self.point_on_b
	}
}

/// Returns the point of the Minkowski difference of shape A and shape B that is furthest along `direction`.
///
/// `support_a` and `support_b` each return the point of their shape that is furthest along the direction they are
/// given, as [`crate::physics::ConvexHull::support_point`] does for a hull. Both have to answer in one space, so for
/// two bodies they have to place each shape where its body is.
///
/// No point of shape A is further along `direction` than the one `support_a` returns, and no point of shape B is
/// less far along it than the one `support_b` returns for the opposite direction. So no difference between a point
/// of each is further along it than the difference between those two.
pub fn minkowski_support<Space>(
	support_a: impl Fn(Vector<Space>) -> Point<Space>,
	support_b: impl Fn(Vector<Space>) -> Point<Space>,
	direction: Vector<Space>,
) -> MinkowskiPoint<Space> {
	let point_on_a = support_a(direction);
	let point_on_b = support_b(-direction);
	MinkowskiPoint {
		difference: point_on_a - point_on_b,
		point_on_a,
		point_on_b,
	}
}

/// Returns whether shape A and shape B overlap, for any two convex shapes.
///
/// `support_a` and `support_b` each return the point of their shape that is furthest along the direction they are
/// given, as [`crate::physics::ConvexHull::support_point`] does for a hull. Both have to answer in one space, so for
/// two bodies they have to place each shape where its body is.
///
/// Shapes that only touch, or that are less than a ten-thousandth of a unit apart or into each other, may be
/// reported either way.
pub fn shapes_overlap<Space>(
	support_a: impl Fn(Vector<Space>) -> Point<Space>,
	support_b: impl Fn(Vector<Space>) -> Point<Space>,
) -> bool {
	close_in(
		|direction| minkowski_support(&support_a, &support_b, direction),
		0.0,
		any_direction(),
	)
	.holds_origin
}

/// Returns the contact between shape A and shape B when they overlap, for any two convex shapes.
///
/// The contact's normal is the direction in which shape B has the shortest way out of shape A, its depth is how far
/// that is, and its points are where each shape reaches furthest into the other.
///
/// `support_a` and `support_b` each return the point of their shape that is furthest along the direction they are
/// given, as [`crate::physics::ConvexHull::support_point`] does for a hull. Both have to answer in one space, so for
/// two bodies they have to place each shape where its body is.
///
/// `scratch` is working storage. Pass the same one each time to find contact after contact without allocating.
///
/// Returns `None` when the shapes are apart. Shapes less than a ten-thousandth of a unit apart may get a contact
/// without depth. Also returns `None` when the shapes overlap but have no volume between them, as two points or two
/// segments on one line have not, because nothing then says which way is out.
pub fn shapes_contact<Space>(
	support_a: impl Fn(Vector<Space>) -> Point<Space>,
	support_b: impl Fn(Vector<Space>) -> Point<Space>,
	scratch: &mut ContactScratch<Space>,
) -> Option<Intersection<Space>> {
	rounded_shapes_contact(support_a, 0.0, support_b, 0.0, None, None, 0.0, scratch)
}

/// Returns the contact between shape A and shape B when they overlap, where each shape is every point within its
/// radius of a core shape. A ball is a point with a radius, and a capsule is a segment with one.
///
/// This finds a round shape's contact from its core, which has few corners, so the contact is as exact as that of
/// two shapes with flat sides and takes as little work. Passing the round shape's own furthest points to
/// [`shapes_contact`] takes more, because flat faces follow a round surface only roughly.
///
/// `support_a` and `support_b` each return the point of their core shape that is furthest along the direction they
/// are given, as [`crate::physics::ConvexHull::support_point`] does for a hull. Both have to answer in one space, so
/// for two bodies they have to place each shape where its body is. A radius of zero leaves a shape as its core.
///
/// `toward_b` is a direction that leads from shape A toward shape B, such as from the middle of one to the middle
/// of the other. The search starts along it, and when the shapes are apart along it, which most shapes that are
/// apart are, that is all the search takes. Any direction leads to the same contact. Pass `None` when there is
/// none at hand.
///
/// `extent` is a bound on how far apart a point of one core and a point of the other can be along any one
/// coordinate axis, which the boxes around the two shapes give. It may be several times too large, and must not be
/// too small. Pass `None` when there is none at hand: the cores are then asked for six more of their furthest
/// points when they overlap.
///
/// `within` is how far apart the shapes may be and still get a contact. Such a contact has a negative depth, which
/// is the gap between the shapes, a normal that leads from shape A to shape B across the gap, and the points of the
/// two shapes that are nearest each other. Pass zero to get contacts only for shapes that touch. Pass infinity to
/// learn how far apart any two shapes are and in which direction, as moving a fast body up to another one needs.
///
/// `scratch` is working storage. Pass the same one each time to find contact after contact without allocating.
///
/// Returns `None` when the shapes are more than `within` apart, or when their cores overlap but have no volume
/// between them, as two points at one place have not, because nothing then says which way is out.
pub fn rounded_shapes_contact<Space>(
	support_a: impl Fn(Vector<Space>) -> Point<Space>,
	radius_a: f32,
	support_b: impl Fn(Vector<Space>) -> Point<Space>,
	radius_b: f32,
	toward_b: Option<Vector<Space>>,
	extent: Option<f32>,
	within: f32,
	scratch: &mut ContactScratch<Space>,
) -> Option<Intersection<Space>> {
	// GJK settles most pairs, the ones that are apart, with a few points of the difference between the cores.
	let support = |direction| minkowski_support(&support_a, &support_b, direction);
	let reach = radius_a + radius_b;
	// Cores that are further apart than this belong to shapes that are more than `within` apart.
	let furthest = reach + within;
	let search = close_in(support, furthest, toward_b.unwrap_or_else(any_direction));

	let (normal, depth, on_a, on_b) = if search.holds_origin {
		// The cores overlap. The difference's surface is explored to find how far inside it the origin is, starting
		// from the points that GJK ended with. The shapes are deeper in each other than their cores by their radii.
		let cores = epa::penetration(support, &search.simplex[..search.count], extent, scratch)?;
		(cores.normal(), cores.depth() + reach, cores.point_on_a(), cores.point_on_b())
	} else {
		// The cores are apart, and the shapes overlap when that is by less than their radii. Their surfaces are
		// nearest each other along the line between the cores' closest points.
		if search.nearest.distance_squared > furthest * furthest {
			return None;
		}
		let (on_a, on_b) = search.points();
		let (normal, distance) = (-search.nearest.point).normalize_with_length().ok()?;
		(normal, reach - distance, on_a, on_b)
	};
	Some(Intersection::new(
		normal,
		depth,
		on_a + normal * radius_a,
		on_b - normal * radius_b,
	))
}

/// Returns the point of shape A and the point of shape B that are closest to each other, in that sequence, for any
/// two convex shapes that are apart. The distance between the two points is the gap between the shapes.
///
/// `support_a` and `support_b` each return the point of their shape that is furthest along the direction they are
/// given, as [`crate::physics::ConvexHull::support_point`] does for a hull. Both have to answer in one space, so for
/// two bodies they have to place each shape where its body is.
///
/// Returns `None` when the shapes overlap, touch, or are less than a ten-thousandth of a unit apart. Use
/// [`shapes_contact`] for those. Where the shapes face each other with flat sides or parallel edges, many pairs of
/// points are equally close, and one of them is returned.
pub fn shapes_closest_points<Space>(
	support_a: impl Fn(Vector<Space>) -> Point<Space>,
	support_b: impl Fn(Vector<Space>) -> Point<Space>,
) -> Option<(Point<Space>, Point<Space>)> {
	let search = close_in(
		|direction| minkowski_support(&support_a, &support_b, direction),
		f32::INFINITY,
		any_direction(),
	);
	(!search.holds_origin).then(|| search.points())
}

/// Returns how far along `ray` the ray first meets a shape that is every point within `radius` of a convex core
/// shape, and the shape's outward normal there, when that is no further than `max_distance` along it.
///
/// This is how a ray or segment query tests one shape, after the boxes around shapes have ruled out the ones the
/// ray does not come near. For a segment, start the ray at one end, point it at the other, and pass the segment's
/// length as `max_distance`. Pass infinity for a ray without an end.
///
/// `support` returns the point of the core that is furthest along the direction it is given, as
/// [`crate::physics::ConvexHull::support_point`] does for a hull. It has to answer in the ray's space. A ball is a
/// point with a radius, and a capsule is a segment with one. A radius of zero leaves the shape as its core.
///
/// A ray that starts inside the shape or on its surface meets it at a distance of zero, with a normal that leads
/// back along the ray, since it does not enter the shape anywhere.
///
/// The distance is short of where the ray meets the shape by about a ten-thousandth of a unit at most, so the
/// point it gives is never inside the shape.
///
/// Returns `None` when the ray passes the shape or ends before it. A ray that only grazes the shape may be reported
/// either way.
pub fn rounded_shape_ray_cast<Space>(
	support: impl Fn(Vector<Space>) -> Point<Space>,
	radius: f32,
	ray: &Ray<Space>,
	max_distance: f32,
) -> Option<(f32, UnitVector<Space>)> {
	// This is the ray cast of Gino van den Bergen's "Ray Casting against General Convex Objects with Application to
	// Continuous Collision Detection". A point moves along the ray, and GJK closes in on the core's point nearest
	// it. The core's furthest point toward the moving point puts a plane across the way to it, and the shape is
	// behind that plane moved out by the radius. While the moving point is beyond that plane, it can move up to it
	// without entering the shape. The simplex is of differences between the moving point and points of the core,
	// which are taken again whenever the point moves.
	let direction = ray.direction();
	let from_core = |at: Point<Space>, on_core: Point<Space>| MinkowskiPoint {
		difference: at - on_core,
		point_on_a: at,
		point_on_b: on_core,
	};
	let mut distance = 0.0;
	let mut at = ray.origin();
	let mut simplex = [from_core(at, support(any_direction())); 4];
	let mut count = 1;
	let mut nearest = nearest_on_simplex(&simplex[..count]);
	// The direction of the last plane the point moved up to. Where a flat side stops the ray, it is the side's
	// normal.
	let mut facing = None;

	for _ in 0..STEP_LIMIT {
		// The simplex is part of the core, so the core is no further from the point than the simplex is. The ray has
		// met the shape once what is left between them is less than a ten-thousandth of a unit along the ray. A ray
		// that slants across the shape's surface covers more than the gap before it meets it, by one over how
		// directly it heads in, which `nearest.point` gives scaled by the gap.
		let gap = nearest.distance_squared.sqrt();
		let heading_in = nearest.point.dot(direction.into_vector()).abs();
		if (gap - radius) * gap <= RAY_REACH * heading_in || count == 4 {
			break;
		}

		// Both of these are scaled by the gap, which is the length of `nearest.point`.
		let on_core = support(nearest.point);
		let beyond = nearest.point.dot(at - on_core) - radius * gap;
		let moves = beyond > 0.0;
		if moves {
			// The point is beyond the plane. A ray that does not lead toward the plane never reaches the shape behind
			// it.
			let approach = -nearest.point.dot(direction.into_vector());
			if approach.is_nan() || approach <= 0.0 {
				return None;
			}
			distance += beyond / approach;
			if distance.is_nan() || distance > max_distance {
				return None;
			}
			at = ray.point_at(distance);
			facing = Some(nearest.point);
			for point in &mut simplex[..count] {
				*point = from_core(at, point.point_on_b);
			}
		}

		let point = from_core(at, on_core);
		let added = !has_point(&simplex[..count], &point);
		if added {
			simplex[count] = point;
		}
		let next = nearest_on_simplex(&simplex[..count + usize::from(added)]);
		// When the point stays where it is and the simplex does not get nearer to it, GJK has found the core's point
		// nearest it, as far as rounding lets it.
		if !moves && (!added || next.weights[count] == 0.0) {
			break;
		}
		nearest = next;
		count = keep_weighted(&mut simplex, &mut nearest.weights);
	}

	// The search ran out of steps or rounding stopped it short of the shape, which only happens to rays that graze
	// it. A ray whose point came within a ten-thousandth of a unit of the shape meets it there.
	if nearest.distance_squared.sqrt() > radius + RAY_REACH {
		return None;
	}
	let Some(facing) = facing else {
		return Some((0.0, -direction));
	};
	// Where the shape is round, its normal leads from the core's nearest point to where the ray meets it. Where it is
	// flat, the core is the shape and that point is where the ray meets it, so the plane that stopped the ray gives
	// the normal.
	let normal = if radius > 0.0 {
		nearest.point.normalized()
	} else {
		facing.normalized()
	};
	Some((distance, normal.or_else(|_| facing.normalized()).ok()?))
}

/// The `Search` struct holds what GJK ends with, so that the test for overlap, the contact, and the closest points
/// all come from one search.
struct Search<Space> {
	/// The points of the Minkowski difference that the search ended on. Only the first `count` mean anything.
	simplex: [MinkowskiPoint<Space>; 4],
	count: usize,
	/// The simplex's point nearest the origin, with how much of each of those points makes it up.
	nearest: Nearest<4, Space>,
	/// Whether that nearest point is the origin, which is when the shapes touch or overlap.
	holds_origin: bool,
}

impl<Space> Search<Space> {
	/// Returns the point of shape A and the point of shape B that the simplex's nearest point is the offset between.
	///
	/// A point of the difference is a point of shape A minus one of shape B. So the nearest point's share of each
	/// point of the simplex, taken of the points on each shape that they came from, gives the two points.
	fn points(&self) -> (Point<Space>, Point<Space>) {
		let on_shape = |point_on: fn(&MinkowskiPoint<Space>) -> Point<Space>| {
			(0..self.count).fold(Point::origin(), |sum, index| {
				sum + (point_on(&self.simplex[index]) - Point::origin()) * self.nearest.weights[index]
			})
		};
		(on_shape(MinkowskiPoint::point_on_a), on_shape(MinkowskiPoint::point_on_b))
	}
}

/// How near the origin a simplex's nearest point has to be to count as the origin.
const ORIGIN_REACH: f32 = 0.0001;

/// How much further along the way to the origin a new point has to be than the simplex is, as a share of the
/// distances involved, for the search to go on. Less than this is rounding.
const FURTHER_REACH: f32 = 1e-6;

/// How near a ray's point has to come to a shape, along the ray, to count as meeting it.
const RAY_REACH: f32 = 0.0001;

/// How many points the search asks for at most. Shapes with flat sides need under ten, and balls needed up to 35. A
/// search that is cut off here ends with the nearest simplex found so far.
const STEP_LIMIT: usize = 64;

/// Moves a simplex of points of the Minkowski difference toward the origin, with points from `support`, until it
/// holds the origin or the difference has no point nearer the origin than the simplex has.
///
/// The simplex is made of points of the difference, so it is inside the difference. When it holds the origin, the
/// shapes overlap. When it cannot get nearer, its nearest point is the difference's nearest, which is the offset
/// between the closest points of the two shapes.
///
/// The search also ends as soon as the shapes are known to be more than `reach` apart, which is sooner than the
/// nearest point is known. The simplex is then further than `reach` from the origin too. Pass zero to only learn
/// whether the shapes overlap, and infinity to always get the nearest point.
///
/// `first` is the direction the search starts along. Any direction will do. One that leads from the first shape
/// toward the second starts the simplex where the difference is nearest the origin, and for most shapes that are
/// apart already shows that they are.
fn close_in<Space>(
	support: impl Fn(Vector<Space>) -> MinkowskiPoint<Space>,
	reach: f32,
	first: Vector<Space>,
) -> Search<Space> {
	// Any point of the difference can start the simplex.
	let start = support(first);
	let mut simplex = [start; 4];
	let mut count = 1;
	let mut nearest = nearest_on_simplex(&simplex[..count]);
	// The difference reaches no further along the first direction than its first point. When that is short of the
	// origin by more than `reach`, the shapes are that far apart, as in the search below. A first point that counts
	// as the origin is left to the search, which ends on it.
	let along = first.dot(start.difference);
	let at_origin = nearest.distance_squared < ORIGIN_REACH * ORIGIN_REACH;
	if !at_origin && along < 0.0 && along * along > reach * reach * first.length_squared() {
		return Search {
			simplex,
			count,
			nearest,
			holds_origin: false,
		};
	}

	for _ in 0..STEP_LIMIT {
		if nearest.distance_squared < ORIGIN_REACH * ORIGIN_REACH {
			break;
		}

		// Look for the point of the difference that is furthest toward the origin from the simplex.
		let direction = -nearest.point;
		let point = support(direction);
		// The simplex already reaches as far that way as the difference does.
		if has_point(&simplex[..count], &point) {
			break;
		}
		// Even the furthest point that way is short of the origin, so the whole difference is, by that point's
		// distance along the way. The direction is as long as the simplex is far from the origin.
		let along = direction.dot(point.difference);
		if along < 0.0 && along * along > reach * reach * nearest.distance_squared {
			break;
		}
		// No point of the difference is further along the way to the origin than the new one. When it is no further
		// along than the simplex is, the simplex's nearest point is the difference's.
		//
		// This is told from how far along the new point is, and not from whether it brings the simplex nearer the
		// origin. Sliding the nearest point sideways along a flat side or an edge of the difference changes its
		// distance from the origin by too little for rounding to show, long before it is under the origin.
		//
		// The simplex's points are rounded in proportion to how far from the origin they are, and so is the nearest
		// point made of them. Further along by less than a share of those distances is not further along at all.
		if along + nearest.distance_squared <= FURTHER_REACH * (nearest.distance_squared + point.difference.length_squared()) {
			break;
		}

		simplex[count] = point;
		let next = nearest_on_simplex(&simplex[..count + 1]);
		// A new point that is further along has a share in the new nearest point. When rounding leaves it none, the
		// simplex stays as it was, and would stay so for every point after this one.
		if next.weights[count] == 0.0 {
			break;
		}
		nearest = next;
		// Only the points that the nearest point is made of are needed to carry on from it.
		count = keep_weighted(&mut simplex, &mut nearest.weights);
	}

	Search {
		simplex,
		count,
		holds_origin: nearest.distance_squared < ORIGIN_REACH * ORIGIN_REACH,
		nearest,
	}
}

/// Moves the points of `simplex` that have a weight, and their `weights`, to the front, in their sequence, and
/// returns how many they are. The points and weights after them are left over and mean nothing.
fn keep_weighted<Space>(simplex: &mut [MinkowskiPoint<Space>; 4], weights: &mut [f32; 4]) -> usize {
	let mut kept = 0;
	for index in 0..4 {
		if weights[index] != 0.0 {
			simplex[kept] = simplex[index];
			weights[kept] = weights[index];
			kept += 1;
		}
	}
	kept
}

/// Returns a direction to start a search along when nothing is known about where the shapes are.
fn any_direction<Space>() -> Vector<Space> {
	Vector::new(1.0, 1.0, 1.0)
}

/// How near each other two points of a Minkowski difference have to be to count as the same point.
const SAME_POINT: f32 = 1e-6;

/// Returns whether `simplex` already holds `point`: a point of the Minkowski difference at the same place, to within
/// a millionth of a unit, whichever points of the shapes it came from.
///
/// When [`minkowski_support`] returns a point that the simplex already holds, the difference reaches no further
/// toward the origin than the simplex does, so GJK has found the nearest point and stops.
fn has_point<Space>(simplex: &[MinkowskiPoint<Space>], point: &MinkowskiPoint<Space>) -> bool {
	simplex
		.iter()
		.any(|held| (held.difference - point.difference).length_squared() < SAME_POINT * SAME_POINT)
}

/// The `Nearest` struct holds the point of a simplex that is nearest the origin with what was worked out on the way
/// to it, so that a larger simplex built on this one, and GJK after it, use those values and do not work them out
/// again.
pub(super) struct Nearest<const N: usize, Space> {
	/// How much of each corner of the simplex makes up the nearest point. The weights add up to one, and a corner
	/// with a weight of zero is not needed to reach the point, so GJK drops it from its simplex.
	pub(super) weights: [f32; N],
	/// The nearest point, as its offset from the origin.
	pub(super) point: Vector<Space>,
	/// The length of `point`, squared.
	pub(super) distance_squared: f32,
}

// `derive` would make this copyable only in a space that is copyable itself, and spaces are bare markers.
impl<const N: usize, Space> Copy for Nearest<N, Space> {}

impl<const N: usize, Space> Clone for Nearest<N, Space> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<const N: usize, Space> Nearest<N, Space> {
	fn new(weights: [f32; N], point: Vector<Space>) -> Self {
		Self {
			weights,
			point,
			distance_squared: point.length_squared(),
		}
	}

	/// Returns the same point as one of a larger simplex that has `corners[n]` of its corners where this simplex has
	/// its corner `n`. The larger simplex's other corners get no weight.
	fn among<const M: usize>(self, corners: [usize; N]) -> Nearest<M, Space> {
		let mut weights = [0.0; M];
		for (corner, weight) in corners.into_iter().zip(self.weights) {
			weights[corner] = weight;
		}
		Nearest {
			weights,
			point: self.point,
			distance_squared: self.distance_squared,
		}
	}
}

/// Returns the point of `simplex` that is nearest the origin.
///
/// The simplex is the one to four points of the Minkowski difference that GJK holds: a point, a segment, a triangle,
/// or a tetrahedron. The weights past its last point are zero.
///
/// # Panics
///
/// Panics when the simplex has no points or more than four.
fn nearest_on_simplex<Space>(simplex: &[MinkowskiPoint<Space>]) -> Nearest<4, Space> {
	match simplex {
		[a] => Nearest::new([1.0], a.difference).among([0]),
		[a, b] => nearest_on_segment(a.difference, b.difference).among([0, 1]),
		[a, b, c] => nearest_on_triangle(a.difference, b.difference, c.difference).among([0, 1, 2]),
		[a, b, c, d] => nearest_on_tetrahedron(a.difference, b.difference, c.difference, d.difference),
		_ => panic!(
			"A simplex has one to four points, not {}. The most likely cause is a point added to a simplex that already held the origin.",
			simplex.len()
		),
	}
}

/// Returns the point of the segment from `a` to `b` that is nearest the origin.
///
/// The ends are points of a Minkowski difference, given as their offsets from the origin.
fn nearest_on_segment<Space>(a: Vector<Space>, b: Vector<Space>) -> Nearest<2, Space> {
	let along = b - a;
	let length_squared = along.length_squared();
	// A segment without length is one point, and the first end stands for it.
	if length_squared == 0.0 {
		return Nearest::new([1.0, 0.0], a);
	}

	// The point of the whole line through the ends that is nearest the origin.
	let projected = a + along * (along.dot(-a) / length_squared);

	// That point is compared with the ends on one axis: the one the segment is longest on, where dividing by the
	// segment's extent loses the least to rounding.
	let along = along.to_array();
	let axis = (0..3)
		.max_by(|&left: &usize, &right: &usize| along[left].abs().total_cmp(&along[right].abs()))
		.unwrap_or(0);
	let (start, end, between) = (a.to_array()[axis], b.to_array()[axis], projected.to_array()[axis]);

	// Each end's weight is the signed length from the projected point to the other end, as a share of the segment.
	// Both are positive when the projected point is between the ends.
	let weights = [(end - between) / along[axis], (between - start) / along[axis]];
	if weights[0] > 0.0 && weights[1] > 0.0 {
		Nearest::new(weights, projected)
	} else if weights[1] <= 0.0 {
		// The projected point is at the first end or beyond it.
		Nearest::new([1.0, 0.0], a)
	} else {
		Nearest::new([0.0, 1.0], b)
	}
}

/// Returns the point of the triangle `a`, `b`, `c` that is nearest the origin.
///
/// The corners are points of a Minkowski difference, given as their offsets from the origin.
pub(super) fn nearest_on_triangle<Space>(a: Vector<Space>, b: Vector<Space>, c: Vector<Space>) -> Nearest<3, Space> {
	let corners = [a, b, c];
	let normal = (b - a).cross(c - a);
	nearest_on_face(corners, normal, a.dot(normal), |first, second| {
		nearest_on_segment(corners[first], corners[second])
	})
}

/// Returns the point of the triangle with these `corners` that is nearest the origin, from values that whoever asks
/// may already have: the triangle's `normal`, which is the cross product of its sides from the first corner to the
/// second and to the third, and its `height`, which is the first corner's length along that normal.
///
/// `edge` returns the nearest point on the edge from one corner to another, both given by their place in `corners`.
/// It is asked only when the nearest point is not inside the triangle, and then once for each edge.
fn nearest_on_face<Space>(
	corners: [Vector<Space>; 3],
	normal: Vector<Space>,
	height: f32,
	mut edge: impl FnMut(usize, usize) -> Nearest<2, Space>,
) -> Nearest<3, Space> {
	let normal_squared = normal.length_squared();

	// A triangle without area has no plane to project on, and is covered by its edges below.
	if normal_squared > 0.0 {
		// The point of the triangle's whole plane that is nearest the origin.
		let projected = normal * (height / normal_squared);

		// That point is compared with the triangle looking along one axis: the one the triangle shows the most of its
		// area along, where dividing by that area loses the least to rounding. Seen along an axis, the triangle's
		// area is that component of its normal.
		let areas = normal.to_array();
		let axis = (0..3)
			.max_by(|&left: &usize, &right: &usize| areas[left].abs().total_cmp(&areas[right].abs()))
			.unwrap_or(0);
		let flat = |vector: Vector<Space>| {
			let vector = vector.to_array();
			[vector[(axis + 1) % 3], vector[(axis + 2) % 3]]
		};
		let center = flat(projected);
		let spokes = corners.map(|corner| {
			let corner = flat(corner);
			[corner[0] - center[0], corner[1] - center[1]]
		});

		// Each corner's weight is the signed area of the triangle that the projected point makes with the other two
		// corners, as a share of the whole triangle. All are positive when the projected point is inside it.
		let weights = [0, 1, 2].map(|corner| {
			let (from, to) = (spokes[(corner + 1) % 3], spokes[(corner + 2) % 3]);
			(from[0] * to[1] - from[1] * to[0]) / areas[axis]
		});
		if weights.iter().all(|&weight| weight > 0.0) {
			return Nearest::new(weights, projected);
		}
	}

	// The nearest point is on an edge. Find it on each, and keep the nearest of the three.
	let mut nearest = edge(1, 2).among([1, 2]);
	for [first, second] in [[2, 0], [0, 1]] {
		let on_edge = edge(first, second);
		if on_edge.distance_squared < nearest.distance_squared {
			nearest = on_edge.among([first, second]);
		}
	}
	nearest
}

/// Returns the point of the tetrahedron `a`, `b`, `c`, `d` that is nearest the origin. When every weight is more than
/// zero, the origin is inside the tetrahedron, and the point is the origin itself.
///
/// The corners are points of a Minkowski difference, given as their offsets from the origin.
fn nearest_on_tetrahedron<Space>(a: Vector<Space>, b: Vector<Space>, c: Vector<Space>, d: Vector<Space>) -> Nearest<4, Space> {
	let corners = [a, b, c, d];

	// Each face is three corners in a row, counted around from its first one, and leaves the corner before its first
	// one out. Its normal and how far along it the face is serve twice: for telling whether the origin is inside the
	// tetrahedron, and for finding the nearest point on the face when it is not.
	let faces: [[usize; 3]; 4] = std::array::from_fn(|first| [first, (first + 1) % 4, (first + 2) % 4]);
	let normals = faces.map(|[first, second, third]| (corners[second] - corners[first]).cross(corners[third] - corners[first]));
	let heights: [f32; 4] = std::array::from_fn(|face| corners[faces[face][0]].dot(normals[face]));

	// Each corner's weight is the signed volume of the tetrahedron that the origin makes with the other three
	// corners, as a share of the whole tetrahedron. That volume is the height of the face that leaves the corner
	// out, with the sign that the face's place around the tetrahedron gives it. These are the cofactors of the
	// bottom row of the matrix that has a corner and a one in each column, which is why their signs alternate, and
	// they add up to that matrix's determinant.
	let mut volumes = [0.0; 4];
	for face in 0..4 {
		volumes[(face + 3) % 4] = if face % 2 == 0 { heights[face] } else { -heights[face] };
	}
	let volume: f32 = volumes.iter().sum();

	// All are positive when the origin is inside. A tetrahedron without volume has no inside, and is covered by its
	// faces below.
	let weights = volumes.map(|part| part / volume);
	if volume != 0.0 && weights.iter().all(|&weight| weight > 0.0) {
		return Nearest {
			weights,
			point: Vector::zero(),
			distance_squared: 0.0,
		};
	}

	// The nearest point is on a face. Find it on each, and keep the nearest of the four. Every edge is shared by two
	// faces, and its nearest point is worked out for the first that asks.
	let mut edges: [Option<Nearest<2, Space>>; 6] = [None; 6];
	let mut nearest: Option<Nearest<4, Space>> = None;
	for face in 0..4 {
		let on_face = nearest_on_face(
			faces[face].map(|corner| corners[corner]),
			normals[face],
			heights[face],
			|first, second| shared_edge(&mut edges, &corners, faces[face][first], faces[face][second]),
		);
		if nearest.is_none_or(|nearest| on_face.distance_squared < nearest.distance_squared) {
			nearest = Some(on_face.among(faces[face]));
		}
	}
	nearest.unwrap_or(Nearest::new([1.0, 0.0, 0.0, 0.0], a))
}

/// Returns the nearest point on the edge of a tetrahedron from corner `from` to corner `to`, and keeps it in `edges`
/// for the other face that shares the edge.
fn shared_edge<Space>(
	edges: &mut [Option<Nearest<2, Space>>; 6],
	corners: &[Vector<Space>; 4],
	from: usize,
	to: usize,
) -> Nearest<2, Space> {
	// An edge is kept as worked out from its lower corner to its higher one, in the slot of that pair among the six
	// pairs of four corners.
	let (low, high) = (from.min(to), from.max(to));
	let edge = *edges[low * (5 - low) / 2 + high - 1].get_or_insert_with(|| nearest_on_segment(corners[low], corners[high]));
	if from == low { edge } else { edge.among([1, 0]) }
}

#[cfg(test)]
mod tests {
	use math::AABB;

	use super::*;
	use crate::physics::{ConvexHull, LocalSpace, build_convex_hull};

	/// Finds a contact with working storage of its own, as [`super::shapes_contact`] does with storage it is given.
	fn shapes_contact(
		support_a: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		support_b: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
	) -> Option<Intersection<LocalSpace>> {
		super::shapes_contact(support_a, support_b, &mut ContactScratch::default())
	}

	/// Finds a contact with working storage of its own, as [`super::rounded_shapes_contact`] does with storage it is
	/// given.
	fn rounded_shapes_contact(
		support_a: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		radius_a: f32,
		support_b: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		radius_b: f32,
	) -> Option<Intersection<LocalSpace>> {
		super::rounded_shapes_contact(
			support_a,
			radius_a,
			support_b,
			radius_b,
			None,
			None,
			0.0,
			&mut ContactScratch::default(),
		)
	}

	/// Returns a box hull with the given half sizes around `middle`.
	fn box_hull(half_sizes: [f32; 3], middle: [f32; 3]) -> ConvexHull {
		let mut corners = Vec::new();
		for x in [-1.0, 1.0] {
			for y in [-1.0, 1.0] {
				for z in [-1.0, 1.0] {
					corners.push(Point::new(
						middle[0] + x * half_sizes[0],
						middle[1] + y * half_sizes[1],
						middle[2] + z * half_sizes[2],
					));
				}
			}
		}
		build_convex_hull(&corners).expect("a box encloses a volume")
	}

	/// Returns the point that `weights` make of `corners`.
	fn weighted<const N: usize>(corners: [Vector<LocalSpace>; N], weights: [f32; N]) -> Vector<LocalSpace> {
		(0..N).fold(Vector::zero(), |sum, corner| sum + corners[corner] * weights[corner])
	}

	/// Asserts that `weights` make the point of the simplex with these `corners` that is closest to the origin.
	///
	/// A point of a convex shape is its closest to the origin exactly when no other point of the shape is nearer the
	/// origin along the line to it. For a simplex it is enough that no corner is.
	fn assert_closest<const N: usize>(corners: [Vector<LocalSpace>; N], weights: [f32; N], margin: f32) {
		assert!(weights.iter().all(|&weight| weight >= 0.0), "{weights:?}");
		assert!((weights.iter().sum::<f32>() - 1.0).abs() <= 1e-5, "{weights:?}");

		let closest = weighted(corners, weights);
		for corner in corners {
			let nearer = closest.dot(closest - corner);
			assert!(nearer <= margin, "a corner is {nearer} nearer for {weights:?} of {corners:?}");
		}
	}

	/// Asserts that `nearest` is the point of the simplex with these `corners` that is nearest the origin, and that
	/// its point and its distance are what its weights make of the corners.
	fn assert_nearest<const N: usize>(corners: [Vector<LocalSpace>; N], nearest: Nearest<N, LocalSpace>) {
		assert_closest(corners, nearest.weights, 1e-4);
		assert!((weighted(corners, nearest.weights) - nearest.point).length() <= 1e-5);
		assert!((nearest.point.length_squared() - nearest.distance_squared).abs() <= 1e-6);
	}

	/// Returns a source of vectors with coordinates from -1 to 1 that are the same on every run.
	fn scattered() -> impl FnMut() -> Vector<LocalSpace> {
		let mut state = 0x2545_f491_4f6c_dd1du64;
		let mut next = move || {
			state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			(state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
		};
		move || Vector::new(next(), next(), next())
	}

	/// Returns a point of a Minkowski difference at `difference` from the origin.
	fn at(difference: [f32; 3]) -> MinkowskiPoint<LocalSpace> {
		minkowski_support(|_| Point::from_array(difference), |_| Point::origin(), Vector::zero())
	}

	fn assert_near<const N: usize>(weights: [f32; N], expected: [f32; N]) {
		assert!(
			(0..N).all(|corner| (weights[corner] - expected[corner]).abs() <= 1e-6),
			"{weights:?} are not {expected:?}"
		);
	}

	#[test]
	fn the_difference_reaches_as_far_as_any_pair_of_points() {
		let a = box_hull([1.0, 2.0, 0.5], [0.0, 0.0, 0.0]);
		let b = box_hull([0.5, 0.5, 3.0], [4.0, 1.0, -2.0]);

		for direction in [
			[1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[0.3, -2.0, 0.7],
			[-0.2, 0.9, -1.5],
			[1.0, 1.0, 1.0],
		] {
			let direction = Vector::from_array(direction);

			let support = minkowski_support(|along| a.support_point(along), |along| b.support_point(along), direction);

			assert!(a.vertices().contains(&support.point_on_a()));
			assert!(b.vertices().contains(&support.point_on_b()));
			assert_eq!(support.difference(), support.point_on_a() - support.point_on_b());

			let furthest = a
				.vertices()
				.iter()
				.flat_map(|&on_a| b.vertices().iter().map(move |&on_b| direction.dot(on_a - on_b)))
				.fold(f32::NEG_INFINITY, f32::max);
			assert_eq!(direction.dot(support.difference()), furthest);
		}
	}

	#[test]
	fn the_difference_of_shapes_apart_stays_on_one_side_of_the_origin() {
		// The second box is 2 beyond the first along x, so even the difference's furthest point toward +x is short of
		// the origin by that gap.
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let b = box_hull([1.0, 1.0, 1.0], [4.0, 0.0, 0.0]);

		let support = minkowski_support(
			|along| a.support_point(along),
			|along| b.support_point(along),
			Vector::new(1.0, 0.0, 0.0),
		);

		assert_eq!(support.difference().x(), -2.0);
	}

	#[test]
	fn the_closest_point_between_the_ends_takes_a_share_of_both() {
		let (a, b) = (Vector::<LocalSpace>::new(-1.0, 1.0, 0.0), Vector::new(3.0, 1.0, 0.0));

		assert_eq!(nearest_on_segment(a, b).weights, [0.75, 0.25]);
		assert_eq!(nearest_on_segment(b, a).weights, [0.25, 0.75]);
	}

	#[test]
	fn the_closest_point_of_a_slanted_segment_is_square_to_it() {
		let (a, b) = (Vector::<LocalSpace>::new(2.0, -1.0, 0.5), Vector::new(-1.0, 3.0, 4.0));

		let weights = nearest_on_segment(a, b).weights;

		assert!(weights.iter().all(|&weight| weight > 0.0));
		assert!((weights[0] + weights[1] - 1.0).abs() <= 1e-6);
		assert!(weighted([a, b], weights).dot(b - a).abs() <= 1e-5);
	}

	#[test]
	fn the_closest_point_beyond_an_end_is_that_end() {
		let (near, far) = (Vector::<LocalSpace>::new(1.0, 2.0, 0.0), Vector::new(5.0, 2.0, 0.0));

		assert_eq!(nearest_on_segment(near, far).weights, [1.0, 0.0]);
		assert_eq!(nearest_on_segment(far, near).weights, [0.0, 1.0]);
		// The origin is square to an end exactly.
		let end = Vector::<LocalSpace>::new(0.0, 2.0, 0.0);
		assert_eq!(nearest_on_segment(end, far).weights, [1.0, 0.0]);
		assert_eq!(nearest_on_segment(far, end).weights, [0.0, 1.0]);
	}

	#[test]
	fn a_segment_without_length_is_its_first_end() {
		let point = Vector::<LocalSpace>::new(1.0, 2.0, 3.0);

		assert_eq!(nearest_on_segment(point, point).weights, [1.0, 0.0]);
	}
	#[test]
	fn the_closest_point_inside_a_triangle_takes_a_share_of_every_corner() {
		// The origin is straight below the middle of the triangle.
		let (a, b, c) = (
			Vector::<LocalSpace>::new(-1.0, -1.0, 2.0),
			Vector::new(2.0, -1.0, 2.0),
			Vector::new(-1.0, 2.0, 2.0),
		);

		assert_near(nearest_on_triangle(a, b, c).weights, [1.0 / 3.0; 3]);
		// Listing the corners the other way around turns the triangle over, and changes nothing.
		assert_near(nearest_on_triangle(c, b, a).weights, [1.0 / 3.0; 3]);
	}

	#[test]
	fn the_closest_point_beside_a_triangle_is_on_its_edge_or_corner() {
		let (a, b, c) = (
			Vector::<LocalSpace>::new(1.0, -1.0, 0.0),
			Vector::new(1.0, 1.0, 0.0),
			Vector::new(3.0, 0.0, 0.0),
		);
		assert_eq!(nearest_on_triangle(a, b, c).weights, [0.5, 0.5, 0.0]);
		assert_eq!(nearest_on_triangle(c, a, b).weights, [0.0, 0.5, 0.5]);

		let (a, b, c) = (
			Vector::<LocalSpace>::new(1.0, 1.0, 0.0),
			Vector::new(3.0, 1.0, 0.0),
			Vector::new(1.0, 3.0, 1.0),
		);
		assert_eq!(nearest_on_triangle(a, b, c).weights, [1.0, 0.0, 0.0]);
		assert_eq!(nearest_on_triangle(b, c, a).weights, [0.0, 0.0, 1.0]);
	}

	#[test]
	fn a_triangle_without_area_is_its_longest_edge() {
		// The corners are in line, and the origin is nearest the stretch between the last two.
		let (a, b, c) = (
			Vector::<LocalSpace>::new(-4.0, 1.0, 0.0),
			Vector::new(-1.0, 1.0, 0.0),
			Vector::new(3.0, 1.0, 0.0),
		);

		let weights = nearest_on_triangle(a, b, c).weights;

		assert_closest([a, b, c], weights, 1e-6);
		assert_eq!(weighted([a, b, c], weights), Vector::new(0.0, 1.0, 0.0));

		let point = Vector::<LocalSpace>::new(1.0, 2.0, 3.0);
		assert_closest([point; 3], nearest_on_triangle(point, point, point).weights, 1e-6);
	}

	#[test]
	fn the_origin_inside_a_tetrahedron_takes_a_share_of_every_corner() {
		let corners = [
			Vector::<LocalSpace>::new(1.0, 1.0, 1.0),
			Vector::new(-1.0, -1.0, 1.0),
			Vector::new(-1.0, 1.0, -1.0),
			Vector::new(1.0, -1.0, -1.0),
		];
		let [a, b, c, d] = corners;

		assert_near(nearest_on_tetrahedron(a, b, c, d).weights, [0.25; 4]);
		// Swapping two corners turns the tetrahedron inside out, and changes nothing.
		assert_near(nearest_on_tetrahedron(b, a, c, d).weights, [0.25; 4]);

		// Moved so that the origin is nearer the first corner, but still inside.
		let moved = corners.map(|corner| corner - Vector::new(0.5, 0.5, 0.5));
		let weights = nearest_on_tetrahedron(moved[0], moved[1], moved[2], moved[3]).weights;
		assert_near(weights, [0.625, 0.125, 0.125, 0.125]);
		assert!(weighted(moved, weights).length() <= 1e-6);
	}

	#[test]
	fn the_closest_point_outside_a_tetrahedron_is_on_its_surface() {
		let corners = [
			Vector::<LocalSpace>::new(0.0, 0.0, 0.0),
			Vector::new(1.0, 0.0, 0.0),
			Vector::new(0.0, 1.0, 0.0),
			Vector::new(0.0, 0.0, 1.0),
		];
		let weights_from = |origin: [f32; 3]| {
			let [a, b, c, d] = corners.map(|corner| corner - Vector::from_array(origin));
			nearest_on_tetrahedron(a, b, c, d).weights
		};

		// Outside each of the three faces that touch the first corner.
		assert_near(weights_from([0.25, 0.25, -2.0]), [0.5, 0.25, 0.25, 0.0]);
		assert_near(weights_from([0.25, -2.0, 0.25]), [0.5, 0.25, 0.0, 0.25]);
		assert_near(weights_from([-2.0, 0.25, 0.25]), [0.5, 0.0, 0.25, 0.25]);
		// Beside the edge between the first two corners.
		assert_near(weights_from([0.25, -1.0, -1.0]), [0.75, 0.25, 0.0, 0.0]);
		// Beyond the last corner.
		assert_near(weights_from([0.0, 0.0, 3.0]), [0.0, 0.0, 0.0, 1.0]);
		// Outside the slanted face, which is the only one that does not touch the first corner.
		assert_near(weights_from([1.0, 1.0, 1.0]), [0.0, 1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0]);
	}

	#[test]
	fn a_tetrahedron_without_volume_is_its_faces() {
		// A flat square, with the origin above its middle.
		let corners = [
			Vector::<LocalSpace>::new(-1.0, -1.0, 2.0),
			Vector::new(1.0, -1.0, 2.0),
			Vector::new(1.0, 1.0, 2.0),
			Vector::new(-1.0, 1.0, 2.0),
		];
		let [a, b, c, d] = corners;

		let weights = nearest_on_tetrahedron(a, b, c, d).weights;

		assert_closest(corners, weights, 1e-6);
		assert!((weighted(corners, weights) - Vector::new(0.0, 0.0, 2.0)).length() <= 1e-6);
	}

	#[test]
	fn scattered_simplices_get_their_closest_point() {
		let mut next = scattered();
		for _ in 0..2000 {
			// Half of the simplices are moved off the origin, so that it is outside most of those.
			let offset = next() * 2.0;
			let [a, b, c, d] = [next(), next(), next(), next()].map(|corner| corner + offset);
			assert_nearest([a, b], nearest_on_segment(a, b));
			assert_nearest([a, b, c], nearest_on_triangle(a, b, c));
			assert_nearest([a, b, c, d], nearest_on_tetrahedron(a, b, c, d));

			let [a, b, c, d] = [next(), next(), next(), next()];
			assert_nearest([a, b, c, d], nearest_on_tetrahedron(a, b, c, d));
		}
	}
	#[test]
	fn the_nearest_point_of_one_point_is_that_point() {
		let nearest = nearest_on_simplex(&[at([1.0, -2.0, 3.0])]);

		assert_eq!(nearest.weights, [1.0, 0.0, 0.0, 0.0]);
		assert_eq!(nearest.point, Vector::new(1.0, -2.0, 3.0));
		assert_eq!(nearest.distance_squared, 14.0);
	}

	#[test]
	fn the_nearest_point_of_a_simplex_comes_with_its_weights_and_distance() {
		let segment = nearest_on_simplex(&[at([-1.0, 1.0, 0.0]), at([3.0, 1.0, 0.0])]);
		assert_eq!(segment.weights, [0.75, 0.25, 0.0, 0.0]);
		assert_eq!(segment.point, Vector::new(0.0, 1.0, 0.0));
		assert_eq!(segment.distance_squared, 1.0);

		let triangle = nearest_on_simplex(&[at([-1.0, -1.0, 2.0]), at([2.0, -1.0, 2.0]), at([-1.0, 2.0, 2.0])]);
		assert_near(triangle.weights, [1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0]);
		assert!((triangle.point - Vector::new(0.0, 0.0, 2.0)).length() <= 1e-6);
		assert!((triangle.distance_squared - 4.0).abs() <= 1e-5);

		// The origin is below the face of the first three points, so the fourth is not needed.
		let tetrahedron = nearest_on_simplex(&[
			at([-0.25, -0.25, 2.0]),
			at([0.75, -0.25, 2.0]),
			at([-0.25, 0.75, 2.0]),
			at([-0.25, -0.25, 3.0]),
		]);
		assert_near(tetrahedron.weights, [0.5, 0.25, 0.25, 0.0]);
		assert!((tetrahedron.point - Vector::new(0.0, 0.0, 2.0)).length() <= 1e-6);
		assert!((tetrahedron.distance_squared - 4.0).abs() <= 1e-5);
	}

	#[test]
	fn the_nearest_point_of_a_tetrahedron_around_the_origin_is_the_origin() {
		let tetrahedron = nearest_on_simplex(&[
			at([1.0, 1.0, 1.0]),
			at([-1.0, -1.0, 1.0]),
			at([-1.0, 1.0, -1.0]),
			at([1.0, -1.0, -1.0]),
		]);

		assert_near(tetrahedron.weights, [0.25; 4]);
		assert_eq!(tetrahedron.point, Vector::zero());
		assert_eq!(tetrahedron.distance_squared, 0.0);
	}

	#[test]
	#[should_panic(expected = "A simplex has one to four points")]
	fn a_simplex_without_points_has_no_nearest_point() {
		nearest_on_simplex::<LocalSpace>(&[]);
	}

	#[test]
	fn a_simplex_holds_a_point_at_the_same_place() {
		let simplex = [at([1.0, 0.0, 0.0]), at([0.0, 2.0, 0.0]), at([0.0, 0.0, 3.0])];

		assert!(has_point(&simplex, &at([0.0, 2.0, 0.0])));
		assert!(has_point(&simplex, &at([0.0, 2.0, 0.0000005])));
		assert!(!has_point(&simplex, &at([0.0, 2.0, 0.00001])));
		assert!(!has_point(&simplex, &at([1.0, 2.0, 3.0])));
		assert!(!has_point(&[], &at([1.0, 0.0, 0.0])));

		// The same difference between other points of the shapes is still the same point of the difference.
		let elsewhere = minkowski_support(|_| Point::new(5.0, 0.0, 0.0), |_| Point::new(4.0, 0.0, 0.0), Vector::zero());
		assert!(has_point(&simplex, &elsewhere));
	}
	/// Returns the furthest point of a ball along a direction.
	fn ball(center: [f32; 3], radius: f32) -> impl Fn(Vector<LocalSpace>) -> Point<LocalSpace> {
		move |direction| match direction.normalized() {
			Ok(direction) => Point::from_array(center) + direction * radius,
			Err(_) => Point::from_array(center),
		}
	}

	/// Returns a hull of a cube with a half size of one, turned an eighth of a turn about the z-axis, around `middle`.
	fn turned_cube(middle: [f32; 3]) -> ConvexHull {
		let reach = std::f32::consts::SQRT_2;
		let mut corners = Vec::new();
		for [x, y] in [[reach, 0.0], [0.0, reach], [-reach, 0.0], [0.0, -reach]] {
			for z in [-1.0, 1.0] {
				corners.push(Point::new(middle[0] + x, middle[1] + y, middle[2] + z));
			}
		}
		build_convex_hull(&corners).expect("a cube encloses a volume")
	}

	#[test]
	fn hulls_overlap_when_they_share_space() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let overlap = |b: &ConvexHull| shapes_overlap(|along| a.support_point(along), |along| b.support_point(along));

		assert!(overlap(&box_hull([1.0, 1.0, 1.0], [1.5, 0.5, -0.5])));
		assert!(!overlap(&box_hull([1.0, 1.0, 1.0], [2.5, 0.5, -0.5])));
		// One inside the other, and the other way around.
		assert!(overlap(&box_hull([0.25, 0.25, 0.25], [0.5, -0.5, 0.25])));
		assert!(overlap(&box_hull([5.0, 6.0, 7.0], [1.0, 2.0, 3.0])));
		assert!(overlap(&a));
	}

	#[test]
	fn hulls_whose_bounds_overlap_can_be_apart() {
		// The turned cube shows a slanted side to the vertical edge of the first at (1, 1). That side is 1 from the
		// turned cube's middle, and the bounds of the two overlap long before the edge reaches it.
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let overlap = |b: &ConvexHull| shapes_overlap(|along| a.support_point(along), |along| b.support_point(along));

		assert!(!overlap(&turned_cube([2.3, 2.3, 0.0])));
		assert!(!overlap(&turned_cube([1.75, 1.75, 0.0])));
		assert!(overlap(&turned_cube([1.65, 1.65, 0.0])));
		assert!(overlap(&turned_cube([1.0, 1.5, 0.5])));
	}

	#[test]
	fn scattered_boxes_overlap_when_their_bounds_do() {
		let mut next = scattered();
		let mut overlapping = 0;
		for _ in 0..500 {
			let (middle_a, middle_b) = (next().to_array(), (next() * 2.0).to_array());
			let (half_a, half_b) = (
				next().to_array().map(|size| size.abs() + 0.1),
				next().to_array().map(|size| size.abs() + 0.1),
			);
			// How far apart the boxes are on the axis that separates them most. Under zero, they overlap.
			let gap = (0..3)
				.map(|axis| (middle_a[axis] - middle_b[axis]).abs() - half_a[axis] - half_b[axis])
				.fold(f32::NEG_INFINITY, f32::max);
			let (a, b) = (box_hull(half_a, middle_a), box_hull(half_b, middle_b));

			let overlap = shapes_overlap(|along| a.support_point(along), |along| b.support_point(along));

			// Boxes that all but touch may be reported either way.
			if gap.abs() > 1e-3 {
				assert_eq!(overlap, gap < 0.0, "boxes {gap} apart");
			}
			overlapping += usize::from(overlap);
		}
		// Both answers have to come up for the comparison to mean anything.
		assert!((100..400).contains(&overlapping), "{overlapping} of 500 overlap");
	}

	#[test]
	fn scattered_balls_overlap_when_their_centers_are_near_enough() {
		// A ball has no corners, so its furthest point is a new one for every direction.
		let mut next = scattered();
		let mut overlapping = 0;
		for _ in 0..2000 {
			let (center_a, center_b) = (next(), next());
			let (radius_a, radius_b) = (next().x().abs() + 0.05, next().x().abs() + 0.05);
			let gap = (center_a - center_b).length() - radius_a - radius_b;

			let overlap = shapes_overlap(ball(center_a.to_array(), radius_a), ball(center_b.to_array(), radius_b));

			if gap.abs() > 1e-3 {
				assert_eq!(overlap, gap < 0.0, "balls {gap} apart");
			}
			overlapping += usize::from(overlap);
		}
		assert!((400..1600).contains(&overlapping), "{overlapping} of 2000 overlap");
	}

	#[test]
	fn a_ball_overlaps_a_hull_that_it_reaches() {
		let hull = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let overlap = |center: [f32; 3], radius: f32| shapes_overlap(|along| hull.support_point(along), ball(center, radius));

		// Beside a face, an edge, and a corner. The corner is the square root of three from the ball's center.
		assert!(overlap([2.0, 0.0, 0.0], 1.1));
		assert!(!overlap([2.0, 0.0, 0.0], 0.9));
		assert!(overlap([2.0, 2.0, 0.0], 1.5));
		assert!(!overlap([2.0, 2.0, 0.0], 1.3));
		assert!(overlap([2.0, 2.0, 2.0], 1.8));
		assert!(!overlap([2.0, 2.0, 2.0], 1.6));
	}

	#[test]
	fn shapes_at_one_point_overlap() {
		let point = |_| Point::<LocalSpace>::new(3.0, -2.0, 1.0);

		assert!(shapes_overlap(point, point));
		assert!(!shapes_overlap(point, |_| Point::new(3.0, -2.0, 1.5)));
		// Points within reach of each other count as touching, and points just past it do not.
		assert!(shapes_overlap(point, |_| Point::new(3.00005, -2.0, 1.0)));
		assert!(!shapes_overlap(point, |_| Point::new(3.0002, -2.0, 1.0)));
	}
	/// Asserts what every contact has to satisfy: its points are its depth apart along its normal, and each is as
	/// far toward the other shape as its own shape reaches.
	fn assert_contact(
		contact: &Intersection<LocalSpace>,
		support_a: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		support_b: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		margin: f32,
	) {
		let normal: Vector<LocalSpace> = contact.normal().into();
		let apart = contact.point_on_a() - contact.point_on_b();
		assert!(
			(apart - normal * contact.depth()).length() <= margin,
			"the points are {apart:?} apart: {contact:?}"
		);

		let reach_a = normal.dot(support_a(normal) - contact.point_on_a());
		let reach_b = normal.dot(contact.point_on_b() - support_b(-normal));
		assert!(
			reach_a.abs() <= margin,
			"shape A reaches {reach_a} past its point: {contact:?}"
		);
		assert!(
			reach_b.abs() <= margin,
			"shape B reaches {reach_b} past its point: {contact:?}"
		);
	}

	#[test]
	fn boxes_get_a_contact_along_their_shallowest_overlap() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let b = box_hull([1.0, 1.0, 1.0], [1.75, 0.5, -0.25]);
		let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

		let contact = shapes_contact(support_a, support_b).expect("the boxes overlap");

		assert!(
			(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.25).abs() <= 1e-6);
		assert_contact(&contact, support_a, support_b, 1e-5);
		// Each point is on the side of its own box that is inside the other, and within both sides.
		assert!((contact.point_on_a().x() - 1.0).abs() <= 1e-6);
		assert!((contact.point_on_b().x() - 0.75).abs() <= 1e-6);
		assert!((-0.5..=1.0).contains(&contact.point_on_a().y()) && (-1.0..=0.75).contains(&contact.point_on_a().z()));

		// The other way around, the same contact is seen from the other side.
		let swapped = shapes_contact(support_b, support_a).expect("the boxes overlap");
		assert!((swapped.normal().into_vector() - Vector::new(-1.0, 0.0, 0.0)).length() <= 1e-6);
		assert!((swapped.depth() - 0.25).abs() <= 1e-6);
		assert_contact(&swapped, support_b, support_a, 1e-5);
	}

	#[test]
	fn scattered_boxes_get_the_contact_of_their_bounds() {
		let mut next = scattered();
		let mut contacts = 0;
		for _ in 0..500 {
			let (middle_a, middle_b) = (next().to_array(), (next() * 2.0).to_array());
			let (half_a, half_b) = (
				next().to_array().map(|size| size.abs() + 0.1),
				next().to_array().map(|size| size.abs() + 0.1),
			);
			// How far the boxes overlap on each axis. The contact is along the axis where that is least.
			let overlaps: [f32; 3] =
				std::array::from_fn(|axis| half_a[axis] + half_b[axis] - (middle_a[axis] - middle_b[axis]).abs());
			let mut sorted = overlaps;
			sorted.sort_by(f32::total_cmp);
			let (a, b) = (box_hull(half_a, middle_a), box_hull(half_b, middle_b));
			let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

			let contact = shapes_contact(support_a, support_b);

			// Boxes that all but touch may be reported either way.
			if sorted[0].abs() <= 1e-3 {
				continue;
			}
			let Some(contact) = contact else {
				assert!(sorted[0] < 0.0, "boxes {} into each other have no contact", sorted[0]);
				continue;
			};
			contacts += 1;
			assert!(sorted[0] > 0.0, "boxes {} apart have a contact", -sorted[0]);
			assert!(
				(contact.depth() - sorted[0]).abs() <= 1e-5,
				"{contact:?} for an overlap of {}",
				sorted[0]
			);
			assert_contact(&contact, support_a, support_b, 1e-4);
			// When one axis is clearly the shallowest, the normal is along it, from the first box toward the second.
			if sorted[1] - sorted[0] > 1e-3 {
				let axis = (0..3)
					.find(|&axis| overlaps[axis] == sorted[0])
					.expect("an axis has the least overlap");
				let toward = (middle_b[axis] - middle_a[axis]).signum();
				assert!(
					(contact.normal().into_vector().to_array()[axis] - toward).abs() <= 1e-5,
					"{contact:?}"
				);
			}
		}
		assert!(contacts > 50, "{contacts} of 500 have a contact");
	}

	#[test]
	fn a_slanted_side_gets_a_slanted_contact() {
		// The turned cube's slanted side is 1 from its middle, and the box's vertical edge at (1, 1) is 0.2 inside it.
		let reach = 0.8 / std::f32::consts::SQRT_2;
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let b = turned_cube([1.0 + reach, 1.0 + reach, 0.0]);
		let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

		let contact = shapes_contact(support_a, support_b).expect("the edge is inside the cube");

		let slant = std::f32::consts::FRAC_1_SQRT_2;
		assert!(
			(contact.normal().into_vector() - Vector::new(slant, slant, 0.0)).length() <= 1e-5,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.2).abs() <= 1e-5, "{contact:?}");
		assert_contact(&contact, support_a, support_b, 1e-5);
		assert!((contact.point_on_a().x() - 1.0).abs() <= 1e-5 && (contact.point_on_a().y() - 1.0).abs() <= 1e-5);
	}

	#[test]
	fn a_shape_inside_another_gets_the_shortest_way_out() {
		// The small box is 0.25 from the large one's side at x = 2, and further from every other side.
		let a = box_hull([2.0, 3.0, 4.0], [0.0, 0.0, 0.0]);
		let b = box_hull([0.25, 0.25, 0.25], [1.5, 0.5, -1.0]);
		let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

		let contact = shapes_contact(support_a, support_b).expect("one box is inside the other");

		// The small box leaves through that side once all of it has passed, which is its width further.
		assert!(
			(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.75).abs() <= 1e-6, "{contact:?}");
		assert_contact(&contact, support_a, support_b, 1e-5);
	}

	#[test]
	fn scattered_balls_get_the_contact_between_their_centers() {
		let mut next = scattered();
		let mut contacts = 0;
		for _ in 0..500 {
			let (center_a, center_b) = (next(), next());
			let (radius_a, radius_b) = (next().x().abs() + 0.05, next().x().abs() + 0.05);
			let reach = radius_a + radius_b;
			let gap = (center_a - center_b).length() - reach;
			let (support_a, support_b) = (ball(center_a.to_array(), radius_a), ball(center_b.to_array(), radius_b));

			let contact = shapes_contact(&support_a, &support_b);

			if gap.abs() <= 1e-3 {
				continue;
			}
			let Some(contact) = contact else {
				assert!(gap > 0.0, "balls {} into each other have no contact", -gap);
				continue;
			};
			contacts += 1;
			assert!(gap < 0.0, "balls {gap} apart have a contact");
			// The flat faces that follow a ball's surface are a little inside it, so the depth can only come out
			// short, and the normal a little off the line between the centers.
			assert!(
				(-2e-4..=1e-5).contains(&(contact.depth() + gap)),
				"{contact:?} for balls {gap} apart"
			);
			let between = (center_b - center_a) / (center_b - center_a).length();
			if (center_b - center_a).length() > 0.05 {
				assert!(
					contact.normal().dot(between) >= 0.999,
					"{contact:?} between {center_a:?} and {center_b:?}"
				);
			}
			assert_contact(&contact, &support_a, &support_b, 1e-3 + 0.02 * reach);
		}
		assert!(contacts > 100, "{contacts} of 500 have a contact");
	}

	#[test]
	fn a_ball_gets_a_contact_with_the_side_of_a_hull() {
		let hull = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let (support_a, support_b) = (|along| hull.support_point(along), ball([1.75, 0.25, -0.5], 1.0));

		let contact = shapes_contact(support_a, &support_b).expect("the ball reaches into the box");

		assert!(contact.normal().dot(Vector::new(1.0, 0.0, 0.0)) >= 0.9999, "{contact:?}");
		assert!((contact.depth() - 0.25).abs() <= 2e-4, "{contact:?}");
		assert!(
			(contact.point_on_a() - Point::new(1.0, 0.25, -0.5)).length() <= 0.02,
			"{contact:?}"
		);
		assert!(
			(contact.point_on_b() - Point::new(0.75, 0.25, -0.5)).length() <= 0.02,
			"{contact:?}"
		);
	}

	#[test]
	fn shapes_apart_or_without_volume_have_no_contact() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let b = box_hull([1.0, 1.0, 1.0], [2.5, 0.0, 0.0]);
		assert!(shapes_contact(|along| a.support_point(along), |along| b.support_point(along)).is_none());
		assert!(shapes_contact(ball([0.0; 3], 1.0), ball([0.0, 3.0, 0.0], 1.5)).is_none());

		// Two points at one place overlap, and no direction leads out.
		let point = |_| Point::<LocalSpace>::new(3.0, -2.0, 1.0);
		assert!(shapes_contact(point, point).is_none());
	}

	#[test]
	fn a_flat_shape_gets_a_contact_with_a_solid_one() {
		// A square without thickness, halfway into the top of a box.
		let square = |along: Vector<LocalSpace>| Point::new(along.x().signum() * 0.5, 0.75, along.z().signum() * 0.5);
		let solid = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let support_a = |along| solid.support_point(along);

		let contact = shapes_contact(support_a, square).expect("the square is inside the box");

		assert!(
			(contact.normal().into_vector() - Vector::new(0.0, 1.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.25).abs() <= 1e-6, "{contact:?}");
		assert_contact(&contact, support_a, square, 1e-5);
	}
	/// Asserts that `closest` are the closest points of two shapes that are apart: a plane square to the line between
	/// them, through either point, has all of that point's shape on its far side.
	fn assert_closest_points(
		closest: (Point<LocalSpace>, Point<LocalSpace>),
		support_a: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		support_b: impl Fn(Vector<LocalSpace>) -> Point<LocalSpace>,
		margin: f32,
	) {
		let (on_a, on_b) = closest;
		let toward_b = (on_b - on_a) / (on_b - on_a).length();
		let past_a = toward_b.dot(support_a(toward_b) - on_a);
		let past_b = toward_b.dot(on_b - support_b(-toward_b));
		assert!(
			past_a.abs() <= margin,
			"shape A reaches {past_a} past {on_a:?} toward {on_b:?}"
		);
		assert!(
			past_b.abs() <= margin,
			"shape B reaches {past_b} past {on_b:?} toward {on_a:?}"
		);
	}

	#[test]
	fn boxes_apart_get_the_points_across_their_gap() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let b = box_hull([1.0, 1.0, 1.0], [2.5, 0.5, -0.25]);
		let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

		let (on_a, on_b) = shapes_closest_points(support_a, support_b).expect("the boxes are apart");

		// The points are straight across from each other, on the sides that face each other, where those overlap.
		assert!(
			(on_b - on_a - Vector::new(0.5, 0.0, 0.0)).length() <= 1e-6,
			"{on_a:?} and {on_b:?}"
		);
		assert!((on_a.x() - 1.0).abs() <= 1e-6);
		assert!((-0.5..=1.0).contains(&on_a.y()) && (-1.0..=0.75).contains(&on_a.z()));

		let (on_b, on_a) = shapes_closest_points(support_b, support_a).expect("the boxes are apart");
		assert!(
			(on_b - on_a - Vector::new(0.5, 0.0, 0.0)).length() <= 1e-6,
			"{on_a:?} and {on_b:?}"
		);
	}

	#[test]
	fn scattered_boxes_are_as_far_apart_as_their_bounds() {
		let mut next = scattered();
		let mut apart = 0;
		for _ in 0..500 {
			let (middle_a, middle_b) = (next().to_array(), (next() * 3.0).to_array());
			let (half_a, half_b) = (
				next().to_array().map(|size| size.abs() + 0.1),
				next().to_array().map(|size| size.abs() + 0.1),
			);
			// The gap on each axis, where there is one. Boxes are as far apart as those gaps taken together.
			let gaps: [f32; 3] =
				std::array::from_fn(|axis| (middle_a[axis] - middle_b[axis]).abs() - half_a[axis] - half_b[axis]);
			let distance = gaps.iter().map(|gap| gap.max(0.0).powi(2)).sum::<f32>().sqrt();
			let (a, b) = (box_hull(half_a, middle_a), box_hull(half_b, middle_b));
			let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));

			let closest = shapes_closest_points(support_a, support_b);

			// Boxes that all but touch may be reported either way.
			if gaps.iter().fold(f32::NEG_INFINITY, |most, &gap| most.max(gap)).abs() <= 1e-3 {
				continue;
			}
			let Some((on_a, on_b)) = closest else {
				assert!(distance == 0.0, "boxes {distance} apart have no closest points");
				continue;
			};
			apart += 1;
			assert!(
				((on_b - on_a).length() - distance).abs() <= 1e-5,
				"{on_a:?} and {on_b:?} for {distance}"
			);
			// Rounding moves each point a little, which tilts the line between them more the closer they are.
			assert_closest_points((on_a, on_b), support_a, support_b, 1e-5 + 2e-7 / distance);
		}
		assert!(apart > 100, "{apart} of 500 are apart");
	}

	#[test]
	fn scattered_balls_are_closest_between_their_centers() {
		let mut next = scattered();
		let mut apart = 0;
		for _ in 0..500 {
			let (center_a, center_b) = (Point::origin() + next(), Point::origin() + next() * 2.0);
			let (radius_a, radius_b) = (next().x().abs() + 0.05, next().x().abs() + 0.05);
			let between = center_b - center_a;
			let gap = between.length() - radius_a - radius_b;
			let (support_a, support_b) = (ball(center_a.to_array(), radius_a), ball(center_b.to_array(), radius_b));

			let closest = shapes_closest_points(&support_a, &support_b);

			if gap.abs() <= 1e-3 {
				continue;
			}
			let Some((on_a, on_b)) = closest else {
				assert!(gap < 0.0, "balls {gap} apart have no closest points");
				continue;
			};
			apart += 1;
			let toward_b = between / between.length();
			// On a round surface, a point a little to the side is almost as close, so the points are found less
			// exactly than the distance between them.
			assert!(
				(on_a - (center_a + toward_b * radius_a)).length() <= 5e-3,
				"{on_a:?} for {gap}"
			);
			assert!(
				(on_b - (center_b - toward_b * radius_b)).length() <= 5e-3,
				"{on_b:?} for {gap}"
			);
			assert!(
				((on_b - on_a).length() - gap).abs() <= 1e-5,
				"{on_a:?} and {on_b:?} for {gap}"
			);
		}
		assert!(apart > 100, "{apart} of 500 are apart");
	}

	#[test]
	fn a_ball_is_closest_to_the_nearest_part_of_a_hull() {
		let hull = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let support_a = |along| hull.support_point(along);

		// Beside a side, an edge, and a corner of the box.
		for (center, on_hull) in [
			([3.0, 0.25, -0.5], [1.0, 0.25, -0.5]),
			([3.0, 2.5, 0.5], [1.0, 1.0, 0.5]),
			([2.0, 3.0, -2.5], [1.0, 1.0, -1.0]),
		] {
			let (center, on_hull) = (Point::from_array(center), Point::from_array(on_hull));
			let toward_hull = (on_hull - center) / (on_hull - center).length();

			let (on_a, on_b) = shapes_closest_points(support_a, ball(center.to_array(), 0.5)).expect("the ball is outside");

			assert!((on_a - on_hull).length() <= 1e-3, "{on_a:?} is not {on_hull:?}");
			assert!((on_b - (center + toward_hull * 0.5)).length() <= 1e-3, "{on_b:?}");
		}
	}

	#[test]
	fn shapes_that_overlap_have_no_closest_points() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let support_a = |along| a.support_point(along);

		let b = box_hull([1.0, 1.0, 1.0], [1.5, 0.5, -0.5]);
		assert!(shapes_closest_points(support_a, |along| b.support_point(along)).is_none());
		let inside = box_hull([0.25, 0.25, 0.25], [0.5, -0.5, 0.25]);
		assert!(shapes_closest_points(support_a, |along| inside.support_point(along)).is_none());
		assert!(shapes_closest_points(support_a, ball([1.5, 0.0, 0.0], 1.0)).is_none());
		assert!(shapes_closest_points(support_a, support_a).is_none());
	}
	#[test]
	fn rounded_points_get_the_contact_of_two_balls() {
		let mut next = scattered();
		let mut contacts = 0;
		for _ in 0..500 {
			let (center_a, center_b) = (Point::<LocalSpace>::origin() + next(), Point::origin() + next());
			let (radius_a, radius_b) = (next().x().abs() + 0.05, next().x().abs() + 0.05);
			let expected = math::collision::sphere_vs_sphere(
				&math::Sphere::new(center_a, radius_a),
				&math::Sphere::new(center_b, radius_b),
			);

			let contact = rounded_shapes_contact(|_| center_a, radius_a, |_| center_b, radius_b);

			// Balls that all but touch, or that share a center, may be reported either way.
			let gap = (center_b - center_a).length() - radius_a - radius_b;
			if gap.abs() <= 1e-3 || (center_b - center_a).length() <= 1e-3 {
				continue;
			}
			assert_eq!(contact.is_some(), expected.is_some(), "balls {gap} apart");
			let (Some(contact), Some(expected)) = (contact, expected) else {
				continue;
			};
			contacts += 1;
			assert!((contact.normal().into_vector() - expected.normal().into_vector()).length() <= 1e-5);
			assert!((contact.depth() - expected.depth()).abs() <= 1e-5);
			assert!((contact.point_on_a() - expected.point_on_a()).length() <= 1e-5);
			assert!((contact.point_on_b() - expected.point_on_b()).length() <= 1e-5);
		}
		assert!(contacts > 100, "{contacts} of 500 have a contact");
	}

	#[test]
	fn a_rounded_point_gets_an_exact_contact_with_a_hull() {
		let hull = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let support_a = |along| hull.support_point(along);
		let contact_with = |center: [f32; 3], radius: f32| {
			let center = Point::<LocalSpace>::from_array(center);
			rounded_shapes_contact(support_a, 0.0, move |_| center, radius)
		};

		// Beside a side: the ball's center is 0.75 from it.
		let contact = contact_with([1.75, 0.25, -0.5], 1.0).expect("the ball reaches into the box");
		assert!(
			(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.25).abs() <= 1e-6);
		assert!((contact.point_on_a() - Point::new(1.0, 0.25, -0.5)).length() <= 1e-6);
		assert!((contact.point_on_b() - Point::new(0.75, 0.25, -0.5)).length() <= 1e-6);
		assert!(contact_with([1.75, 0.25, -0.5], 0.7).is_none());

		// Beside a corner, which is the square root of three from the ball's center.
		let contact = contact_with([2.0, 2.0, 2.0], 1.8).expect("the ball reaches the corner");
		let slant = 1.0 / 3f32.sqrt();
		assert!(
			(contact.normal().into_vector() - Vector::new(slant, slant, slant)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - (1.8 - 3f32.sqrt())).abs() <= 1e-6);
		assert!((contact.point_on_a() - Point::new(1.0, 1.0, 1.0)).length() <= 1e-6);
		assert!(contact_with([2.0, 2.0, 2.0], 1.7).is_none());

		// With its center inside the box, the ball leaves through the nearest side, by its radius more than its
		// center does.
		let contact = contact_with([0.5, 0.0, 0.0], 0.25).expect("the ball is inside the box");
		assert!(
			(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.75).abs() <= 1e-6);
		assert!((contact.point_on_a() - Point::new(1.0, 0.0, 0.0)).length() <= 1e-6);
		assert!((contact.point_on_b() - Point::new(0.25, 0.0, 0.0)).length() <= 1e-6);
	}

	#[test]
	fn a_rounded_segment_gets_the_contact_of_a_capsule() {
		// An upright capsule beside a box: its segment runs along the box's side, 0.2 from it.
		let hull = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let segment = |along: Vector<LocalSpace>| Point::new(1.2, 0.6f32.copysign(along.y()), 0.25);

		let contact =
			rounded_shapes_contact(|along| hull.support_point(along), 0.0, segment, 0.3).expect("the capsule reaches");

		assert!(
			(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-6,
			"{contact:?}"
		);
		assert!((contact.depth() - 0.1).abs() <= 1e-6);
		// The points are across from each other, somewhere along the stretch where the capsule runs beside the box.
		assert!((contact.point_on_a() - contact.point_on_b() - Vector::new(0.1, 0.0, 0.0)).length() <= 1e-6);
		assert!((contact.point_on_a().x() - 1.0).abs() <= 1e-6 && contact.point_on_a().y().abs() <= 0.6);
	}
	#[test]
	fn working_storage_that_was_used_finds_the_same_contacts_as_new_storage() {
		// Pairs of every kind in turn, so that each contact is found with what the one before left behind: boxes,
		// which take a handful of points, and balls deep in each other, which take many.
		let mut next = scattered();
		let mut used = ContactScratch::default();
		let mut contacts = 0;
		for pair in 0..300 {
			let (middle_a, middle_b) = (next().to_array(), next().to_array());
			let (size_a, size_b) = (
				next().to_array().map(|size| size.abs() + 0.1),
				next().to_array().map(|size| size.abs() + 0.1),
			);
			let (with_used, with_new) = if pair % 2 == 0 {
				let (a, b) = (box_hull(size_a, middle_a), box_hull(size_b, middle_b));
				let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));
				(
					super::shapes_contact(support_a, support_b, &mut used),
					shapes_contact(support_a, support_b),
				)
			} else {
				let (support_a, support_b) = (ball(middle_a, size_a[0]), ball(middle_b, size_b[0]));
				(
					super::shapes_contact(&support_a, &support_b, &mut used),
					shapes_contact(&support_a, &support_b),
				)
			};

			assert_eq!(with_used, with_new, "pair {pair}");
			contacts += usize::from(with_used.is_some());
		}
		assert!(contacts > 50, "{contacts} of 300 have a contact");
	}

	#[test]
	fn a_contact_is_the_same_whichever_direction_the_search_starts_along() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let support_a = |along| a.support_point(along);
		let starts = [
			None,
			// Toward the other shape, away from it, across, and nowhere.
			Some(Vector::new(1.0, 0.0, 0.0)),
			Some(Vector::new(-3.0, 0.0, 0.0)),
			Some(Vector::new(0.0, 0.0, 2.0)),
			Some(Vector::new(0.4, -0.7, 0.2)),
			Some(Vector::zero()),
		];

		// Each is a box at this place along x, each shape made thicker by a tenth, and how deep they then are in
		// each other: with their cores overlapping, with only the thicker shapes overlapping, and apart.
		for (place, depth) in [(1.5, Some(0.7)), (2.1, Some(0.1)), (2.5, None)] {
			let b = box_hull([1.0, 1.0, 1.0], [place, 0.3, -0.2]);
			let support_b = |along| b.support_point(along);

			for start in starts {
				let contact = super::rounded_shapes_contact(
					support_a,
					0.1,
					support_b,
					0.1,
					start,
					None,
					0.0,
					&mut ContactScratch::default(),
				);

				assert_eq!(contact.is_some(), depth.is_some(), "at {place} from {start:?}");
				if let (Some(contact), Some(depth)) = (contact, depth) {
					let normal: Vector<LocalSpace> = contact.normal().into();
					assert!(
						(normal - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-5,
						"at {place} from {start:?}"
					);
					assert!(
						(contact.depth() - depth).abs() <= 1e-5,
						"at {place} from {start:?}: {contact:?}"
					);
				}
			}
		}
	}

	#[test]
	fn shapes_that_are_apart_get_a_contact_with_their_gap_when_it_is_within_reach() {
		let a = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		// The boxes face each other across half a unit, and each is made thicker by a tenth, which leaves a gap of
		// three tenths.
		let b = box_hull([1.0, 1.0, 1.0], [2.5, 0.3, -0.2]);
		let (support_a, support_b) = (|along| a.support_point(along), |along| b.support_point(along));
		let contact = |within: f32| {
			super::rounded_shapes_contact(
				support_a,
				0.1,
				support_b,
				0.1,
				None,
				None,
				within,
				&mut ContactScratch::default(),
			)
		};

		assert!(contact(0.0).is_none());
		assert!(contact(0.2).is_none());
		for within in [0.4, f32::INFINITY] {
			let contact = contact(within).expect("the gap is within reach");
			let normal: Vector<LocalSpace> = contact.normal().into();

			assert!((normal - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-5, "within {within}");
			assert!((contact.depth() + 0.3).abs() <= 1e-5, "within {within}: {contact:?}");
			// The points are on the sides that face each other, straight across the gap.
			assert!((contact.point_on_a().x() - 1.1).abs() <= 1e-5, "within {within}: {contact:?}");
			assert!(
				(contact.point_on_b() - contact.point_on_a() - normal * 0.3).length() <= 1e-5,
				"within {within}: {contact:?}"
			);
		}
	}

	#[test]
	fn a_contact_is_on_the_surface_when_another_plane_is_as_near() {
		// The tip of a diamond at (1, 0, 0) is 0.25 inside two sides of a box at once: the one it came in through,
		// which faces -x, and the one that faces -z. Only the first way out is that short. Past the second, the
		// diamond widens.
		let tips = [
			[1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, -1.0, 0.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, -1.0],
		]
		.map(Point::<LocalSpace>::from_array);
		let diamond = build_convex_hull(&tips).expect("the tips enclose a volume");
		let cube = box_hull([0.5, 0.5, 0.5], [1.25, -0.25, 0.25]);
		let (support_a, support_b) = (|along| diamond.support_point(along), |along| cube.support_point(along));

		// With the cores asked for their extent, and with a bound on it given.
		for extent in [None, Some(4.0), Some(100.0)] {
			let contact = super::rounded_shapes_contact(
				support_a,
				0.0,
				support_b,
				0.0,
				None,
				extent,
				0.0,
				&mut ContactScratch::default(),
			)
			.expect("the tip is inside the box");

			assert!(
				(contact.normal().into_vector() - Vector::new(1.0, 0.0, 0.0)).length() <= 1e-5,
				"with {extent:?}: {contact:?}"
			);
			assert!((contact.depth() - 0.25).abs() <= 1e-5);
			assert_contact(&contact, support_a, support_b, 1e-5);
		}
	}

	/// Returns a ray from `origin` toward `toward`, which need not have a length of one.
	fn ray(origin: [f32; 3], toward: [f32; 3]) -> Ray<LocalSpace> {
		let toward = Vector::new(toward[0], toward[1], toward[2]);
		Ray::new(
			Point::from_array(origin),
			toward.normalized().expect("the ray has a direction"),
		)
	}

	/// Returns the furthest corner of the box from `min` to `max` along a direction.
	fn aligned_box(min: [f32; 3], max: [f32; 3]) -> impl Fn(Vector<LocalSpace>) -> Point<LocalSpace> + Copy {
		move |direction| {
			let along = direction.to_array();
			Point::from_array([0, 1, 2].map(|axis| if along[axis] >= 0.0 { max[axis] } else { min[axis] }))
		}
	}

	/// Asserts that `hit` is at `distance` with `normal`.
	fn assert_hit(hit: Option<(f32, UnitVector<LocalSpace>)>, distance: f32, normal: [f32; 3]) {
		let (found, found_normal) = hit.expect("the ray meets the shape");
		assert!((found - distance).abs() <= 1e-4, "{found} instead of {distance}");
		let normal = Vector::new(normal[0], normal[1], normal[2])
			.normalized()
			.expect("a normal has a direction");
		assert!(
			(found_normal.into_vector() - normal.into_vector()).length() <= 1e-4,
			"{found_normal:?} instead of {normal:?}"
		);
	}

	#[test]
	fn a_ray_meets_a_box_at_the_side_it_enters() {
		let cube = box_hull([1.0, 1.0, 1.0], [0.0, 0.0, 0.0]);
		let cast = |ray: Ray<LocalSpace>| rounded_shape_ray_cast(|along| cube.support_point(along), 0.0, &ray, f32::INFINITY);

		assert_hit(cast(ray([5.0, 0.3, -0.2], [-1.0, 0.0, 0.0])), 4.0, [1.0, 0.0, 0.0]);
		assert_hit(cast(ray([0.4, -3.0, 0.9], [0.0, 1.0, 0.0])), 2.0, [0.0, -1.0, 0.0]);
		// Slanted down onto the top, where it is 3 units lower and half a unit further along z.
		assert_hit(cast(ray([0.0, 3.0, 0.0], [0.0, -2.0, 0.5])), 4.25f32.sqrt(), [0.0, 1.0, 0.0]);

		// A cube turned an eighth of a turn about z has a slanted side where x + y is the square root of two.
		let turned = turned_cube([0.0, 0.0, 0.0]);
		let hit = rounded_shape_ray_cast(
			|along| turned.support_point(along),
			0.0,
			&ray([5.0, 0.5, 0.0], [-1.0, 0.0, 0.0]),
			f32::INFINITY,
		);
		assert_hit(hit, 5.0 - (std::f32::consts::SQRT_2 - 0.5), [1.0, 1.0, 0.0]);
	}

	#[test]
	fn a_ray_meets_a_capsule_on_its_side_and_on_its_caps() {
		// An upright capsule whose segment reaches 0.6 up and down from the origin, 0.3 thick.
		let segment = |along: Vector<LocalSpace>| Point::new(0.0, 0.6f32.copysign(along.y()), 0.0);
		let cast = |ray: Ray<LocalSpace>| rounded_shape_ray_cast(segment, 0.3, &ray, f32::INFINITY);

		assert_hit(cast(ray([5.0, 0.2, 0.0], [-1.0, 0.0, 0.0])), 4.7, [1.0, 0.0, 0.0]);
		assert_hit(cast(ray([0.0, 5.0, 0.0], [0.0, -1.0, 0.0])), 4.1, [0.0, 1.0, 0.0]);
		// Down onto the upper cap, a tenth of a unit off its middle.
		let height = 0.08f32.sqrt();
		assert_hit(cast(ray([0.1, 5.0, 0.0], [0.0, -1.0, 0.0])), 4.4 - height, [0.1, height, 0.0]);
	}

	#[test]
	fn a_ray_that_passes_leads_away_from_or_ends_short_of_a_shape_misses_it() {
		let cube = aligned_box([-1.0; 3], [1.0; 3]);
		let ball = |_| Point::<LocalSpace>::origin();

		for (support, radius) in [(&cube as &dyn Fn(_) -> _, 0.0), (&ball, 1.0)] {
			let cast = |ray: Ray<LocalSpace>, max_distance| rounded_shape_ray_cast(support, radius, &ray, max_distance);
			assert!(cast(ray([5.0, 1.01, 0.0], [-1.0, 0.0, 0.0]), f32::INFINITY).is_none());
			assert!(cast(ray([5.0, 0.0, 0.0], [1.0, 0.0, 0.0]), f32::INFINITY).is_none());
			assert!(cast(ray([5.0, 0.0, 0.0], [-1.0, 0.0, 0.0]), 3.9).is_none());
			assert_hit(cast(ray([5.0, 0.0, 0.0], [-1.0, 0.0, 0.0]), 4.0), 4.0, [1.0, 0.0, 0.0]);
		}
	}

	#[test]
	fn a_ray_that_starts_in_or_on_a_shape_meets_it_at_once() {
		let cube = aligned_box([-1.0; 3], [1.0; 3]);
		let ball = |_| Point::<LocalSpace>::origin();

		for (support, radius) in [(&cube as &dyn Fn(_) -> _, 0.0), (&ball, 1.0)] {
			let cast = |ray: Ray<LocalSpace>| rounded_shape_ray_cast(support, radius, &ray, 0.5);
			assert_hit(cast(ray([0.5, 0.0, 0.0], [1.0, 0.0, 0.0])), 0.0, [-1.0, 0.0, 0.0]);
			assert_hit(cast(ray([0.0, 0.0, 0.0], [0.0, 1.0, 1.0])), 0.0, [0.0, -1.0, -1.0]);
			// On the surface, leading out.
			assert_hit(cast(ray([1.0, 0.0, 0.0], [1.0, 0.0, 0.0])), 0.0, [-1.0, 0.0, 0.0]);
		}
	}

	#[test]
	fn scattered_rays_and_segments_meet_boxes_where_they_enter_their_slabs() {
		let mut next = scattered();
		let mut hits = 0;
		for _ in 0..2000 {
			let (middle, half_sizes) = (next().to_array(), next().to_array().map(|size| size.abs() + 0.1));
			let grown = |by: f32| {
				AABB::new(
					Point::from_array(middle) - Vector::from_array(half_sizes.map(|size| size + by)),
					Point::from_array(middle) + Vector::from_array(half_sizes.map(|size| size + by)),
				)
			};
			let (min, max) = (grown(0.0).min().to_array(), grown(0.0).max().to_array());
			// Each ray leads toward somewhere near the box, so that many meet it.
			let origin = Point::from_array(middle) + next() * 4.0;
			let Ok(direction) = (Point::from_array(middle) + next() * 1.5 - origin).normalized() else {
				continue;
			};
			let ray = Ray::new(origin, direction);
			let length = next().x().abs() * 6.0;

			// The reference: where the ray enters the box, and whether a ray of that length gets there. Rays that
			// only graze the box, or end at it, may be reported either way.
			let entry = |box_: &AABB<LocalSpace>, length: f32| {
				math::collision::ray_aabb_intersection(&ray, box_).filter(|entry| *entry <= length)
			};
			if entry(&grown(-1e-3), length - 1e-3).is_some() != entry(&grown(1e-3), length + 1e-3).is_some() {
				continue;
			}
			let expected = entry(&grown(0.0), length);

			let hit = rounded_shape_ray_cast(aligned_box(min, max), 0.0, &ray, length);
			assert_eq!(
				hit.is_some(),
				expected.is_some(),
				"{ray:?} of {length} and a box from {min:?} to {max:?}"
			);
			let (Some((distance, normal)), Some(expected)) = (hit, expected) else {
				continue;
			};
			hits += 1;
			// The ray stops within a ten-thousandth of a unit of where it enters the box.
			assert!((distance - expected).abs() <= 1e-4, "{distance} instead of {expected}");
			if distance == 0.0 {
				continue;
			}
			// The normal faces the ray, and the box is behind the plane through where the ray meets it. That plane is
			// the side the ray meets, or one of those at an edge or a corner.
			assert!(normal.dot(direction.into_vector()) < 0.0, "{normal:?} along {direction:?}");
			let at = ray.point_at(distance);
			let ahead = normal.dot(aligned_box(min, max)(normal.into_vector()) - at);
			assert!(ahead.abs() <= 1e-4, "{normal:?} at {at:?} has the box {ahead} ahead");
		}
		assert!(hits > 200, "{hits} of 2000 meet their box");
	}

	#[test]
	fn scattered_rays_meet_balls_where_their_lines_cross_the_ball() {
		let mut next = scattered();
		let mut hits = 0;
		for _ in 0..2000 {
			let (center, radius) = (Point::<LocalSpace>::origin() + next(), next().x().abs() + 0.05);
			// Each ray leads toward somewhere near the ball, so that many meet it.
			let origin = center + next() * 4.0;
			let Ok(direction) = (center + next() * 1.5 * radius - origin).normalized() else {
				continue;
			};
			let ray = Ray::new(origin, direction);

			// Rays that only graze the ball may be reported either way.
			let offset = center - ray.origin();
			let across = (offset - direction * direction.dot(offset)).length();
			if (across - radius).abs() <= 1e-3 {
				continue;
			}
			// The reference, worked out in double precision: where the ray's line enters and leaves the ball.
			let (offset, along) = ((center - ray.origin()).to_array(), direction.into_vector().to_array());
			let offset = offset.map(f64::from);
			let projection: f64 = (0..3).map(|axis| offset[axis] * f64::from(along[axis])).sum();
			let across_squared = offset.iter().map(|part| part * part).sum::<f64>() - projection * projection;
			let half_chord_squared = f64::from(radius) * f64::from(radius) - across_squared;
			let expected = (half_chord_squared >= 0.0)
				.then(|| (projection - half_chord_squared.sqrt(), projection + half_chord_squared.sqrt()))
				.filter(|(_, exit)| *exit >= 0.0)
				.map(|(entry, _)| entry.max(0.0) as f32);

			let hit = rounded_shape_ray_cast(|_| center, radius, &ray, f32::INFINITY);
			assert_eq!(
				hit.is_some(),
				expected.is_some(),
				"{ray:?} and a ball of {radius} at {center:?}"
			);
			let (Some((distance, normal)), Some(expected)) = (hit, expected) else {
				continue;
			};
			hits += 1;
			// The ray stops about a ten-thousandth of a unit short of where it enters the ball at most, and never
			// inside it. How short is judged from how directly the ray heads in from where it is, which is a little
			// more directly than where it enters.
			assert!((distance - expected).abs() <= 2e-4, "{distance} instead of {expected}");
			let gap = (ray.point_at(distance) - center).length() - radius;
			assert!(gap <= 1e-4 + 1e-5 && (distance == 0.0 || gap >= -1e-5), "{gap} from the ball");
			if distance > 0.0 {
				let outward = (ray.point_at(distance) - center)
					.normalized()
					.expect("the ray stops outside the center");
				let outward = outward.into_vector();
				assert!(
					(normal.into_vector() - outward).length() <= 1e-3,
					"{normal:?} instead of {outward:?}"
				);
			}
		}
		assert!(hits > 300, "{hits} of 2000 meet their ball");
	}
}
