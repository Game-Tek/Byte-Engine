//! Convex hulls of point clouds, such as the vertices of a mesh that should collide as one simple solid.

use std::simd::{
	Select as _,
	cmp::{SimdPartialEq as _, SimdPartialOrd as _},
	f32x4,
	num::{SimdFloat as _, SimdUint as _},
	u32x4,
};

use math::{AABB, Point, Vector, is_finite};

use crate::physics::LocalSpace;

/// The `ConvexHull` struct describes the smallest convex solid around a set of local points, so that a detailed mesh
/// can be treated as one simple shape.
///
/// Build one from a mesh's vertex positions with [`build_convex_hull`], and give it to a body with
/// [`crate::physics::Shapes::convex_hull`]. A body collides as the hull with its corners and edges rounded by the
/// hull's [`ConvexHull::margin`].
#[derive(Debug, Clone, PartialEq)]
pub struct ConvexHull {
	corners: Corners,
	triangles: Vec<[u32; 3]>,
	bounds: AABB<LocalSpace>,
	center_of_mass: Point<LocalSpace>,
	inertia_tensor: [[f32; 3]; 3],
	/// The corners of the hull with every side moved in by `margin`.
	core: Corners,
	margin: f32,
}

/// How far a hull's corners and edges are rounded for collision, in meters, unless the hull is too small for that.
///
/// Bodies at rest are a few millimeters into each other, and gravity brings them a few more closer in a step. With
/// this much margin on each hull, the cores of two resting hulls stay apart.
const MARGIN: f32 = 0.02;

/// The largest share of the distance from a hull's center of mass to its nearest side that its margin may be, so
/// that a small or a thin hull keeps most of its shape.
const MARGIN_SHARE: f64 = 0.25;

impl ConvexHull {
	/// Returns the corners of the hull. Each is one of the points the hull was built from, and none lies inside the
	/// hull or within a face or an edge of it.
	pub fn vertices(&self) -> &[Point<LocalSpace>] {
		&self.corners.points
	}

	/// Returns the triangles that cover the hull's surface, as indices into [`ConvexHull::vertices`].
	///
	/// The triangles form a closed surface, and for a triangle `[a, b, c]` the vector `(b - a).cross(c - a)` points
	/// out of the hull. A flat side with more than three corners is covered by several triangles.
	pub fn triangles(&self) -> &[[u32; 3]] {
		&self.triangles
	}

	/// Returns the axis-aligned box around the hull.
	pub fn bounds(&self) -> AABB<LocalSpace> {
		self.bounds
	}

	/// Returns the corner of the hull that is furthest along `direction`, which no point of the hull is further along
	/// than. Of corners that are equally far along, the first in [`ConvexHull::vertices`] is returned.
	///
	/// Pass this to [`crate::physics::minkowski_support`] to compare the hull with another shape.
	pub fn support_point(&self, direction: Vector<LocalSpace>) -> Point<LocalSpace> {
		self.corners.furthest(direction)
	}

	/// Returns how far the hull's corners and edges are rounded when a body collides as the hull, in the hull's units.
	/// Flat sides are where they are in the hull.
	///
	/// A body collides as every point within this distance of the hull's core, which is the hull with each side moved
	/// in by as much. While the cores of two shapes are apart, where the shapes touch follows from the cores' closest
	/// points, which takes a fraction of the work that finding how deep two overlapping solids are in each other
	/// does. Bodies at rest are in each other by less than the margin, so resting contacts are the quick ones.
	///
	/// The margin is two hundredths of a unit, or less for a hull that is small or thin: at most a quarter of the
	/// distance from the center of mass to the nearest side.
	pub fn margin(&self) -> f32 {
		self.margin
	}

	/// Returns the corner of the hull's core that is furthest along `direction`. The hull as a body collides with
	/// is every point within [`ConvexHull::margin`] of the core.
	pub(crate) fn core_support(&self, direction: Vector<LocalSpace>) -> Point<LocalSpace> {
		self.core.furthest(direction)
	}

	/// Returns the corners of the hull's core, which [`ConvexHull::core_support`] picks from.
	pub(crate) fn core_vertices(&self) -> &[Point<LocalSpace>] {
		&self.core.points
	}

	/// Returns the center of mass of the hull as a solid of even density.
	///
	/// Pass it to [`crate::physics::Body::with_center_of_mass`] for a body that collides as this hull. The
	/// [`ConvexHull::inertia_tensor`] is taken about this point.
	pub fn center_of_mass(&self) -> Point<LocalSpace> {
		self.center_of_mass
	}

	/// Returns the inertia tensor of the hull as a solid of even density and a mass of one, about its
	/// [`ConvexHull::center_of_mass`] and along the local axes. Multiply it by a body's mass to get that body's
	/// tensor.
	///
	/// The tensor is symmetric. Its diagonal holds the moments of inertia about the x, y, and z axes. The other
	/// entries are the products of inertia with their signs flipped, so that the tensor times a spin gives the
	/// angular momentum, and they are zero only when the hull's mass is balanced about the local axes.
	#[inline]
	pub fn inertia_tensor(&self) -> [[f32; 3]; 3] {
		self.inertia_tensor
	}
}

/// The `Corners` struct keeps the corners of a convex solid twice: as points, and by coordinate in vectors of four,
/// so that the corner furthest along a direction is found four corners at a time. Finding that corner is most of
/// what testing a hull against another shape takes.
#[derive(Debug, Clone, PartialEq)]
struct Corners {
	points: Vec<Point<LocalSpace>>,
	/// The x, the y, and the z of four corners at a time, in the sequence of `points`. The last four are filled up
	/// with copies of the last corner.
	fours: Vec<[f32x4; 3]>,
}

impl Corners {
	/// Keeps `points`, of which there has to be at least one.
	fn new(points: Vec<Point<LocalSpace>>) -> Self {
		let fours = points
			.chunks(4)
			.map(|four| {
				let corners: [[f32; 3]; 4] = std::array::from_fn(|lane| four[lane.min(four.len() - 1)].to_array());
				[0, 1, 2].map(|axis| f32x4::from_array(corners.map(|corner| corner[axis])))
			})
			.collect();
		Self { points, fours }
	}

