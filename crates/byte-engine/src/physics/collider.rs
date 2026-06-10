/// The `Collider` trait allows an entity to present itself as a collision shape.
pub trait Collider: Positionable {
	/// Returns the shape of the collider.
	fn shape(&self) -> Shapes;

	/// Returns the elasticity of the body.
	///
	/// Implement this method to provide a custom elasticity value for the collider.
	fn elasticity(&self) -> f32 {
		0.1f32
	}

	/// Returns the friction of the body.
	///
	/// Implement this method to provide a custom friction value for the collider.
	fn friction(&self) -> f32 {
		0.1f32
	}
}

/// The `CollisionShapes` enum represents the different shapes that a collider can have.
#[derive(Debug, Clone)]
pub enum Shapes {
	/// A sphere shaped collider.
	Sphere {
		/// The radius of the sphere.
		radius: f32,
	},
	/// A cube shaped collider.
	Cube {
		/// The half-size of the cube
		size: Vector3,
	},
	ConvexHull {
		points: Box<[Vector3]>,
		bounds: Bounds,
	},
}

impl Shapes {
	/// Creates a new sphere shaped collider.
	/// The radius parameter is the radius of the sphere.
	pub fn sphere(radius: f32) -> Self {
		Self::Sphere { radius }
	}

	/// Creates a new cube shaped collider.
	/// The size parameter is the half-size of the cube.
	pub fn cube(size: Vector3) -> Self {
		Self::Cube { size }
	}

	pub fn convex_hull(points: Box<[Vector3]>) -> Self {
		let mut bounds = points
			.first()
			.map(|point| Bounds::new(*point, *point))
			.unwrap_or_else(Bounds::zero);
		bounds.expand_to_fit_points(&points);

		Self::ConvexHull { points, bounds }
	}

	pub fn support_point(&self, direction: Vector3) -> Vector3 {
		match self {
			Self::Sphere { radius } => normalize_or_zero(direction) * *radius,
			Self::Cube { size: half_size } => {
				let half_size = Vector3::new(half_size.x.abs(), half_size.y.abs(), half_size.z.abs());
				let x = support_axis(half_size.x, direction.x);
				let y = support_axis(half_size.y, direction.y);
				let z = support_axis(half_size.z, direction.z);

				Vector3::new(x, y, z)
			}
			Self::ConvexHull { points, .. } => {
				furthest_point_in_direction(points.iter().copied(), direction).unwrap_or_else(Vector3::zero)
			}
		}
	}

	pub fn fastest_linear_speed(&self, angular_velocity: Vector3, direction: Vector3) -> f32 {
		match self {
			Self::Sphere { radius } => {
				let angular_speed = length(angular_velocity);
				angular_speed * *radius
			}
			Self::Cube { size: half_size } => {
				let points = [
					Vector3::new(half_size.x, half_size.y, half_size.z),
					Vector3::new(-half_size.x, half_size.y, half_size.z),
					Vector3::new(half_size.x, -half_size.y, half_size.z),
					Vector3::new(-half_size.x, -half_size.y, half_size.z),
					Vector3::new(half_size.x, half_size.y, -half_size.z),
					Vector3::new(-half_size.x, half_size.y, -half_size.z),
					Vector3::new(half_size.x, -half_size.y, -half_size.z),
					Vector3::new(-half_size.x, -half_size.y, -half_size.z),
				];

				highest_point_speed(points.into_iter(), angular_velocity, direction).unwrap_or(0f32)
			}
			Self::ConvexHull { points, .. } => {
				highest_point_speed(points.iter().copied(), angular_velocity, direction).unwrap_or(0f32)
			}
		}
	}

