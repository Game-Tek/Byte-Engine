use math::{Point, Vector, AABB};
use maths_rs::{mat::MatNew3 as _, Mat3f};

use crate::{physics::LocalSpace, space::Positionable};

/// The `Collider` trait provides collision geometry and material response to a physics world.
///
/// Physical entities usually implement this trait as part of [`crate::physics::Body`].
pub trait Collider: Positionable {
	/// Returns collider-local geometry.
	fn shape(&self) -> Shapes;

	/// Returns the collision elasticity.
	fn elasticity(&self) -> f32 {
		0.1
	}

	/// Returns the collision friction.
	fn friction(&self) -> f32 {
		0.1
	}
}

/// The `Shapes` enum selects the local-space geometry used for collision detection.
#[derive(Debug, Clone)]
pub enum Shapes {
	/// A spherical collider centered on the local origin.
	Sphere { radius: f32 },
	/// An axis-aligned box represented by local half-extents.
	Cube { size: Vector<LocalSpace> },
	/// A convex hull represented by local points and their bounds.
	ConvexHull {
		points: Box<[Point<LocalSpace>]>,
		bounds: AABB<LocalSpace>,
	},
}

impl Shapes {
	/// Creates a spherical collider with `radius`.
	pub fn sphere(radius: f32) -> Self {
		Self::Sphere { radius }
	}

	/// Creates a box collider with local half-extents.
	pub fn cube(size: Vector<LocalSpace>) -> Self {
		Self::Cube { size }
	}

	/// Creates a convex-hull collider from local points.
	pub fn convex_hull(points: Box<[Point<LocalSpace>]>) -> Self {
		let bounds: Option<AABB<LocalSpace>> = points.iter().copied().fold(None, |bounds, point| {
			Some(match bounds {
				Some(bounds) => AABB::new(
					Point::new(
						bounds.min().x().min(point.x()),
						bounds.min().y().min(point.y()),
						bounds.min().z().min(point.z()),
					),
					Point::new(
						bounds.max().x().max(point.x()),
						bounds.max().y().max(point.y()),
						bounds.max().z().max(point.z()),
					),
				),
				None => AABB::new(point, point),
			})
		});
		Self::ConvexHull {
			points,
			bounds: bounds.unwrap_or_else(|| AABB::new(Point::origin(), Point::origin())),
		}
	}

	/// Returns the furthest local point in `direction`.
	pub fn support_point(&self, direction: Vector<LocalSpace>) -> Point<LocalSpace> {
		match self {
			Self::Sphere { radius } => Point::origin() + normalize_or_zero(direction) * *radius,
			Self::Cube { size } => Point::new(
				support_axis(size.x().abs(), direction.x()),
				support_axis(size.y().abs(), direction.y()),
				support_axis(size.z().abs(), direction.z()),
			),
			Self::ConvexHull { points, .. } => {
				furthest_point_in_direction(points.iter().copied(), direction).unwrap_or_else(Point::origin)
			}
		}
	}

	/// Returns the maximum directional speed induced by local angular motion.
	pub fn fastest_linear_speed(&self, angular_velocity: Vector<LocalSpace>, direction: Vector<LocalSpace>) -> f32 {
		match self {
			Self::Sphere { radius } => angular_velocity.length() * *radius,
			Self::Cube { size } => {
				let x = size.x().abs();
				let y = size.y().abs();
				let z = size.z().abs();
				highest_point_speed(
					[
						Point::new(x, y, z),
						Point::new(-x, y, z),
						Point::new(x, -y, z),
						Point::new(-x, -y, z),
						Point::new(x, y, -z),
						Point::new(-x, y, -z),
						Point::new(x, -y, -z),
						Point::new(-x, -y, -z),
					]
					.into_iter(),
					angular_velocity,
					direction,
				)
				.unwrap_or(0.0)
			}
			Self::ConvexHull { points, .. } => {
				highest_point_speed(points.iter().copied(), angular_velocity, direction).unwrap_or(0.0)
			}
		}
	}

	/// Returns the raw local inertia tensor for a unit-mass collider.
	pub fn inertia_tensor(&self) -> Mat3f {
		let half_extents = match self {
			Self::Sphere { radius } => {
				let inertia = 0.4 * radius * radius;
				return Mat3f::new(inertia, 0.0, 0.0, 0.0, inertia, 0.0, 0.0, 0.0, inertia);
			}
			Self::Cube { size } => *size,
			Self::ConvexHull { bounds, .. } => bounds.half_extents(),
		};
		let x = 2.0 * half_extents.x().abs();
		let y = 2.0 * half_extents.y().abs();
		let z = 2.0 * half_extents.z().abs();
		Mat3f::new(
			(y * y + z * z) / 12.0,
			0.0,
			0.0,
			0.0,
			(x * x + z * z) / 12.0,
			0.0,
			0.0,
			0.0,
			(x * x + y * y) / 12.0,
		)
	}