	/// Returns the corner that is furthest along `direction`. Of corners that are equally far along, the first is
	/// returned.
	fn furthest(&self, direction: Vector<LocalSpace>) -> Point<LocalSpace> {
		let [x, y, z] = direction.to_array().map(f32x4::splat);
		// Each lane looks at every fourth corner. It keeps how far along the furthest of them is and which corner
		// that is, and of corners that are equally far along, the first.
		let mut reach = f32x4::splat(f32::NEG_INFINITY);
		let mut found = u32x4::splat(0);
		let mut corners = u32x4::from_array([0, 1, 2, 3]);
		for four in &self.fours {
			let along = four[0] * x + four[1] * y + four[2] * z;
			let further = along.simd_gt(reach);
			reach = further.select(along, reach);
			found = further.select(corners, found);
			corners += u32x4::splat(4);
		}

		// The furthest of the four lanes' corners, and of equally far ones the first in the sequence of corners.
		// This is worked out without a branch: which lane wins changes from one direction to the next, and a wrong
		// guess at it costs more than the whole search.
		let furthest = reach.simd_eq(f32x4::splat(reach.reduce_max()));
		let first = furthest.select(found, u32x4::splat(u32::MAX)).reduce_min() as usize;
		// A copy that fills up the last four stands for the last corner.
		self.points[first.min(self.points.len() - 1)]
	}
}

/// Returns the center of mass and the inertia tensor, for a mass of one, of the solid of even density inside a
/// closed surface of outward-facing `triangles`. Both come from one pass over the surface, which splits the solid
/// into a tetrahedron from one of its corners to each triangle.
fn mass_distribution(vertices: &[Point<LocalSpace>], triangles: &[[u32; 3]]) -> (Point<LocalSpace>, [[f32; 3]; 3]) {
	// Measuring from a corner, not from the local origin, keeps the sums accurate for a hull far from the origin.
	// Because the hull is convex and its triangles face outward, no tetrahedron has a negative volume to cancel
	// against the others.
	let reference = vertices[0].to_array().map(f64::from);
	let mut volume = 0.0;
	let mut first = [0.0; 3];
	let mut second = [[0.0; 3]; 3];

	for triangle in triangles {
		let [a, b, c] = triangle.map(|vertex| {
			let position = vertices[vertex as usize].to_array();
			[0, 1, 2].map(|axis| f64::from(position[axis]) - reference[axis])
		});
		let part =
			(a[0] * (b[1] * c[2] - b[2] * c[1]) + a[1] * (b[2] * c[0] - b[0] * c[2]) + a[2] * (b[0] * c[1] - b[1] * c[0]))
				/ 6.0;
		let sum = [0, 1, 2].map(|axis| a[axis] + b[axis] + c[axis]);

		// Over a tetrahedron with one corner at the reference, a coordinate averages a quarter of the sum of the
		// other corners' coordinates. The product of two coordinates averages a twentieth of the corners' products
		// added to the product of those sums. A product is the same for its two axes either way around, so only one
		// way is added up.
		volume += part;
		for row in 0..3 {
			first[row] += part * sum[row] / 4.0;
			for column in row..3 {
				let products = a[row] * a[column] + b[row] * b[column] + c[row] * c[column];
				second[row][column] += part * (products + sum[row] * sum[column]) / 20.0;
			}
		}
	}

	// The sums are about the reference corner. The spread of the mass about its center, which is the average of the
	// products of each pair of coordinates measured from there, is less by the offset between the two.
	let offset = first.map(|sum| sum / volume);
	let spread = |row: usize, column: usize| {
		let (row, column) = (row.min(column), row.max(column));
		second[row][column] / volume - offset[row] * offset[column]
	};
	// A moment about an axis adds up the spread along the other two, and a product of inertia is the spread along
	// both of its axes at once.
	let trace = spread(0, 0) + spread(1, 1) + spread(2, 2);
	let tensor = std::array::from_fn(|row| {
		std::array::from_fn(|column| {
			let moment = if row == column { trace } else { 0.0 };
			(moment - spread(row, column)) as f32
		})
	});
	let [x, y, z] = [0, 1, 2].map(|axis| (reference[axis] + offset[axis]) as f32);
	(Point::new(x, y, z), tensor)
}

/// Builds the convex hull of `points`: the smallest convex solid that holds them all.
///
/// Returns `None` when the points enclose no volume, which is when there are fewer than four of them or they all lie
/// on one point, line, or plane. Also returns `None` when a coordinate is not finite.
///
/// A point counts as lying on the hull's surface when it is closer to it than a tolerance of a few rounding errors
/// of the largest coordinate. So the hull has no corners for points that only rounding lifts off a side, such as the
/// inner vertices of a tessellated flat side, and every point is inside the hull or within the tolerance of it. The
/// same tolerance decides whether the points are flat.
///
/// The time this takes grows with the number of points times the number of the hull's corners, so it suits collision
/// shapes better than detailed meshes with thousands of corners.
pub fn build_convex_hull(points: &[Point<LocalSpace>]) -> Option<ConvexHull> {
	let (vertices, triangles) = surface(points)?;

	let (mut min, mut max) = (vertices[0].to_array(), vertices[0].to_array());
	for vertex in &vertices[1..] {
		let vertex = vertex.to_array();
		for axis in 0..3 {
			min[axis] = min[axis].min(vertex[axis]);
			max[axis] = max[axis].max(vertex[axis]);
		}
	}
	let (center_of_mass, inertia_tensor) = mass_distribution(&vertices, &triangles);
	// A hull that has no core collides as it is.
	let (core, margin) = core(&vertices, &triangles, center_of_mass).unwrap_or_else(|| (vertices.clone(), 0.0));
	Some(ConvexHull {
		bounds: AABB::new(Point::from_array(min), Point::from_array(max)),
		corners: Corners::new(vertices),
		triangles,
		center_of_mass,
		inertia_tensor,
		core: Corners::new(core),
		margin,
	})
}