	pub fn inertia_tensor(&self) -> Matrix3 {
		match self {
			Self::Sphere { radius } => {
				let inertia = (2.0 / 5.0) * radius * radius;
				Matrix3::new(inertia, 0.0, 0.0, 0.0, inertia, 0.0, 0.0, 0.0, inertia)
			}
			Self::Cube { size: half_size } => {
				let half_size = Vector3::new(half_size.x.abs(), half_size.y.abs(), half_size.z.abs());
				let max = half_size;
				let min = -half_size;

				let dx = max.x - min.x;
				let dy = max.y - min.y;
				let dz = max.z - min.z;

				let dx2 = dx * dx;
				let dy2 = dy * dy;
				let dz2 = dz * dz;

				let tensor = Matrix3::new(
					(dy2 + dz2) / 12f32,
					0.0,
					0.0,
					0.0,
					(dx2 + dz2) / 12f32,
					0.0,
					0.0,
					0.0,
					(dx2 + dy2) / 12f32,
				);

				let cm = Vector3::new((max.x + min.x) * 0.5, (max.y + min.y) * 0.5, (max.z + min.z) * 0.5);

				let r = Vector3::zero() - cm;
				let r2 = magnitude_squared(r);

				let rx2 = r.x * r.x;
				let ry2 = r.y * r.y;
				let rz2 = r.z * r.z;

				let pat_tensor = Matrix3::new(
					r2 - rx2,
					r.x * r.y,
					r.x * r.x,
					r.y * r.x,
					r2 - r.y * r.y,
					r.y * r.z,
					r.z * r.x,
					r.z * r.y,
					r2 - rz2,
				);

				let inertia = Matrix3::new(
					tensor[0] + pat_tensor[0],
					tensor[1] + pat_tensor[1],
					tensor[2] + pat_tensor[2],
					pat_tensor[3],
					tensor[4] + pat_tensor[4],
					pat_tensor[5],
					pat_tensor[6],
					pat_tensor[7],
					tensor[8] + pat_tensor[8],
				);

				inertia
			}
			Self::ConvexHull { bounds, .. } => Shapes::cube(bounds.size() * 0.5).inertia_tensor(),
		}
	}

	pub fn bounds(&self) -> Bounds {
		match self {
			Self::Sphere { radius } => Bounds::new(Vector3::from(-*radius), Vector3::from(*radius)),
			Self::Cube { size: half_size } => Bounds::new(-*half_size, *half_size),
			Self::ConvexHull { bounds, .. } => *bounds,
		}
	}
}

fn support_axis(half_size: f32, direction: f32) -> f32 {
	if direction > 0.0 {
		half_size
	} else if direction < 0.0 {
		-half_size
	} else {
		0.0
	}
}

fn normalize_or_zero(vector: Vector3) -> Vector3 {
	let magnitude_squared = magnitude_squared(vector);

	if magnitude_squared > f32::EPSILON {
		vector / magnitude_squared.sqrt()
	} else {
		Vector3::zero()
	}
}

pub fn highest_point_speed(
	points: impl Iterator<Item = Vector3>,
	angular_velocity: Vector3,
	direction: Vector3,
) -> Option<f32> {
	points
		.map(|e| {
			let linear_velocity = cross(angular_velocity, e);
			let speed = dot(direction, linear_velocity);

			speed
		})
		.max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
}

/// Finds the point furthest from the given line along the given direction.
///
/// Returns `None` if the iterator is empty.
pub fn find_furthest_point_in_direction(points: impl Iterator<Item = (usize, Vector3)>, direction: Vector3) -> Option<usize> {
	points
		.max_by(|&(_, a), &(_, b)| {
			let da = dot(a, direction);
			let db = dot(b, direction);
			da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
		})
		.map(|(i, _)| i)
}

/// Finds the point furthest from the given line along the given direction.
///
/// Returns `None` if the iterator is empty.
pub fn furthest_point_in_direction(points: impl Iterator<Item = Vector3>, direction: Vector3) -> Option<Vector3> {
	points.max_by(|&a, &b| {
		let da = dot(a, direction);
		let db = dot(b, direction);
		da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
	})
}

pub fn distance_from_line(a: Vector3, b: Vector3, pt: Vector3) -> f32 {
	let ab = normalize_or_zero(b - a);

	if ab == Vector3::zero() {
		return magnitude(pt - a);
	}

	let ray = pt - a;
	let projection = ab * dot(ray, ab);
	let perpendicular = ray - projection;

	magnitude(perpendicular)
}

/// Finds the point furthest from the given line along the given direction.
///
/// Returns `None` if the iterator is empty.
pub fn find_point_furthest_from_line(points: impl Iterator<Item = Vector3>, a: Vector3, b: Vector3) -> Option<Vector3> {
	points.max_by(|&pa, &pb| {
		distance_from_line(a, b, pa)
			.partial_cmp(&distance_from_line(a, b, pb))
			.unwrap_or(std::cmp::Ordering::Equal)
	})
}

pub fn distance_from_triangle(a: Vector3, b: Vector3, c: Vector3, pt: Vector3) -> f32 {
	signed_distance_from_triangle(a, b, c, pt).abs()
}