	/// Returns local axis-aligned bounds for this shape.
	pub fn bounds(&self) -> AABB<LocalSpace> {
		match self {
			Self::Sphere { radius } => {
				AABB::from_center_and_half_extents(Point::origin(), Vector::new(*radius, *radius, *radius))
			}
			Self::Cube { size } => AABB::from_center_and_half_extents(Point::origin(), *size),
			Self::ConvexHull { bounds, .. } => *bounds,
		}
	}
}

fn support_axis(half_extent: f32, direction: f32) -> f32 {
	if direction > 0.0 {
		half_extent
	} else if direction < 0.0 {
		-half_extent
	} else {
		0.0
	}
}

fn normalize_or_zero(vector: Vector<LocalSpace>) -> Vector<LocalSpace> {
	vector
		.normalized()
		.map_or_else(|_| Vector::zero(), |direction| direction.into_vector())
}

/// Returns the largest speed projected along `direction` among local `points`.
pub fn highest_point_speed(
	points: impl Iterator<Item = Point<LocalSpace>>,
	angular_velocity: Vector<LocalSpace>,
	direction: Vector<LocalSpace>,
) -> Option<f32> {
	points
		.map(|point| direction.dot(angular_velocity.cross(point - Point::origin())))
		.max_by(|a, b| a.total_cmp(b))
}

/// Returns the index of the local point furthest in `direction`.
pub fn find_furthest_point_in_direction(
	points: impl Iterator<Item = (usize, Point<LocalSpace>)>,
	direction: Vector<LocalSpace>,
) -> Option<usize> {
	points
		.max_by(|(_, a), (_, b)| point_projection(*a, direction).total_cmp(&point_projection(*b, direction)))
		.map(|(index, _)| index)
}

/// Returns the local point furthest in `direction`.
pub fn furthest_point_in_direction(
	points: impl Iterator<Item = Point<LocalSpace>>,
	direction: Vector<LocalSpace>,
) -> Option<Point<LocalSpace>> {
	points.max_by(|a, b| point_projection(*a, direction).total_cmp(&point_projection(*b, direction)))
}

fn point_projection(point: Point<LocalSpace>, direction: Vector<LocalSpace>) -> f32 {
	direction.dot(point - Point::origin())
}

/// Returns the distance from `point` to the local line through `a` and `b`.
pub fn distance_from_line(a: Point<LocalSpace>, b: Point<LocalSpace>, point: Point<LocalSpace>) -> f32 {
	let offset = b - a;
	let Some(direction) = offset.normalized().ok() else {
		return point.distance_to(a);
	};
	let ray = point - a;
	(ray - direction * direction.dot(ray)).length()
}

/// Returns the local point furthest from the line through `a` and `b`.
pub fn find_point_furthest_from_line(
	points: impl Iterator<Item = Point<LocalSpace>>,
	a: Point<LocalSpace>,
	b: Point<LocalSpace>,
) -> Option<Point<LocalSpace>> {
	points.max_by(|a_point, b_point| distance_from_line(a, b, *a_point).total_cmp(&distance_from_line(a, b, *b_point)))
}

/// Returns the unsigned distance from `point` to the local triangle plane.
pub fn distance_from_triangle(
	a: Point<LocalSpace>,
	b: Point<LocalSpace>,
	c: Point<LocalSpace>,
	point: Point<LocalSpace>,
) -> f32 {
	signed_distance_from_triangle(a, b, c, point).abs()
}

fn signed_distance_from_triangle(
	a: Point<LocalSpace>,
	b: Point<LocalSpace>,
	c: Point<LocalSpace>,
	point: Point<LocalSpace>,
) -> f32 {
	let normal = (b - a).cross(c - a).normalized();
	match normal {
		Ok(normal) => normal.dot(point - a),
		Err(_) => distance_from_line(a, b, point)
			.max(distance_from_line(a, c, point))
			.max(distance_from_line(b, c, point)),
	}
}

/// Returns the local point furthest from the triangle plane through `a`, `b`, and `c`.
pub fn find_point_furthest_from_triangle(
	points: impl Iterator<Item = Point<LocalSpace>>,
	a: Point<LocalSpace>,
	b: Point<LocalSpace>,
	c: Point<LocalSpace>,
) -> Option<Point<LocalSpace>> {
	points.max_by(|a_point, b_point| {
		distance_from_triangle(a, b, c, *a_point).total_cmp(&distance_from_triangle(a, b, c, *b_point))
	})
}