/// Returns the corners of the convex hull of `points`, and the triangles that cover its surface as indices into
/// those corners, as [`ConvexHull::vertices`] and [`ConvexHull::triangles`] describe them. Returns `None` for the
/// points that [`build_convex_hull`] has no hull for.
fn surface(points: &[Point<LocalSpace>]) -> Option<(Vec<Point<LocalSpace>>, Vec<[u32; 3]>)> {
	if points.len() < 4 {
		return None;
	}
	// Faces name points with a `u32`.
	let count = u32::try_from(points.len()).ok()?;
	let (grid, tolerance) = snap_to_grid(points)?;
	let grid = grid.as_slice();
	let (mut faces, anchor) = build_tetrahedron(grid, tolerance)?;

	// The hull grows from the tetrahedron by one point at a time: the faces that the point is above are removed, and
	// the hole they leave is closed with a triangle from each edge of its rim to the point. Whether a face is removed
	// is decided in exact arithmetic, so the faces always are the exact hull of the corners chosen so far. That is
	// what makes the removed faces one patch with one rim. If rounding could call one face above a point and its
	// neighbor in the same plane not, the patch could have holes or pinches, and the surface would stop being closed.
	let mut outside: Vec<Outsider> = (0..count)
		.filter_map(|point| {
			furthest_face(&faces, grid[point as usize], tolerance).map(|(face, height)| Outsider { point, face, height })
		})
		.collect();
	let mut rim: Vec<[u32; 2]> = Vec::new();
	let mut moved: Vec<u32> = Vec::new();
	let mut heights: Vec<f64> = Vec::new();

	while let Some(start) = outside.iter().max_by(|left, right| left.height.total_cmp(&right.height)) {
		// The next corner is the point furthest out from a face that still has points above it. No point is further
		// out that way, so it is on the final hull.
		let face = faces[start.face as usize];
		let (index, _) = most_extreme(
			outside.len(),
			|index| face.height(grid[outside[index].point as usize]),
			|index| distance_squared(anchor, grid[outside[index].point as usize]),
			tolerance,
			&mut heights,
		)?;
		let eye = outside.swap_remove(index).point;
		moved.clear();
		add_corner(&mut faces, |point| grid[point as usize], eye, &mut rim, |to| moved.push(to));

		// A point that was above a removed face may be above any other face now, or inside the hull.
		outside.retain_mut(|outsider| {
			let face = moved[outsider.face as usize];
			if face != REMOVED {
				outsider.face = face;
				return true;
			}
			let Some((face, height)) = furthest_face(&faces, grid[outsider.point as usize], tolerance) else {
				return false;
			};
			outsider.face = face;
			outsider.height = height;
			true
		});
	}

	// A corner that a later one covered has no face left, so only the corners that faces still name are returned.
	// A closed surface of triangles has two more corners than half its triangles.
	let slots = &mut moved;
	slots.clear();
	slots.resize(points.len(), REMOVED);
	let mut vertices = Vec::with_capacity(faces.len() / 2 + 2);
	let triangles = faces
		.iter()
		.map(|face| {
			face.corners.map(|corner| {
				let slot = &mut slots[corner as usize];
				if *slot == REMOVED {
					*slot = vertices.len() as u32;
					vertices.push(points[corner as usize]);
				}
				*slot
			})
		})
		.collect::<Vec<_>>();
	Some((vertices, triangles))
}

/// How far short of one the dot product of the directions that two triangles of a hull face may be for the
/// triangles to count as parts of one flat side. Rounding turns the triangles of one side by far less than this.
const SAME_SIDE: f64 = 1e-6;

/// Returns the corners of a hull's core, and the margin the core is for: the solid that is left when every side of
/// the hull is moved in by the margin. Every point within the margin of the core is in the hull, and the two differ
/// only near the hull's corners and edges, which are round on the core's side.
///
/// `vertices` and `triangles` are the hull's surface and `center` is a point inside it, from which the margin is
/// worked out: [`MARGIN`], or less for a hull whose nearest side is close to the center.
///
/// Moving the sides in can do more than move the corners. A corner where more than three sides meet may come apart
/// into several, and a small side may vanish. So the core is not worked out corner by corner. Each side, as seen
/// from the center, is a point in the direction it faces, nearer the further away the side is. The hull of those
/// points has one face for each corner of the solid that the sides enclose, with the sides that meet there at its
/// corners.
///
/// Returns `None` when the hull has no room for a core.
fn core(
	vertices: &[Point<LocalSpace>],
	triangles: &[[u32; 3]],
	center: Point<LocalSpace>,
) -> Option<(Vec<Point<LocalSpace>>, f32)> {
	let center = center.to_array().map(f64::from);
	let from_center = |point: Point<LocalSpace>| {
		let point = point.to_array();
		[0, 1, 2].map(|axis| f64::from(point[axis]) - center[axis])
	};
	let subtract = |left: [f64; 3], right: [f64; 3]| [0, 1, 2].map(|axis| left[axis] - right[axis]);
	let cross = |left: [f64; 3], right: [f64; 3]| {
		[
			left[1] * right[2] - left[2] * right[1],
			left[2] * right[0] - left[0] * right[2],
			left[0] * right[1] - left[1] * right[0],
		]
	};
	let dot = |left: [f64; 3], right: [f64; 3]| left[0] * right[0] + left[1] * right[1] + left[2] * right[2];

	// The hull's flat sides, each as the direction it faces and how far from the center it is. The triangles of one
	// side face the same way to within rounding, and the nearest of them stands for the side.
	let mut sides: Vec<([f64; 3], f64)> = Vec::new();
	let mut size: f64 = 0.0;
	for triangle in triangles {
		let [a, b, c] = triangle.map(|vertex| from_center(vertices[vertex as usize]));
		let outward = cross(subtract(b, a), subtract(c, a));
		let length = dot(outward, outward).sqrt();
		let outward = outward.map(|part| part / length);
		let distance = dot(outward, a);
		size = size.max(dot(a, a).sqrt());
		match sides.iter_mut().find(|(other, _)| dot(*other, outward) > 1.0 - SAME_SIDE) {
			Some((_, nearest)) => *nearest = nearest.min(distance),
			None => sides.push((outward, distance)),
		}
	}
	let nearest = sides.iter().map(|(_, distance)| *distance).fold(f64::INFINITY, f64::min);
	let margin = f64::from(MARGIN).min(nearest * MARGIN_SHARE);
	if margin <= 0.0 || !margin.is_finite() {
		return None;
	}

	// One point for each side once it is moved in. A side that the others then leave no part of ends up inside the
	// hull of these points, and takes no part in the core.
	let moved: Vec<Point<LocalSpace>> = sides
		.iter()
		.map(|(outward, distance)| {
			let [x, y, z] = outward.map(|part| (part / (distance - margin)) as f32);
			Point::new(x, y, z)
		})
		.collect();
	let (moved, faces) = surface(&moved)?;

	// Rounding may leave a corner this far outside a side, or two corners that are one this far apart.
	let (outside, same) = (1e-5 * size, 1e-6 * size);
	let mut corners: Vec<[f64; 3]> = Vec::new();
	for face in &faces {
		// The corner where the three sides at the corners of this face meet is the point whose dot product with
		// each of their three points is one.
		let [a, b, c] = face.map(|side| moved[side as usize].to_array().map(f64::from));
		let level = cross(subtract(b, a), subtract(c, a));
		let scale = dot(level, a);
		if scale <= 0.0 || scale.is_nan() {
			continue;
		}
		let corner = level.map(|part| part / scale);
		// Three sides that nearly face the same way meet far from where the core is. Such a face comes from sides
		// that rounding left a sliver between, and the core's real corner there comes from a face next to it.
		let inside = sides
			.iter()
			.all(|(outward, distance)| dot(*outward, corner) <= distance - margin + outside);
		let known = corners.iter().any(|&other| {
			let apart = subtract(other, corner);
			dot(apart, apart) < same * same
		});
		if inside && !known && corner.iter().all(|part| part.is_finite()) {
			corners.push(corner);
		}
	}
	if corners.len() < 4 {
		return None;
	}
	let core = corners
		.into_iter()
		.map(|corner| {
			let [x, y, z] = [0, 1, 2].map(|axis| (center[axis] + corner[axis]) as f32);
			Point::new(x, y, z)
		})
		.collect();
	Some((core, margin as f32))
}