fn signed_distance_from_triangle(a: Vector3, b: Vector3, c: Vector3, pt: Vector3) -> f32 {
	let ab = b - a;
	let ac = c - a;
	let normal = normalize_or_zero(cross(ab, ac));

	if normal == Vector3::zero() {
		let ab_length = magnitude_squared(ab);
		let ac_length = magnitude_squared(ac);
		let bc_length = magnitude_squared(c - b);

		// Degenerate triangles collapse to their longest edge, or to a point if all vertices match.
		if ab_length >= ac_length && ab_length >= bc_length {
			return distance_from_line(a, b, pt);
		} else if ac_length >= bc_length {
			return distance_from_line(a, c, pt);
		} else {
			return distance_from_line(b, c, pt);
		}
	}

	let ray = pt - a;
	let distance = dot(ray, normal);

	distance
}

/// Finds the point furthest from the given triangle along the given direction.
///
/// Returns `None` if the iterator is empty.
pub fn find_point_furthest_from_triangle(
	points: impl Iterator<Item = Vector3>,
	a: Vector3,
	b: Vector3,
	c: Vector3,
) -> Option<Vector3> {
	points.max_by(|&pa, &pb| {
		distance_from_triangle(a, b, c, pa)
			.partial_cmp(&distance_from_triangle(a, b, c, pb))
			.unwrap_or(std::cmp::Ordering::Equal)
	})
}

/// Builds a stable initial tetrahedron from a point cloud.
///
/// This function checks for "sanity", therefore making slower for cases where we already know the input is invalid.
/// "unsafe" functions will be built in the future to accelerate this process.
///
/// Returns `None` when the input cannot form a non-degenerate volume.
pub fn build_tetrahedron(verts: impl Iterator<Item = Vector3> + Clone) -> Option<(Vec<Vector3>, Vec<(usize, usize, usize)>)> {
	let a = furthest_point_in_direction(verts.clone(), Vector3::new(1.0, 0.0, 0.0))?;
	let b = furthest_point_in_direction(verts.clone(), Vector3::new(-1.0, 0.0, 0.0))?;

	if magnitude_squared(b - a) <= f32::EPSILON {
		return None;
	}

	let c = find_point_furthest_from_line(verts.clone(), a, b)?;

	if distance_from_line(a, b, c) <= f32::EPSILON {
		return None;
	}

	let d = find_point_furthest_from_triangle(verts.clone(), a, b, c)?;
	let distance = signed_distance_from_triangle(a, b, c, d);

	if distance.abs() <= f32::EPSILON {
		return None;
	}

	// Make sure the order is CCW
	let (a, b) = if distance > 0.0 { (b, a) } else { (a, b) };

	let mut tetrahedron_vertices = Vec::with_capacity(4);
	let mut tetrahedron_triangles = Vec::with_capacity(4);

	tetrahedron_vertices.push(a);
	tetrahedron_vertices.push(b);
	tetrahedron_vertices.push(c);
	tetrahedron_vertices.push(d);

	tetrahedron_triangles.push((0, 1, 2));
	tetrahedron_triangles.push((0, 2, 3));
	tetrahedron_triangles.push((2, 1, 3));
	tetrahedron_triangles.push((1, 0, 3));

	Some((tetrahedron_vertices, tetrahedron_triangles))
}

pub fn expand_convex_hull(
	hull_vertices: &mut Vec<Vector3>,
	hull_triangles: &mut Vec<(usize, usize, usize)>,
	vertices: &[Vector3],
) {
	let mut external_vertices = vertices.to_vec();

	remove_internal_points(hull_vertices, hull_triangles, &mut external_vertices);

	while !external_vertices.is_empty() {
		let idx = find_furthest_point_in_direction(
			external_vertices.iter().enumerate().map(|(i, e)| (i, *e)),
			external_vertices[0],
		)
		.unwrap();

		let pt = external_vertices[idx];

		external_vertices.remove(idx);

		add_point_to_hull(hull_vertices, hull_triangles, pt);

		remove_internal_points(hull_vertices, hull_triangles, &mut external_vertices);
	}

	remove_unreferenced_vertices(hull_vertices, hull_triangles);
}

/// Retains only points lying outside at least one hull face.
pub fn remove_internal_points(
	hull_vertices: &[Vector3],
	hull_triangles: &[(usize, usize, usize)],
	check_points: &mut Vec<Vector3>,
) {
	check_points.retain(|&pt| {
		for &(a, b, c) in hull_triangles {
			let (a, b, c) = (hull_vertices[a], hull_vertices[b], hull_vertices[c]);

			let distance = signed_distance_from_triangle(a, b, c, pt);

			if distance > 0.0 {
				return true;
			}
		}

		// TODO: cull points close to the surface

		false
	});
}