/// Builds a stable initial tetrahedron from a local point cloud.
pub fn build_tetrahedron(
	vertices: impl Iterator<Item = Point<LocalSpace>> + Clone,
) -> Option<(Vec<Point<LocalSpace>>, Vec<(usize, usize, usize)>)> {
	let a = furthest_point_in_direction(vertices.clone(), Vector::new(1.0, 0.0, 0.0))?;
	let b = furthest_point_in_direction(vertices.clone(), Vector::new(-1.0, 0.0, 0.0))?;
	if (b - a).length_squared() <= f32::EPSILON {
		return None;
	}
	let c = find_point_furthest_from_line(vertices.clone(), a, b)?;
	if distance_from_line(a, b, c) <= f32::EPSILON {
		return None;
	}
	let d = find_point_furthest_from_triangle(vertices.clone(), a, b, c)?;
	let distance = signed_distance_from_triangle(a, b, c, d);
	if distance.abs() <= f32::EPSILON {
		return None;
	}
	let (a, b) = if distance > 0.0 { (b, a) } else { (a, b) };
	Some((vec![a, b, c, d], vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)]))
}

/// Expands a local convex hull until it encloses every external input point.
pub fn expand_convex_hull(
	hull_vertices: &mut Vec<Point<LocalSpace>>,
	hull_triangles: &mut Vec<(usize, usize, usize)>,
	vertices: &[Point<LocalSpace>],
) {
	let mut external_vertices = vertices.to_vec();

	remove_internal_points(hull_vertices, hull_triangles, &mut external_vertices);

	while !external_vertices.is_empty() {
		let index = find_furthest_point_in_direction(
			external_vertices.iter().enumerate().map(|(index, point)| (index, *point)),
			external_vertices[0] - Point::origin(),
		)
		.expect("an external vertex must exist while the candidate list is non-empty");
		let point = external_vertices.remove(index);

		add_point_to_hull(hull_vertices, hull_triangles, point);
		remove_internal_points(hull_vertices, hull_triangles, &mut external_vertices);
	}

	remove_unreferenced_vertices(hull_vertices, hull_triangles);
}

/// Retains candidate points that lie outside at least one oriented local hull face.
pub fn remove_internal_points(
	hull_vertices: &[Point<LocalSpace>],
	hull_triangles: &[(usize, usize, usize)],
	check_points: &mut Vec<Point<LocalSpace>>,
) {
	check_points.retain(|point| {
		for &(a, b, c) in hull_triangles {
			let distance = signed_distance_from_triangle(hull_vertices[a], hull_vertices[b], hull_vertices[c], *point);
			if distance > 0.0 {
				return true;
			}
		}

		false
	});
}

/// Returns whether `edge` belongs to only one triangle in the selected face set.
pub fn is_edge_unique(
	triangles: &[(usize, usize, usize)],
	facing_triangles: &[usize],
	ignore_triangle: usize,
	edge: (usize, usize),
) -> bool {
	let reverse_edge = (edge.1, edge.0);

	for &triangle_index in facing_triangles {
		if triangle_index == ignore_triangle {
			continue;
		}

		let (a, b, c) = triangles[triangle_index];
		let edges = [(a, b), (b, c), (c, a)];

		// Adjacent consistently wound faces traverse their shared edge in opposite directions.
		if edges
			.into_iter()
			.any(|candidate| candidate == edge || candidate == reverse_edge)
		{
			return false;
		}
	}

	true
}

/// Expands a local convex hull by replacing faces visible from `point` with horizon faces.
pub fn add_point_to_hull(
	hull_vertices: &mut Vec<Point<LocalSpace>>,
	hull_triangles: &mut Vec<(usize, usize, usize)>,
	point: Point<LocalSpace>,
) {
	let facing_triangles: Vec<usize> = hull_triangles
		.iter()
		.enumerate()
		.rev()
		.filter_map(|(index, &(a, b, c))| {
			(signed_distance_from_triangle(hull_vertices[a], hull_vertices[b], hull_vertices[c], point) > 0.0).then_some(index)
		})
		.collect();

	// The inner iterator outlives each `flat_map` call, so capture only shared references.
	let triangles = hull_triangles.as_slice();
	let facing = facing_triangles.as_slice();
	let unique_edges: Vec<(usize, usize)> = facing
		.iter()
		.flat_map(|&triangle_index| {
			let (a, b, c) = triangles[triangle_index];
			[(a, b), (b, c), (c, a)]
				.into_iter()
				.filter(move |&edge| is_edge_unique(triangles, facing, triangle_index, edge))
		})
		.collect();

	// Indices are collected in reverse order so removals cannot shift later faces.
	for triangle_index in facing_triangles {
		hull_triangles.remove(triangle_index);
	}

	let point_index = hull_vertices.len();
	hull_vertices.push(point);
	for (a, b) in unique_edges {
		hull_triangles.push((a, b, point_index));
	}
}