/// The `HullFace` trait lets [`add_corner`] grow a hull whose faces keep more than a [`Face`], so that what is kept
/// beside a face is made with it and removed with it.
pub(super) trait HullFace: Copy {
	/// Returns the triangle and its plane.
	fn face(&self) -> &Face;

	/// Makes a face of a hull from a triangle and its plane.
	fn from_face(face: Face) -> Self;
}

impl HullFace for Face {
	fn face(&self) -> &Face {
		self
	}

	fn from_face(face: Face) -> Self {
		face
	}
}

/// Grows a hull to hold the point `eye`, which has to be outside at least one of its `faces`: removes the faces that
/// the point is outside of, and closes the hole they leave with a triangle from each edge of its rim to the point.
/// Growing a hull to a point that it already holds changes nothing.
///
/// `position_of` returns where a point is on the grid. `rim` is working space. `moved` is told, for each face that
/// was there before and in their sequence, the face's new index or [`REMOVED`].
pub(super) fn add_corner<F: HullFace>(
	faces: &mut Vec<F>,
	position_of: impl Fn(u32) -> GridPoint,
	eye: u32,
	rim: &mut Vec<[u32; 2]>,
	mut moved: impl FnMut(u32),
) {
	let position = position_of(eye);

	// Remove the faces below the new corner, and note where each of the others moves to.
	rim.clear();
	let mut kept = 0;
	for index in 0..faces.len() {
		let face = faces[index];
		if face.face().side(position) > 0 {
			let [a, b, c] = face.face().corners;
			rim.extend([[a, b], [b, c], [c, a]]);
			moved(REMOVED);
		} else {
			faces[kept] = face;
			moved(kept as u32);
			kept += 1;
		}
	}
	faces.truncate(kept);

	// Two removed faces that share an edge both list it, in opposite directions. An edge listed once is on the rim of
	// the hole, and running the same way as the removed face did, it and the new corner face outward.
	rim.sort_unstable_by_key(|&[from, to]| (from.min(to), from.max(to)));
	let mut index = 0;
	while index < rim.len() {
		let [from, to] = rim[index];
		if rim.get(index + 1) == Some(&[to, from]) {
			index += 2;
		} else {
			let corners = [from, to, eye];
			faces.push(F::from_face(Face::new(
				corners,
				[position_of(from), position_of(to), position],
			)));
			index += 1;
		}
	}
}

/// How finely points are snapped: their coordinates span at most `2^GRID_BITS` grid steps on each axis. With 26 bits
/// a step is under the rounding error of an `f32` coordinate at the bounds, and every product below fits its integer.
pub(super) const GRID_BITS: u32 = 26;

/// Marks a face that was removed, or a point that is not a corner, in a table of new indices.
pub(super) const REMOVED: u32 = u32::MAX;

/// A point's whole coordinates on the grid.
pub(super) type GridPoint = [i32; 3];

/// The `Face` struct keeps a triangle of the growing hull with its plane, so that points can be tested against it
/// without rounding.
#[derive(Clone, Copy)]
pub(super) struct Face {
	/// The points at the triangle's corners, counterclockwise when seen from outside the hull.
	pub(super) corners: [u32; 3],
	/// The first corner's position.
	origin: GridPoint,
	/// The exact cross product of two of the triangle's sides, which points out of the hull.
	normal: [i64; 3],
	/// The length of `normal`.
	length: f64,
}

impl Face {
	/// Makes the face with these `corners`, which are at these `positions` on the grid.
	pub(super) fn new(corners: [u32; 3], [a, b, c]: [GridPoint; 3]) -> Self {
		let normal = cross(offset(a, b), offset(a, c));
		debug_assert!(
			normal != [0; 3],
			"A hull face has no area. The most likely cause is a corner in line with the edge it was joined to."
		);
		Self {
			corners,
			origin: a,
			normal,
			length: length(normal),
		}
	}

	/// Returns a number that is positive when `point` is outside the face's plane, negative when it is inside, and
	/// zero when it is on it. It is exact, so it gives every face in one plane the same answer.
	pub(super) fn side(&self, point: GridPoint) -> i128 {
		dot(self.normal, offset(self.origin, point))
	}

	/// Returns how many grid steps `point` is outside the face's plane.
	///
	/// This is rounded, to a tiny part of a step, where [`Self::side`] is exact. It is for telling how far a point
	/// is from a face, and never for telling which side it is on.
	// Worked out in floating point from the start. Converting the exact 128-bit result of `side` takes a call to a
	// routine that is slow enough to have been 6% of a physics step of hull piles on an Apple M4.
	pub(super) fn height(&self, point: GridPoint) -> f64 {
		let offset = offset(self.origin, point);
		(0..3).map(|axis| self.normal[axis] as f64 * offset[axis] as f64).sum::<f64>() / self.length
	}

	/// Returns the direction that the face looks in, out of the hull, with a length of one.
	pub(super) fn outward(&self) -> [f32; 3] {
		self.normal.map(|part| (part as f64 / self.length) as f32)
	}
}

/// The `Outsider` struct tracks a point that is not known to be inside the hull yet, with one face that proves it.
struct Outsider {
	point: u32,
	/// A face that the point is more than the tolerance above.
	face: u32,
	/// How many grid steps the point is above that face.
	height: f64,
}

/// Snaps `points` to a grid of whole coordinates, and returns them with the tolerance in grid steps.
///
/// Whole coordinates let every later decision about which side of a plane a point is on be exact. Returns `None` when
/// a coordinate is not finite, or when the points are closer together than the tolerance. There must be a point.
fn snap_to_grid(points: &[Point<LocalSpace>]) -> Option<(Vec<GridPoint>, f64)> {
	if !points.iter().all(|point| is_finite(*point)) {
		return None;
	}

	let mut min = [f64::INFINITY; 3];
	let mut max = [f64::NEG_INFINITY; 3];
	for point in points {
		let coordinates = point.to_array();
		for axis in 0..3 {
			min[axis] = min[axis].min(f64::from(coordinates[axis]));
			max[axis] = max[axis].max(f64::from(coordinates[axis]));
		}
	}

	// An `f32` coordinate is rounded by up to its size times `f32::EPSILON`, so a point that should be on a plane can
	// be off it by about that much on each axis. Three times the sum over the axes leaves a margin for the rounding of
	// the other points that set the plane.
	let reach: f64 = (0..3).map(|axis| min[axis].abs().max(max[axis].abs())).sum();
	let tolerance = 3.0 * f64::from(f32::EPSILON) * reach;
	let extent = (0..3).map(|axis| max[axis] - min[axis]).fold(0.0, f64::max);
	if extent <= tolerance {
		return None;
	}

	// One scale for every axis keeps distances on the grid in proportion to distances between the points.
	let scale = f64::from(1u32 << GRID_BITS) / extent;
	let grid = points
		.iter()
		.map(|point| {
			let coordinates = point.to_array();
			[0, 1, 2].map(|axis| ((f64::from(coordinates[axis]) - min[axis]) * scale).round() as i32)
		})
		.collect();

	Some((grid, tolerance * scale))
}