/// Checks whether an edge belongs to only one triangle in a selected set.
pub fn is_edge_unique(
	triangles: &[(usize, usize, usize)],
	facing_tris: &[usize],
	ignore_tri: usize,
	edge: (usize, usize),
) -> bool {
	let reverse_edge = (edge.1, edge.0);

	for &tri_idx in facing_tris {
		if tri_idx == ignore_tri {
			continue;
		}

		let (a, b, c) = triangles[tri_idx];

		let ab = (a, b);
		let bc = (b, c);
		let ca = (c, a);

		// Adjacent consistently wound triangles traverse their shared edge in opposite directions.
		if ab == edge || bc == edge || ca == edge || ab == reverse_edge || bc == reverse_edge || ca == reverse_edge {
			return false;
		}
	}

	true
}

/// Expands a convex hull to include an external point.
pub fn add_point_to_hull(hull_vertices: &mut Vec<Vector3>, hull_triangles: &mut Vec<(usize, usize, usize)>, point: Vector3) {
	let facing_tris: Vec<usize> = hull_triangles
		.iter()
		.enumerate()
		.rev()
		.filter_map(|(idx, &(a, b, c))| {
			let a = hull_vertices[a];
			let b = hull_vertices[b];
			let c = hull_vertices[c];

			if signed_distance_from_triangle(a, b, c, point) > 0.0 {
				Some(idx)
			} else {
				None
			}
		})
		.collect();

	let unique_edges: Vec<(usize, usize)> = facing_tris
		.iter()
		.map(|&idx| {
			let (a, b, c) = hull_triangles[idx];

			let ab = (a, b);
			let bc = (b, c);
			let ca = (c, a);

			[
				is_edge_unique(&hull_triangles, &facing_tris, idx, ab).then_some(ab),
				is_edge_unique(&hull_triangles, &facing_tris, idx, bc).then_some(bc),
				is_edge_unique(&hull_triangles, &facing_tris, idx, ca).then_some(ca),
			]
			.into_iter()
		})
		.flatten()
		.filter(Option::is_some)
		.map(Option::unwrap)
		.collect();

	// Remove old facing tris
	for &idx in &facing_tris {
		hull_triangles.remove(idx);
	}

	// Add new point
	let new_point_idx = hull_vertices.len();
	hull_vertices.push(point);

	// Add triangles for each unique edge
	for &(a, b) in &unique_edges {
		hull_triangles.push((a, b, new_point_idx));
	}
}

/// Removes vertices unused by the hull and remaps triangle indices in place.
pub fn remove_unreferenced_vertices(hull_vertices: &mut Vec<Vector3>, hull_triangles: &mut Vec<(usize, usize, usize)>) {
	let mut i = 0;

	hull_vertices.retain_mut(move |_| {
		for &(a, b, c) in hull_triangles.iter() {
			if a == i || b == i || c == i {
				i += 1;
				return true;
			}
		}

		// The next shifted vertex occupies the same index, so only triangle references move.
		for (a, b, c) in hull_triangles.iter_mut() {
			if *a > i {
				*a -= 1;
			}
			if *b > i {
				*b -= 1;
			}
			if *c > i {
				*c -= 1;
			}
		}

		false
	});
}

pub fn build_convex_hull(vertices: &[Vector3]) -> Option<(Vec<Vector3>, Vec<(usize, usize, usize)>)> {
	if vertices.len() < 4 {
		return None;
	}

	let tetrahedron = build_tetrahedron(vertices.iter().map(|v| *v))?;

	let mut hull_vertices = tetrahedron.0;
	let mut hull_triangles = tetrahedron.1;

	expand_convex_hull(&mut hull_vertices, &mut hull_triangles, vertices);

	Some((hull_vertices, hull_triangles))
}

use math::{cross, dot, length, magnitude, magnitude_squared, mat::MatNew3 as _, Base as _, Matrix3, Vector3};

use crate::{physics::bounds::Bounds, space::Positionable};

#[cfg(test)]
mod tests {
	use math::{assert_float_eq, assert_vec3f_near};

	use super::*;

	#[test]
	fn sphere_support_normalizes_direction_and_tolerates_zero_direction() {
		let sphere = Shapes::sphere(2.0);

		assert_vec3f_near!(sphere.support_point(Vector3::new(3.0, 0.0, 0.0)), Vector3::new(2.0, 0.0, 0.0));
		assert_vec3f_near!(sphere.support_point(Vector3::zero()), Vector3::zero());
	}

