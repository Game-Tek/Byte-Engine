//! The expanding polytope algorithm (EPA), which finds how deep two overlapping convex shapes are in each other.
//!
//! GJK can tell that the Minkowski difference of two shapes holds the origin, but not how far inside it the origin
//! is. EPA grows a solid inside the difference toward the part of the difference's surface that is nearest the
//! origin. The offset from the origin to that part is the shortest move that takes the shapes out of each other.

use math::{Point, Vector, WorldSpace, collision::Intersection};

use super::{
	convex_hull::{Face, GRID_BITS, GridPoint, HullFace, add_corner, cross, distance_squared, dot, offset},
	gjk::{MinkowskiPoint, nearest_on_triangle},
};

/// How near the surface of the Minkowski difference the solid has to get, in units, to count as having reached it.
const SURFACE_REACH: f32 = 0.0001;

/// How many points the solid is grown by at most. Shapes with flat sides need a handful, and so do round shapes that
/// overlap a little. Round shapes that are deep in each other have a surface that is about as near the origin all
/// around, which takes many flat faces to follow: of balls up to two units across, the deepest took 170. A pair that
/// is cut off here gets the contact of the nearest face so far.
const GROWTH_LIMIT: usize = 256;

/// The `ContactScratch` struct keeps the working storage that finding how deep two shapes are in each other needs,
/// so that only the first few contacts found with it allocate.
///
/// Keep one for every thread that finds contacts, and pass it to [`crate::physics::shapes_contact`] or
/// [`crate::physics::rounded_shapes_contact`] each time. It carries nothing over from one contact to the next but
/// its storage.
pub struct ContactScratch<Space = WorldSpace> {
	/// The points of the Minkowski difference found so far.
	points: Vec<Sample<Space>>,
	/// The faces of the solid grown on them.
	faces: Vec<SurfaceFace>,
	/// Working space for growing the solid.
	rim: Vec<[u32; 2]>,
}

// `derive` would only make a scratch for a space that has a default itself, and spaces are bare markers.
impl<Space> Default for ContactScratch<Space> {
	fn default() -> Self {
		Self {
			points: Vec::new(),
			faces: Vec::new(),
			rim: Vec::new(),
		}
	}
}

/// The `Sample` struct keeps a point of the Minkowski difference with where it is on the grid, which is all that is
/// asked about a point while the solid grows.
struct Sample<Space> {
	point: MinkowskiPoint<Space>,
	position: GridPoint,
}

/// The `SurfaceFace` struct keeps a face of the solid with how high it is above the origin, which is asked of every
/// face each time the solid grows and does not change while the face lasts.
#[derive(Clone, Copy)]
struct SurfaceFace {
	face: Face,
	/// How many grid steps the origin is outside the face's plane. Negative for a face that the origin is inside
	/// of, and the nearer zero, the nearer the face is to the origin.
	height: f64,
}

impl HullFace for SurfaceFace {
	fn face(&self) -> &Face {
		&self.face
	}

	fn from_face(face: Face) -> Self {
		Self {
			height: face.height([0; 3]),
			face,
		}
	}
}