/// Builds the four faces of a tetrahedron on four of the points, to grow the hull from, and returns them with the
/// position of one of its corners.
///
/// Every corner is chosen to be a corner of the final hull, so that none has to be removed later. Returns `None` when
/// the points are within `tolerance` of one line or one plane.
fn build_tetrahedron(grid: &[GridPoint], tolerance: f64) -> Option<(Vec<Face>, GridPoint)> {
	let points = 0..grid.len();
	let mut heights = Vec::new();

	// The point furthest from any given position is a corner of the hull.
	let first = *grid.first()?;
	let a = points.clone().max_by_key(|&point| distance_squared(first, grid[point]))?;
	let b = points.max_by_key(|&point| distance_squared(grid[a], grid[point]))?;

	let side = offset(grid[a], grid[b]);
	let side_length = length(side);
	let (c, width) = most_extreme(
		grid.len(),
		|point| length(cross(side, offset(grid[a], grid[point]))) / side_length,
		|point| distance_squared(grid[a], grid[point]),
		tolerance,
		&mut heights,
	)?;
	if width <= tolerance {
		return None;
	}

	let base = Face::new([a as u32, b as u32, c as u32], [grid[a], grid[b], grid[c]]);
	let (d, depth) = most_extreme(
		grid.len(),
		|point| base.height(grid[point]).abs(),
		|point| distance_squared(grid[a], grid[point]),
		tolerance,
		&mut heights,
	)?;
	if depth <= tolerance {
		return None;
	}

	// The base has to face away from the fourth corner for the faces below to face outward.
	let (a, b) = if base.side(grid[d]) > 0 { (b, a) } else { (a, b) };
	let [a, b, c, d] = [a, b, c, d].map(|point| point as u32);
	let faces = [[a, b, c], [a, c, d], [c, b, d], [b, a, d]];

	Some((
		faces
			.into_iter()
			.map(|corners| Face::new(corners, corners.map(|corner| grid[corner as usize])))
			.collect(),
		grid[a as usize],
	))
}

/// Returns the one of `count` candidates that `height` rates highest, with the highest rating. Candidates within
/// `tolerance` of that rating count as level, and of those the one that `spread` rates highest is returned.
///
/// Here `height` is a distance from a plane or from a line, so level candidates share a flat side or a straight edge
/// of the cloud, and `spread` is the distance from a fixed position. The furthest of them from that position is at an
/// end of the side or the edge, which makes it a corner of the hull where an inner one would not be.
///
/// `heights` is working space that keeps each candidate's rating between finding the highest and finding the
/// candidates level with it.
fn most_extreme(
	count: usize,
	height: impl Fn(usize) -> f64,
	spread: impl Fn(usize) -> i128,
	tolerance: f64,
	heights: &mut Vec<f64>,
) -> Option<(usize, f64)> {
	heights.clear();
	heights.extend((0..count).map(height));
	let highest = heights.iter().copied().max_by(f64::total_cmp)?;
	let candidate = (0..count)
		.filter(|&candidate| heights[candidate] >= highest - tolerance)
		.max_by_key(|&candidate| spread(candidate))?;
	Some((candidate, highest))
}

/// Returns the face that `point` is furthest above, with how many grid steps above, when that is more than
/// `tolerance`. Otherwise the point is inside the hull or on its surface.
fn furthest_face(faces: &[Face], point: GridPoint, tolerance: f64) -> Option<(u32, f64)> {
	faces
		.iter()
		.enumerate()
		.map(|(index, face)| (index as u32, face.height(point)))
		.max_by(|left, right| left.1.total_cmp(&right.1))
		.filter(|&(_, height)| height > tolerance)
}

// Grid coordinates take `GRID_BITS` bits, so an offset takes one more with its sign, a cross product of two offsets
// under 55, and a dot product with a third offset under 83. None of these can overflow.

pub(super) fn offset(from: GridPoint, to: GridPoint) -> [i64; 3] {
	[0, 1, 2].map(|axis| i64::from(to[axis]) - i64::from(from[axis]))
}

pub(super) fn cross(left: [i64; 3], right: [i64; 3]) -> [i64; 3] {
	[
		left[1] * right[2] - left[2] * right[1],
		left[2] * right[0] - left[0] * right[2],
		left[0] * right[1] - left[1] * right[0],
	]
}

pub(super) fn dot(left: [i64; 3], right: [i64; 3]) -> i128 {
	(0..3).map(|axis| i128::from(left[axis]) * i128::from(right[axis])).sum()
}

// In floating point from the start, for the reason given at `Face::height`.
fn length(vector: [i64; 3]) -> f64 {
	vector.iter().map(|&part| part as f64 * part as f64).sum::<f64>().sqrt()
}

pub(super) fn distance_squared(from: GridPoint, to: GridPoint) -> i128 {
	let offset = offset(from, to);
	dot(offset, offset)
}

#[cfg(test)]
mod tests {
	use std::collections::HashSet;

	use super::*;

	/// Returns `count` evenly spread values from -1 to 1.
	fn steps(count: usize) -> impl Iterator<Item = f64> + Clone {
		(0..count).map(move |step| -1.0 + 2.0 * step as f64 / (count - 1) as f64)
	}

	/// Returns the points of a lattice with `count` points along each side of a cube from -1 to 1. With `surface`,
	/// only the points on the cube's sides are returned.
	fn lattice(count: usize, surface: bool) -> Vec<[f64; 3]> {
		let mut points = Vec::new();
		for x in steps(count) {
			for y in steps(count) {
				for z in steps(count) {
					if !surface || [x, y, z].iter().any(|coordinate| coordinate.abs() == 1.0) {
						points.push([x, y, z]);
					}
				}
			}
		}
		points
	}

	/// Turns `point` about two axes by angles that line no side of a cube up with an axis, scales it, and moves it.
	fn place(point: [f64; 3], scale: f64, origin: [f64; 3]) -> Point<LocalSpace> {
		let [x, y, z] = point;
		let (sin, cos) = 0.7f64.sin_cos();
		let (x, y) = (x * cos - y * sin, x * sin + y * cos);
		let (sin, cos) = 0.4f64.sin_cos();
		let (y, z) = (y * cos - z * sin, y * sin + z * cos);
		Point::new(
			(x * scale + origin[0]) as f32,
			(y * scale + origin[1]) as f32,
			(z * scale + origin[2]) as f32,
		)
	}