	#[test]
	fn cube_support_uses_axis_signs_and_tolerates_negative_half_size() {
		let cube = Shapes::cube(Vector3::new(-1.0, 2.0, 3.0));

		assert_vec3f_near!(cube.support_point(Vector3::new(1.0, -1.0, 0.0)), Vector3::new(1.0, -2.0, 0.0));
		assert_vec3f_near!(cube.support_point(Vector3::zero()), Vector3::zero());
	}

	#[test]
	fn convex_hull_support_and_bounds_tolerate_empty_and_single_point_hulls() {
		let empty = Shapes::convex_hull(Box::new([]));
		assert_vec3f_near!(empty.support_point(Vector3::new(1.0, 0.0, 0.0)), Vector3::zero());
		assert_vec3f_near!(empty.bounds().min(), Vector3::zero());
		assert_vec3f_near!(empty.bounds().max(), Vector3::zero());

		let point = Vector3::new(1.0, -2.0, 3.0);
		let hull = Shapes::convex_hull(Box::new([point]));
		assert_vec3f_near!(hull.support_point(Vector3::new(-1.0, 0.0, 0.0)), point);
		assert_vec3f_near!(hull.bounds().min(), point);
		assert_vec3f_near!(hull.bounds().max(), point);
	}

	#[test]
	fn highest_point_speed_returns_none_for_empty_iterators_and_handles_zero_motion() {
		assert!(highest_point_speed([].into_iter(), Vector3::zero(), Vector3::new(1.0, 0.0, 0.0)).is_none());

		let speed = highest_point_speed(
			[Vector3::new(0.0, 1.0, 0.0), Vector3::new(0.0, -2.0, 0.0)].into_iter(),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(1.0, 0.0, 0.0),
		)
		.unwrap();

		assert_float_eq!(speed, 2.0);
	}

	#[test]
	fn line_distance_tolerates_degenerate_lines() {
		let a = Vector3::new(1.0, 1.0, 1.0);
		let pt = Vector3::new(4.0, 5.0, 1.0);

		assert_float_eq!(distance_from_line(a, a, pt), 5.0);
		assert_float_eq!(
			distance_from_line(Vector3::zero(), Vector3::new(2.0, 0.0, 0.0), Vector3::new(1.0, 3.0, 0.0)),
			3.0
		);
	}

	#[test]
	fn triangle_distance_is_unsigned_and_tolerates_degenerate_triangles() {
		let a = Vector3::zero();
		let b = Vector3::new(1.0, 0.0, 0.0);
		let c = Vector3::new(0.0, 1.0, 0.0);

		assert_float_eq!(distance_from_triangle(a, b, c, Vector3::new(0.0, 0.0, 2.0)), 2.0);
		assert_float_eq!(distance_from_triangle(a, b, c, Vector3::new(0.0, 0.0, -2.0)), 2.0);
		assert_float_eq!(
			distance_from_triangle(a, b, Vector3::new(2.0, 0.0, 0.0), Vector3::new(1.0, 3.0, 0.0)),
			3.0
		);
		assert_float_eq!(distance_from_triangle(a, a, a, Vector3::new(0.0, 4.0, 3.0)), 5.0);
	}