/// Returns the contact between two shapes that overlap, from `support`, which returns the point of their Minkowski
/// difference that is furthest along a direction, and from `known`, which holds points of the difference that were
/// found before, such as the ones GJK ended with. The solid starts out around those, so the nearer they are to the
/// origin's way out, the less there is left to find.
///
/// Returns `None` when the difference has no volume, because the origin then has no shortest way out of it.
pub(super) fn penetration<Space>(
	support: impl Fn(Vector<Space>) -> MinkowskiPoint<Space>,
	known: &[MinkowskiPoint<Space>],
	scratch: &mut ContactScratch<Space>,
) -> Option<Intersection<Space>> {
	let ContactScratch { points, faces, rim } = scratch;
	points.clear();
	faces.clear();

	// The solid is a convex hull, grown like any other: its points are snapped to a grid of whole coordinates, so
	// that which faces a new point replaces is decided without rounding and the solid's surface stays closed. The grid
	// has to cover every point that `support` can return, so it is sized by the furthest of them along each axis.
	let along_axes = [
		[1.0, 0.0, 0.0],
		[-1.0, 0.0, 0.0],
		[0.0, 1.0, 0.0],
		[0.0, -1.0, 0.0],
		[0.0, 0.0, 1.0],
		[0.0, 0.0, -1.0],
	]
	.map(|axis| support(Vector::from_array(axis)));
	let reach = along_axes
		.iter()
		.chain(known)
		.flat_map(|point| point.difference().to_array())
		.fold(0.0, |reach: f32, coordinate| reach.max(coordinate.abs()));
	if reach <= 0.0 || !reach.is_finite() {
		return None;
	}

	// The origin is a point of the grid, so that distances from it are exact too.
	let limit = f64::from(1u32 << (GRID_BITS - 1));
	let step = f64::from(reach) / limit;
	let sample = |point: MinkowskiPoint<Space>| Sample {
		position: point
			.difference()
			.to_array()
			.map(|coordinate| (f64::from(coordinate) / step).round().clamp(-limit, limit) as i32),
		point,
	};
	points.extend(along_axes.iter().chain(known).map(|&point| sample(point)));
	// Snapping moves a point by up to a step, so nearer than two steps cannot be told from on the surface.
	let tolerance = (f64::from(SURFACE_REACH) / step).max(2.0);

	// The solid starts as a tetrahedron on four of the points so far. When those are all in one line or one plane,
	// more are asked for in the directions that lead out of it. A difference with volume has points there.
	let (a, b) = [(0, 1), (2, 3), (4, 5)]
		.into_iter()
		.max_by_key(|&(a, b)| distance_squared(points[a].position, points[b].position))?;
	if points[a].position == points[b].position {
		return None;
	}

	let (start, along) = (points[a].position, offset(points[a].position, points[b].position));
	let off_line = |point: GridPoint| {
		let lean = cross(along, offset(start, point));
		dot(lean, lean)
	};
	let c = match furthest(points, off_line) {
		Some(c) => c,
		None => {
			// Two directions square to the line and to each other. The first comes from the axis that the line
			// leans on least.
			let along = points[b].point.difference() - points[a].point.difference();
			let parts = along.to_array().map(f32::abs);
			let least = (0..3).min_by(|&left: &usize, &right: &usize| parts[left].total_cmp(&parts[right]))?;
			let mut axis = [0.0; 3];
			axis[least] = 1.0;
			let first = along.cross(Vector::from_array(axis));
			let second = along.cross(first);
			points.extend([first, -first, second, -second].map(|direction| sample(support(direction))));
			furthest(points, off_line)?
		}
	};

	let corners = [a, b, c];
	let base = Face::new(
		corners.map(|corner| corner as u32),
		corners.map(|corner| points[corner].position),
	);
	let off_plane = |point: GridPoint| base.side(point).abs();
	let d = match furthest(points, off_plane) {
		Some(d) => d,
		None => {
			let out = Vector::from_array(base.outward());
			points.extend([out, -out].map(|direction| sample(support(direction))));
			furthest(points, off_plane)?
		}
	};

	// The base has to face away from the fourth corner for the faces below to face outward.
	let (a, b) = if base.side(points[d].position) > 0 { (b, a) } else { (a, b) };
	faces.extend([[a, b, c], [a, c, d], [c, b, d], [b, a, d]].map(|corners| {
		SurfaceFace::from_face(Face::new(
			corners.map(|corner| corner as u32),
			corners.map(|corner| points[corner].position),
		))
	}));

	// Every other point so far is a point of the difference too, so the solid is grown to hold them before any more
	// are asked for. Growing it to a point that it already holds changes nothing.
	for point in 0..points.len() as u32 {
		add_corner(faces, |point| points[point as usize].position, point, rim, |_| {});
	}

	// The face whose plane is nearest the origin is where the solid's surface is nearest it. The difference reaches at
	// least as far that way, so the solid is grown to the difference's furthest point in the direction of that face,
	// until that point is no further out than the face is. The face is then on the difference's surface, and no part
	// of that surface is nearer the origin.
	//
	// The origin can start outside the solid, when the points so far were not around it. The faces that it is outside
	// of are the highest above it, so they are the ones replaced until the solid holds it.
	for _ in 0..GROWTH_LIMIT {
		let nearest = faces.iter().max_by(|left, right| left.height.total_cmp(&right.height))?.face;
		let found = sample(support(Vector::from_array(nearest.outward())));
		if nearest.height(found.position) <= tolerance {
			break;
		}

		points.push(found);
		add_corner(
			faces,
			|point| points[point as usize].position,
			(points.len() - 1) as u32,
			rim,
			|_| {},
		);
	}

	// A flat side of the difference can be covered by several faces, whose planes are all as near the origin. The
	// nearest point of the surface is on the one that the origin is straight below, which is the one with the point
	// nearest the origin.
	let highest = faces.iter().map(|face| face.height).fold(f64::NEG_INFINITY, f64::max);
	let (face, corners, nearest) = faces
		.iter()
		.filter(|face| face.height >= highest - tolerance)
		.map(|face| {
			let corners = face.face.corners.map(|corner| points[corner as usize].point);
			let nearest = nearest_on_triangle(corners[0].difference(), corners[1].difference(), corners[2].difference());
			(face.face, corners, nearest)
		})
		.min_by(|left, right| left.2.distance_squared.total_cmp(&right.2.distance_squared))?;

	// A point of the difference is a point of shape A minus one of shape B, so the nearest point's share of each
	// corner, taken of the points on each shape that the corners came from, gives the two points that are that far
	// apart. The face looks out of the difference, which is from shape A toward shape B.
	let normal = Vector::from_array(face.outward()).normalized().ok()?;
	let on_shape = |point_on: fn(&MinkowskiPoint<Space>) -> Point<Space>| {
		(0..3).fold(Point::origin(), |sum, corner| {
			sum + (point_on(&corners[corner]) - Point::origin()) * nearest.weights[corner]
		})
	};
	Some(Intersection::new(
		normal,
		// Shapes that only touch can leave the origin just outside the solid.
		normal.dot(nearest.point).max(0.0),
		on_shape(MinkowskiPoint::point_on_a),
		on_shape(MinkowskiPoint::point_on_b),
	))
}

/// Returns the point that `measure` rates highest, when it rates any above zero.
fn furthest<Space>(points: &[Sample<Space>], measure: impl Fn(GridPoint) -> i128) -> Option<usize> {
	let (most, point) = points
		.iter()
		.enumerate()
		.map(|(point, sample)| (measure(sample.position), point))
		.max()?;
	(most > 0).then_some(point)
}