	fn local(points: &[[f64; 3]]) -> Vec<Point<LocalSpace>> {
		points
			.iter()
			.map(|&[x, y, z]| Point::new(x as f32, y as f32, z as f32))
			.collect()
	}

	/// Returns `count` points with coordinates from -1 to 1 that are the same on every run.
	fn scattered(count: usize) -> Vec<[f64; 3]> {
		let mut state = 0x2545_f491_4f6c_dd1du64;
		let mut next = move || {
			state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			(state >> 40) as f64 / (1u64 << 23) as f64 - 1.0
		};
		(0..count).map(|_| [next(), next(), next()]).collect()
	}

	fn coordinates(point: Point<LocalSpace>) -> [f64; 3] {
		point.to_array().map(f64::from)
	}

	/// Returns how far `point` is outside the plane of each of the hull's triangles. A point inside the hull is
	/// outside none, so every distance is negative.
	fn heights(hull: &ConvexHull, point: Point<LocalSpace>) -> impl Iterator<Item = f64> + '_ {
		let point = coordinates(point);
		hull.triangles().iter().map(move |triangle| {
			let [a, b, c] = triangle.map(|vertex| coordinates(hull.vertices()[vertex as usize]));
			let (u, v, w) = (subtract(b, a), subtract(c, a), subtract(point, a));
			let normal = cross(u, v);
			let length = dot(normal, normal).sqrt();
			assert!(length > 0.0, "a triangle has no area");
			dot(normal, w) / length
		})
	}

	fn subtract(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
		[left[0] - right[0], left[1] - right[1], left[2] - right[2]]
	}

	fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
		[
			left[1] * right[2] - left[2] * right[1],
			left[2] * right[0] - left[0] * right[2],
			left[0] * right[1] - left[1] * right[0],
		]
	}

	fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
		left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
	}

	/// Asserts that `hull` is a closed surface of outward-facing triangles on some of `points`, and that no point is
	/// more than `margin` outside any of its triangles.
	fn assert_encloses(hull: &ConvexHull, points: &[Point<LocalSpace>], margin: f64) {
		let vertices = hull.vertices();
		let triangles = hull.triangles();

		assert!(vertices.iter().all(|vertex| points.contains(vertex)));
		assert_eq!(triangles.len(), 2 * vertices.len() - 4);

		// On a closed surface every edge of a triangle is an edge of exactly one other, which runs along it the other
		// way.
		let edges: HashSet<[u32; 2]> = triangles.iter().flat_map(|&[a, b, c]| [[a, b], [b, c], [c, a]]).collect();
		assert_eq!(edges.len(), 3 * triangles.len());
		assert!(edges.iter().all(|&[from, to]| edges.contains(&[to, from])));

		for &point in points {
			let height = heights(hull, point).fold(f64::NEG_INFINITY, f64::max);
			assert!(height <= margin, "a point is {height} outside the hull");
		}
	}

	#[test]
	fn four_points_with_volume_make_a_tetrahedron() {
		let points = local(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), 4);
		assert_eq!(hull.triangles().len(), 4);
		assert_encloses(&hull, &points, 0.0);
	}

	#[test]
	fn a_filled_cube_keeps_only_its_corners() {
		let points = local(&lattice(5, false));

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), 8);
		assert_eq!(hull.triangles().len(), 12);
		assert!(
			hull.vertices()
				.iter()
				.all(|vertex| vertex.to_array().iter().all(|coordinate| coordinate.abs() == 1.0))
		);
		assert_encloses(&hull, &points, 0.0);
	}

	#[test]
	fn points_that_rounding_lifts_off_a_side_add_no_corners() {
		// Turned, the points of each side are no longer exactly on one plane, and those of each edge no longer in line.
		for (scale, origin) in [
			(1.0, [0.0; 3]),
			(1e-4, [0.0; 3]),
			(1e4, [0.0; 3]),
			(1.0, [100.0, -50.0, 25.0]),
		] {
			let points: Vec<_> = lattice(9, true)
				.into_iter()
				.map(|point| place(point, scale, origin))
				.collect();

			let hull = build_convex_hull(&points).expect("the points enclose a volume");

			assert_eq!(hull.vertices().len(), 8, "at scale {scale} around {origin:?}");
			assert_eq!(hull.triangles().len(), 12);
			let reach = scale * 2.0 + origin.iter().map(|coordinate| coordinate.abs()).sum::<f64>();
			assert_encloses(&hull, &points, 1e-5 * reach);
		}
	}

	#[test]
	fn every_point_of_a_sphere_is_a_corner() {
		// A spiral that spreads the points evenly over the sphere.
		let count = 300;
		let points: Vec<_> = (0..count)
			.map(|index| {
				let height = 1.0 - 2.0 * (index as f64 + 0.5) / count as f64;
				let radius = (1.0 - height * height).sqrt();
				let turn = index as f64 * 2.399963229728653;
				[radius * turn.cos(), height, radius * turn.sin()]
			})
			.collect();
		let points = local(&points);

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), count);
		assert_encloses(&hull, &points, 1e-6);
	}

	#[test]
	fn a_prism_keeps_the_corners_of_both_ends() {
		// Each end is one flat side with many corners, all on a circle.
		let sides = 32;
		let points: Vec<_> = (0..sides)
			.flat_map(|side| {
				let turn = side as f64 / sides as f64 * std::f64::consts::TAU;
				[[turn.cos(), -1.0, turn.sin()], [turn.cos(), 1.0, turn.sin()]]
			})
			.collect();
		let points = local(&points);

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), 2 * sides);
		assert_encloses(&hull, &points, 1e-6);
	}

	#[test]
	fn a_scattered_cloud_is_enclosed() {
		let points = local(&scattered(4000));

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert!(hull.vertices().len() < points.len());
		assert_encloses(&hull, &points, 1e-6);
	}

	#[test]
	fn clouds_on_a_coarse_lattice_get_exactly_their_corners() {
		// Points with small whole coordinates are often exactly in line or in one plane with others, and never nearly
		// so. Their hull involves no rounding, so it has to be exact.
		let mut state = 0x9e37_79b9_7f4a_7c15u64;
		let mut next = move |bound: u64| {
			state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			(state >> 33) % bound
		};

		for round in 0..500 {
			let size = 2 + next(5);
			let cloud: Vec<_> = (0..1 + next(60))
				.map(|_| [next(size) as f64, next(size) as f64, next(size) as f64])
				.collect();
			let points = local(&cloud);
			// The same cloud turned and moved, which rounds its points off the lines and planes they share.
			let turned: Vec<_> = cloud.iter().map(|&point| place(point, 0.37, [5.0, -3.0, 11.0])).collect();

			let Some(hull) = build_convex_hull(&points) else {
				// The cloud has to be flat: no three points span a volume with the first and a fourth.
				let first = cloud[0];
				let mut volumes = cloud.iter().flat_map(|&b| {
					let cloud = &cloud;
					cloud.iter().flat_map(move |&c| {
						cloud
							.iter()
							.map(move |&d| dot(cross(subtract(b, first), subtract(c, first)), subtract(d, first)))
					})
				});
				assert!(
					volumes.all(|volume| volume == 0.0),
					"round {round}: a solid cloud has no hull"
				);
				assert!(
					build_convex_hull(&turned).is_none(),
					"round {round}: a flat cloud has a hull once turned"
				);
				continue;
			};

			assert_encloses(&hull, &points, 0.0);

			// Every corner is needed: it is outside the hull of the others, when they have one.
			for (index, &vertex) in hull.vertices().iter().enumerate() {
				let mut others = hull.vertices().to_vec();
				others.remove(index);
				if let Some(smaller) = build_convex_hull(&others) {
					assert!(
						heights(&smaller, vertex).any(|height| height > 0.0),
						"round {round}: a corner is not one"
					);
				}
			}

			let turned_hull = build_convex_hull(&turned).expect("the turned points enclose a volume");
			assert_eq!(turned_hull.vertices().len(), hull.vertices().len(), "round {round}");
			assert_encloses(&turned_hull, &turned, 2e-5);
		}
	}

	/// Returns the corners of a box with the given half sizes, turned, scaled, and moved as [`place`] does.
	fn placed_box(half_sizes: [f64; 3], scale: f64, origin: [f64; 3]) -> Vec<Point<LocalSpace>> {
		lattice(2, false)
			.into_iter()
			.map(|[x, y, z]| place([x * half_sizes[0], y * half_sizes[1], z * half_sizes[2]], scale, origin))
			.collect()
	}

	#[test]
	fn a_box_hull_has_the_inertia_of_a_box_about_its_middle() {
		let half_sizes = [0.5, 1.0, 1.5];
		// Away from the origin, so that the tensor has to be taken about the box and not about the origin.
		let middle = [4.0, -3.0, 2.0];
		let points: Vec<_> = lattice(2, false)
			.into_iter()
			.map(|[x, y, z]| {
				Point::new(
					(x * half_sizes[0] + middle[0]) as f32,
					(y * half_sizes[1] + middle[1]) as f32,
					(z * half_sizes[2] + middle[2]) as f32,
				)
			})
			.collect();

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		let center = coordinates(hull.center_of_mass());
		assert!((0..3).all(|axis| (center[axis] - middle[axis]).abs() <= 1e-5), "{center:?}");

		let moments = crate::physics::Shapes::cube(math::Vector::new(0.5, 1.0, 1.5)).principal_inertia();
		let expected = [[moments.x, 0.0, 0.0], [0.0, moments.y, 0.0], [0.0, 0.0, moments.z]];
		let tensor = hull.inertia_tensor();
		for row in 0..3 {
			for column in 0..3 {
				assert!((tensor[row][column] - expected[row][column]).abs() <= 1e-5, "{tensor:?}");
			}
		}
	}

	#[test]
	fn a_tetrahedron_hull_has_products_of_inertia() {
		let points = local(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert!((hull.center_of_mass() - Point::new(0.25, 0.25, 0.25)).length() <= 1e-6);
		// The known tensor of this tetrahedron about its center of mass: 3/40 on the diagonal and 1/80 elsewhere.
		let tensor = hull.inertia_tensor();
		for row in 0..3 {
			for column in 0..3 {
				let expected = if row == column { 3.0 / 40.0 } else { 1.0 / 80.0 };
				assert!((tensor[row][column] - expected).abs() <= 1e-6, "{tensor:?}");
			}
		}
	}

	#[test]
	fn a_turned_box_hull_turns_its_inertia_with_it() {
		let half_sizes = [0.5, 1.0, 1.5];
		let (scale, origin) = (2.0, [10.0, 20.0, -30.0]);

		let hull = build_convex_hull(&placed_box(half_sizes, scale, origin)).expect("the points enclose a volume");

		let middle = place([0.0; 3], scale, origin);
		assert!((hull.center_of_mass() - middle).length() <= 1e-4);

		// About each of its own axes, a box resists turning by a twelfth of the sum of its other two sizes squared.
		// Those axes turned with the box, so the tensor has to scale each of them by that moment and leave its
		// direction alone.
		let tensor = hull.inertia_tensor();
		for axis in 0..3 {
			let mut along = [0.0; 3];
			along[axis] = 1.0;
			let direction = subtract(
				coordinates(place(along, 1.0, [0.0; 3])),
				coordinates(place([0.0; 3], 1.0, [0.0; 3])),
			);
			let sizes = half_sizes.map(|half| 2.0 * half * scale);
			let moment = (sizes[(axis + 1) % 3].powi(2) + sizes[(axis + 2) % 3].powi(2)) / 12.0;

			for row in 0..3 {
				let turned: f64 = (0..3).map(|column| f64::from(tensor[row][column]) * direction[column]).sum();
				assert!(
					(turned - moment * direction[row]).abs() <= 1e-4 * moment,
					"axis {axis}: {tensor:?}"
				);
			}
		}
	}

	#[test]
	fn the_support_point_is_the_corner_furthest_along_a_direction() {
		let hull = build_convex_hull(&local(&lattice(3, false))).expect("the points enclose a volume");

		assert_eq!(hull.support_point(Vector::new(1.0, 2.0, 3.0)), Point::new(1.0, 1.0, 1.0));
		assert_eq!(hull.support_point(Vector::new(-0.1, 5.0, -0.2)), Point::new(-1.0, 1.0, -1.0));
		// Every corner is as far along no direction as any other, and one of them is still returned.
		assert!(hull.vertices().contains(&hull.support_point(Vector::zero())));
	}

	/// Returns `count` directions spread evenly over a sphere, followed by the six along the axes.
	fn directions(count: usize) -> Vec<Vector<LocalSpace>> {
		let spiral = (0..count).map(|index| {
			let height = 1.0 - 2.0 * (index as f32 + 0.5) / count as f32;
			let (sin, cos) = (index as f32 * 2.399_963).sin_cos();
			let width = (1.0 - height * height).sqrt();
			Vector::new(width * cos, height, width * sin)
		});
		let axes = [-1.0, 1.0].into_iter().flat_map(|sign| {
			[
				Vector::new(sign, 0.0, 0.0),
				Vector::new(0.0, sign, 0.0),
				Vector::new(0.0, 0.0, sign),
			]
		});
		spiral.chain(axes).collect()
	}

	/// Returns the points of a few solids with flat sides: a cube, a diamond with four sides at each tip, a prism,
	/// and a pyramid whose four slanted sides lean differently.
	fn solids() -> Vec<(&'static str, Vec<Point<LocalSpace>>)> {
		let diamond = [
			[1.0, 0.0, 0.0],
			[-1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[0.0, -1.0, 0.0],
			[0.0, 0.0, 1.0],
			[0.0, 0.0, -1.0],
		];
		let prism: Vec<[f64; 3]> = (0..6)
			.flat_map(|side| {
				let turn = f64::from(side) / 6.0 * std::f64::consts::TAU;
				[-0.5, 0.5].map(|height| [0.5 * turn.cos(), height, 0.5 * turn.sin()])
			})
			.collect();
		let pyramid = [
			[-1.0, 0.0, -1.0],
			[1.0, 0.0, -1.0],
			[1.0, 0.0, 1.0],
			[-1.0, 0.0, 1.0],
			[0.3, 1.5, -0.2],
		];
		vec![
			("cube", local(&lattice(2, false))),
			("diamond", local(&diamond)),
			("prism", local(&prism)),
			("pyramid", local(&pyramid)),
		]
	}

	/// Returns how far along `direction` `point` is.
	fn along(direction: Vector<LocalSpace>, point: Point<LocalSpace>) -> f32 {
		let (direction, point) = (direction.to_array(), point.to_array());
		direction[0] * point[0] + direction[1] * point[1] + direction[2] * point[2]
	}

	#[test]
	fn the_support_point_is_the_first_of_the_corners_that_are_furthest_along() {
		// The solids have four, five, six, eight, twelve, and many corners, so their corners fill whole groups of
		// four and leave some over. Along an axis, several corners of most of them are equally far.
		let mut clouds = solids();
		clouds.push((
			"tetrahedron",
			local(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]),
		));
		clouds.push(("cloud", local(&scattered(200))));

		for (name, points) in clouds {
			let hull = build_convex_hull(&points).expect("the points enclose a volume");
			for direction in directions(96) {
				let mut furthest = hull.vertices()[0];
				for &vertex in hull.vertices() {
					if along(direction, vertex) > along(direction, furthest) {
						furthest = vertex;
					}
				}

				assert_eq!(hull.support_point(direction), furthest, "{name} along {direction:?}");
			}
		}
	}

	#[test]
	fn a_hull_has_a_margin_that_a_small_hull_has_less_of() {
		let cube = |half: f64| {
			let corners: Vec<[f64; 3]> = lattice(2, false)
				.into_iter()
				.map(|corner| corner.map(|part| part * half))
				.collect();
			build_convex_hull(&local(&corners)).expect("a cube encloses a volume")
		};

		assert_eq!(cube(1.0).margin(), 0.02);
		assert_eq!(cube(100.0).margin(), 0.02);
		// A quarter of the way from the middle to a side.
		assert!((cube(0.04).margin() - 0.01).abs() <= 1e-6);
		assert!((cube(0.001).margin() - 0.00025).abs() <= 1e-7);
	}

	#[test]
	fn the_core_has_every_side_of_the_hull_moved_in_by_the_margin() {
		for (name, points) in solids() {
			let hull = build_convex_hull(&points).expect("the points enclose a volume");
			let margin = hull.margin();
			assert!(margin > 0.0, "{name}");

			for triangle in hull.triangles() {
				let [a, b, c] = triangle.map(|vertex| coordinates(hull.vertices()[vertex as usize]));
				let outward = cross(subtract(b, a), subtract(c, a));
				let length = dot(outward, outward).sqrt();
				let [x, y, z] = outward.map(|part| (part / length) as f32);
				let outward = Vector::new(x, y, z);

				let side = along(outward, hull.support_point(outward));
				let core = along(outward, hull.core_support(outward));
				assert!(
					(core + margin - side).abs() <= 1e-5,
					"{name}: {core} + {margin} is not {side}"
				);
			}
		}
	}

	#[test]
	fn every_point_within_the_margin_of_the_core_is_in_the_hull() {
		let mut clouds = solids();
		clouds.push(("cloud", local(&scattered(60))));
		clouds.push((
			"ball",
			local(
				&lattice(5, true)
					.into_iter()
					.map(|[x, y, z]| {
						let length = (x * x + y * y + z * z).sqrt();
						[x / length, y / length, z / length]
					})
					.collect::<Vec<_>>(),
			),
		));

		for (name, points) in clouds {
			let hull = build_convex_hull(&points).expect("the points enclose a volume");
			let margin = hull.margin();
			assert!(margin > 0.0 && hull.core_vertices().len() >= 4, "{name}");

			// The core made thicker by the margin reaches no further than the hull in any direction. It falls short
			// of the hull only toward a corner or an edge, by a few margins where those are pointed.
			for direction in directions(200) {
				let reach = along(direction, hull.support_point(direction));
				let rounded = along(direction, hull.core_support(direction)) + margin;
				assert!(
					rounded <= reach + 1e-5,
					"{name} along {direction:?}: {rounded} is past {reach}"
				);
				assert!(
					rounded >= reach - 10.0 * margin,
					"{name} along {direction:?}: {rounded} is far short of {reach}"
				);
			}
		}
	}

	#[test]
	fn repeated_points_make_one_corner() {
		let corners = lattice(2, false);
		let points = local(&[corners.clone(), corners.clone(), corners].concat());

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), 8);
		assert_encloses(&hull, &points, 0.0);
	}

	#[test]
	fn a_thin_plate_is_a_solid() {
		let points: Vec<_> = lattice(2, false)
			.into_iter()
			.map(|[x, y, z]| Point::new(x as f32, y as f32 * 1e-3, z as f32))
			.collect();

		let hull = build_convex_hull(&points).expect("the points enclose a volume");

		assert_eq!(hull.vertices().len(), 8);
		assert_encloses(&hull, &points, 0.0);
	}

	#[test]
	fn points_without_volume_have_no_hull() {
		assert!(build_convex_hull(&[]).is_none());
		assert!(build_convex_hull(&local(&[[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]])).is_none());
		assert!(build_convex_hull(&[Point::new(1.0, 2.0, 3.0); 8]).is_none());

		let line: Vec<_> = steps(9).map(|x| place([x, 0.0, 0.0], 1.0, [0.0; 3])).collect();
		assert!(build_convex_hull(&line).is_none());

		// Turned, the points of the plane are only on it to within rounding.
		let plane: Vec<_> = steps(9)
			.flat_map(|x| steps(9).map(move |y| place([x, y, 0.0], 1.0, [3.0, 2.0, 1.0])))
			.collect();
		assert!(build_convex_hull(&plane).is_none());
	}

	#[test]
	fn points_that_are_not_finite_have_no_hull() {
		let mut points = local(&lattice(2, false));
		points.push(Point::new(f32::NAN, 0.0, 0.0));
		assert!(build_convex_hull(&points).is_none());

		points.pop();
		points.push(Point::new(0.0, f32::INFINITY, 0.0));
		assert!(build_convex_hull(&points).is_none());
	}
}