	#[test]
	fn internal_point_removal_uses_oriented_hull_faces() {
		let vertices = vec![
			Vector3::zero(),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
		];
		let triangles = vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)];
		let outside = Vector3::new(0.2, 0.2, -1.0);
		let mut points = vec![Vector3::new(0.1, 0.1, 0.1), Vector3::new(0.2, 0.2, 0.0), outside];

		remove_internal_points(&vertices, &triangles, &mut points);

		assert_eq!(points, vec![outside]);
	}

	#[test]
	fn shared_edges_are_compared_without_direction() {
		let triangles = vec![(0, 1, 2), (2, 1, 3)];
		let facing_triangles = vec![0, 1];

		assert!(!is_edge_unique(&triangles, &facing_triangles, 0, (1, 2)));
		assert!(is_edge_unique(&triangles, &facing_triangles, 0, (0, 1)));
	}

	#[test]
	fn adding_point_replaces_only_facing_triangles() {
		let mut vertices = vec![
			Vector3::zero(),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
		];
		let mut triangles = vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)];

		add_point_to_hull(&mut vertices, &mut triangles, Vector3::new(0.2, 0.2, -1.0));

		assert_eq!(vertices.len(), 5);
		assert_eq!(triangles.len(), 6);
		assert!(!triangles.contains(&(0, 1, 2)));
	}

	#[test]
	fn unreferenced_vertex_removal_keeps_the_shifted_index() {
		let mut vertices = vec![
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::zero(),
			Vector3::new(2.0, 0.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
		];
		let mut triangles = vec![(1, 3, 4)];

		remove_unreferenced_vertices(&mut vertices, &mut triangles);

		assert_eq!(
			vertices,
			vec![Vector3::zero(), Vector3::new(0.0, 1.0, 0.0), Vector3::new(0.0, 0.0, 1.0)]
		);
		assert_eq!(triangles, vec![(0, 1, 2)]);
	}

	#[test]
	fn furthest_point_helpers_return_none_for_empty_iterators() {
		assert!(furthest_point_in_direction([].into_iter(), Vector3::new(1.0, 0.0, 0.0)).is_none());
		assert!(find_point_furthest_from_line([].into_iter(), Vector3::zero(), Vector3::new(1.0, 0.0, 0.0)).is_none());
		assert!(find_point_furthest_from_triangle(
			[].into_iter(),
			Vector3::zero(),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0)
		)
		.is_none());
	}

	#[test]
	fn build_tetrahedron_returns_non_degenerate_tetrahedron() {
		let points = [
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, 0.0, 1.0),
			Vector3::new(0.25, 0.25, 0.25),
		];

		let (verts, triangles) = build_tetrahedron(points.into_iter()).unwrap();

		assert_eq!(verts.len(), 4);
		assert_eq!(triangles, vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)]);
		assert!(triangles
			.iter()
			.all(|&(a, b, c)| a < verts.len() && b < verts.len() && c < verts.len()));

		let volume = dot(verts[3] - verts[0], cross(verts[1] - verts[0], verts[2] - verts[0])).abs() / 6.0;
		assert!(volume > f32::EPSILON);
	}

	#[test]
	fn build_tetrahedron_uses_opposite_x_extremes_instead_of_origin_relative_direction() {
		let points = [
			Vector3::new(10.0, 0.0, 0.0),
			Vector3::new(11.0, 0.0, 0.0),
			Vector3::new(10.0, 1.0, 0.0),
			Vector3::new(10.0, 0.0, 1.0),
		];

		let (verts, _) = build_tetrahedron(points.into_iter()).unwrap();
		let has_min_x = verts.iter().any(|point| point.x == 10.0);
		let has_max_x = verts.iter().any(|point| point.x == 11.0);

		assert!(has_min_x);
		assert!(has_max_x);
	}

	#[test]
	fn build_tetrahedron_rejects_degenerate_point_clouds() {
		assert!(build_tetrahedron([].into_iter()).is_none());
		assert!(build_tetrahedron([Vector3::new(1.0, 2.0, 3.0)].into_iter()).is_none());

		let identical = [Vector3::new(1.0, 1.0, 1.0); 4];
		assert!(build_tetrahedron(identical.into_iter()).is_none());

		let collinear = [
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(0.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(2.0, 0.0, 0.0),
		];
		assert!(build_tetrahedron(collinear.into_iter()).is_none());

		let coplanar = [
			Vector3::new(-1.0, 0.0, 0.0),
			Vector3::new(1.0, 0.0, 0.0),
			Vector3::new(0.0, 1.0, 0.0),
			Vector3::new(0.0, -1.0, 0.0),
			Vector3::new(0.5, 0.5, 0.0),
		];
		assert!(build_tetrahedron(coplanar.into_iter()).is_none());
	}

	#[test]
	fn convex_hull_builds_cube_and_discards_interior_points() {
		let points = [
			Vector3::new(-1.0, -1.0, -1.0),
			Vector3::new(-1.0, -1.0, 1.0),
			Vector3::new(-1.0, 1.0, -1.0),
			Vector3::new(-1.0, 1.0, 1.0),
			Vector3::new(1.0, -1.0, -1.0),
			Vector3::new(1.0, -1.0, 1.0),
			Vector3::new(1.0, 1.0, -1.0),
			Vector3::new(1.0, 1.0, 1.0),
			Vector3::zero(),
		];

		let (vertices, triangles) = build_convex_hull(&points).unwrap();

		assert_eq!(vertices.len(), 8);
		assert_eq!(triangles.len(), 12);
		assert!(!vertices.contains(&Vector3::zero()));
		assert!(triangles
			.iter()
			.all(|&(a, b, c)| a < vertices.len() && b < vertices.len() && c < vertices.len()));
	}
}