/// Removes unused local hull vertices and remaps triangle indices in place.
pub fn remove_unreferenced_vertices(
	hull_vertices: &mut Vec<Point<LocalSpace>>,
	hull_triangles: &mut Vec<(usize, usize, usize)>,
) {
	let mut index = 0;

	hull_vertices.retain_mut(|_| {
		if hull_triangles.iter().any(|&(a, b, c)| a == index || b == index || c == index) {
			index += 1;
			return true;
		}

		// The next shifted vertex occupies this index, so only later references move.
		for (a, b, c) in hull_triangles.iter_mut() {
			if *a > index {
				*a -= 1;
			}
			if *b > index {
				*b -= 1;
			}
			if *c > index {
				*c -= 1;
			}
		}

		false
	});
}

/// Builds a convex hull from local points.
pub fn build_convex_hull(vertices: &[Point<LocalSpace>]) -> Option<(Vec<Point<LocalSpace>>, Vec<(usize, usize, usize)>)> {
	if vertices.len() < 4 {
		return None;
	}

	let (mut hull_vertices, mut hull_triangles) = build_tetrahedron(vertices.iter().copied())?;
	expand_convex_hull(&mut hull_vertices, &mut hull_triangles, vertices);

	Some((hull_vertices, hull_triangles))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn support_points_keep_local_positions_distinct_from_directions() {
		let sphere = Shapes::sphere(2.0);
		assert_eq!(sphere.support_point(Vector::new(3.0, 0.0, 0.0)), Point::new(2.0, 0.0, 0.0));
		assert_eq!(sphere.support_point(Vector::zero()), Point::origin());
	}

	#[test]
	fn cube_bounds_preserve_half_extents() {
		let cube = Shapes::cube(Vector::new(1.0, 2.0, 3.0));
		assert_eq!(cube.bounds().min(), Point::new(-1.0, -2.0, -3.0));
		assert_eq!(cube.bounds().max(), Point::new(1.0, 2.0, 3.0));
	}

	#[test]
	fn tetrahedron_rejects_degenerate_clouds() {
		assert!(build_tetrahedron([Point::new(1.0, 2.0, 3.0); 4].into_iter()).is_none());
	}

	#[test]
	fn internal_point_removal_uses_oriented_hull_faces() {
		let vertices = vec![
			Point::origin(),
			Point::new(0.0, 1.0, 0.0),
			Point::new(1.0, 0.0, 0.0),
			Point::new(0.0, 0.0, 1.0),
		];
		let triangles = vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)];
		let outside = Point::new(0.2, 0.2, -1.0);
		let mut points = vec![Point::new(0.1, 0.1, 0.1), Point::new(0.2, 0.2, 0.0), outside];

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
			Point::origin(),
			Point::new(0.0, 1.0, 0.0),
			Point::new(1.0, 0.0, 0.0),
			Point::new(0.0, 0.0, 1.0),
		];
		let mut triangles = vec![(0, 1, 2), (0, 2, 3), (2, 1, 3), (1, 0, 3)];

		add_point_to_hull(&mut vertices, &mut triangles, Point::new(0.2, 0.2, -1.0));

		assert_eq!(vertices.len(), 5);
		assert_eq!(triangles.len(), 6);
		assert!(!triangles.contains(&(0, 1, 2)));
	}

	#[test]
	fn unreferenced_vertex_removal_keeps_shifted_indices_valid() {
		let mut vertices = vec![
			Point::new(-1.0, 0.0, 0.0),
			Point::origin(),
			Point::new(2.0, 0.0, 0.0),
			Point::new(0.0, 1.0, 0.0),
			Point::new(0.0, 0.0, 1.0),
		];
		let mut triangles = vec![(1, 3, 4)];

		remove_unreferenced_vertices(&mut vertices, &mut triangles);

		assert_eq!(
			vertices,
			vec![Point::origin(), Point::new(0.0, 1.0, 0.0), Point::new(0.0, 0.0, 1.0)]
		);
		assert_eq!(triangles, vec![(0, 1, 2)]);
	}

	#[test]
	fn convex_hull_builds_cube_and_discards_interior_points() {
		let points = [
			Point::new(-1.0, -1.0, -1.0),
			Point::new(-1.0, -1.0, 1.0),
			Point::new(-1.0, 1.0, -1.0),
			Point::new(-1.0, 1.0, 1.0),
			Point::new(1.0, -1.0, -1.0),
			Point::new(1.0, -1.0, 1.0),
			Point::new(1.0, 1.0, -1.0),
			Point::new(1.0, 1.0, 1.0),
			Point::origin(),
		];

		let (vertices, triangles) = build_convex_hull(&points).expect("cube points must form a hull");

		assert_eq!(vertices.len(), 8);
		assert_eq!(triangles.len(), 12);
		assert!(!vertices.contains(&Point::origin()));
		assert!(triangles
			.iter()
			.all(|&(a, b, c)| a < vertices.len() && b < vertices.len() && c < vertices.len()));
	}
}
