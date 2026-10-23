//! Navigation meshes built from walkable and obstacle polygons.

impl<Space> NavigationMesh<Space> {
	/// Builds a navigation mesh that covers the walkable polygons minus the obstacle polygons, on XZ.
	///
	/// Use it when walkable surfaces and the obstacles on them are authored as separate shapes, as
	/// [`Navigation`](super::Navigation) does with each [`NavigationArea`](super::NavigationArea). Every polygon must
	/// be convex on XZ, in either winding; polygons without area are ignored. Where walkable polygons overlap, the
	/// earlier one keeps the overlap, and every vertex keeps the height of the walkable polygon it came from. Obstacles
	/// cut at every height. Polygons that touch along an edge connect. Call [`Self::find_path`] next.
	pub fn from_areas(
		walkable: &[impl AsRef<[Point<Space>]>],
		obstacles: &[impl AsRef<[Point<Space>]>],
	) -> Result<Self, NavigationMeshBuildError> {
		let walkable: Vec<_> = walkable.iter().filter_map(|area| counterclockwise(area.as_ref())).collect();
		let obstacles: Vec<_> = obstacles.iter().filter_map(|area| counterclockwise(area.as_ref())).collect();
		let mut pieces = Vec::new();
		let mut current = Vec::new();
		let mut next = Vec::new();

		for (index, area) in walkable.iter().enumerate() {
			current.push(area.clone());
			// Cutting earlier walkable polygons out keeps pieces from overlapping, which the mesh requires.
			for cutter in walkable[..index].iter().chain(&obstacles) {
				for piece in current.drain(..) {
					subtract_convex(piece, cutter, &mut next);
				}
				std::mem::swap(&mut current, &mut next);
			}
			pieces.append(&mut current);
		}

		let (vertices, polygons) = weld(&pieces);
		let polygons = split_at_junctions(&vertices, polygons);
		Self::new(vertices, polygons)
	}
}

/// Returns `polygon` counterclockwise on XZ, or [`None`] when it has no area to walk on or cut.
fn counterclockwise<Space>(polygon: &[Point<Space>]) -> Option<Vec<Point<Space>>> {
	let area = polygon_area_xz(polygon);
	if area.is_nan() || area.abs() <= NAVIGATION_TOLERANCE * NAVIGATION_TOLERANCE {
		return None;
	}

	let mut polygon = polygon.to_vec();
	if area < 0.0 {
		polygon.reverse();
	}
	Some(polygon)
}

/// Pushes the convex pieces of `piece` that lie outside the convex, counterclockwise `cutter` into `output`.
///
/// Each cutter edge splits off the part of the piece outside it, and what is inside every edge is the overlap the
/// cutter removes. A piece the cutter doesn't overlap is pushed whole.
fn subtract_convex<Space>(piece: Vec<Point<Space>>, cutter: &[Point<Space>], output: &mut Vec<Vec<Point<Space>>>) {
	let first_output = output.len();
	let mut inside = piece.clone();
	let mut left = Vec::with_capacity(inside.len() + 1);
	let mut right = Vec::with_capacity(inside.len() + 1);

	for (index, &start) in cutter.iter().enumerate() {
		let end = cutter[(index + 1) % cutter.len()];
		// The cutter is counterclockwise, so its inside is left of every edge.
		split_convex_polygon_xz(&inside, start, end, NAVIGATION_TOLERANCE, &mut left, &mut right);
		if !right.is_empty() {
			output.push(std::mem::take(&mut right));
		}
		std::mem::swap(&mut inside, &mut left);

		if inside.is_empty() {
			// Nothing of the piece is inside the cutter, so keep it whole instead of in fragments.
			output.truncate(first_output);
			output.push(piece);
			return;
		}
	}
}

/// Merges the pieces' vertices that lie within [`NAVIGATION_TOLERANCE`] of each other and indexes the pieces by them.
///
/// Welding can fold a short edge into one vertex, so repeats are removed, and pieces that collapse below a triangle
/// are slivers and are dropped.
fn weld<Space>(pieces: &[Vec<Point<Space>>]) -> (Vec<Point<Space>>, Vec<Vec<NavigationVertexHandle>>) {
	let mut vertices = Vec::new();
	let mut polygons = Vec::with_capacity(pieces.len());

	for piece in pieces {
		let mut polygon = Vec::with_capacity(piece.len());
		for &point in piece {
			// A linear search keeps welding simple; meshes rebuild only when their areas change.
			let vertex = vertices
				.iter()
				.position(|&vertex| distance_xz(vertex, point) <= NAVIGATION_TOLERANCE)
				.unwrap_or_else(|| {
					vertices.push(point);
					vertices.len() - 1
				}) as NavigationVertexHandle;
			if !polygon.contains(&vertex) {
				polygon.push(vertex);
			}
		}
		if polygon.len() >= 3 {
			polygons.push(polygon);
		}
	}

	(vertices, polygons)
}

/// Inserts into each polygon edge every vertex that lies on it, in order along the edge.
///
/// A vertex in the middle of a neighbor's edge is a T-junction: the two polygons touch but share no edge, so the
/// mesh wouldn't connect them. Splitting the longer edge at the vertex gives both polygons the same edge.
fn split_at_junctions<Space>(
	vertices: &[Point<Space>],
	polygons: Vec<Vec<NavigationVertexHandle>>,
) -> Vec<Vec<NavigationVertexHandle>> {
	let mut on_edge: Vec<(f32, NavigationVertexHandle)> = Vec::new();

	polygons
		.into_iter()
		.map(|polygon| {
			let mut split = Vec::with_capacity(polygon.len());
			for (index, &from) in polygon.iter().enumerate() {
				let to = polygon[(index + 1) % polygon.len()];
				let (start, end) = (vertices[from as usize], vertices[to as usize]);
				split.push(from);

				on_edge.clear();
				on_edge.extend(vertices.iter().enumerate().filter_map(|(vertex, &point)| {
					let vertex = vertex as NavigationVertexHandle;
					let fraction = line_fraction_xz(point, start, end);
					let on_line = signed_distance_to_line_xz(point, start, end).abs() <= NAVIGATION_TOLERANCE;
					// Welded vertices are farther apart than the tolerance, so a vertex strictly between the ends is
					// not one of them.
					let inside = fraction > 0.0 && fraction < 1.0;
					(on_line && inside && !polygon.contains(&vertex)).then_some((fraction, vertex))
				}));
				on_edge.sort_unstable_by(|(first, _), (second, _)| first.total_cmp(second));
				split.extend(on_edge.iter().map(|&(_, vertex)| vertex));
			}
			split
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn point(x: f32, z: f32) -> Point {
		Point::new(x, 0.0, z)
	}

	/// Returns the XZ rectangle between two corners, counterclockwise.
	fn rectangle([minimum_x, minimum_z]: [f32; 2], [maximum_x, maximum_z]: [f32; 2]) -> [Point; 4] {
		[
			point(minimum_x, minimum_z),
			point(maximum_x, minimum_z),
			point(maximum_x, maximum_z),
			point(minimum_x, maximum_z),
		]
	}

	const NONE: &[[Point; 4]] = &[];

	#[test]
	fn an_obstacle_cuts_a_hole_that_paths_go_around() {
		let mesh = NavigationMesh::from_areas(&[rectangle([-5.0, -5.0], [5.0, 5.0])], &[rectangle([-1.0, -1.0], [1.0, 1.0])])
			.expect("a floor with a hole");

		assert_eq!(mesh.locate_polygon(point(0.0, 0.0)), None);
		assert_eq!(mesh.locate_polygon(point(0.9, -0.9)), None);
		assert!(mesh.locate_polygon(point(1.1, 0.0)).is_some());
		assert!(mesh.locate_polygon(point(-4.9, 4.9)).is_some());

		let path = mesh.find_path(point(-3.0, 0.0), point(3.0, 0.0)).expect("a way around");
		assert_eq!(path.first(), Some(&point(-3.0, 0.0)));
		assert_eq!(path.last(), Some(&point(3.0, 0.0)));
		// The straight line crosses the hole, so the path bends at its corners.
		assert!(path.len() > 2);
		assert!(path.iter().all(|corner| corner.x().abs() >= 1.0 || corner.z().abs() >= 1.0));
	}

	#[test]
	fn a_rotated_obstacle_cuts_its_own_footprint() {
		let diamond = [point(0.0, -1.0), point(1.0, 0.0), point(0.0, 1.0), point(-1.0, 0.0)];
		let mesh = NavigationMesh::from_areas(&[rectangle([-5.0, -5.0], [5.0, 5.0])], &[diamond]).expect("a floor");

		assert_eq!(mesh.locate_polygon(point(0.4, 0.4)), None);
		assert!(mesh.locate_polygon(point(0.9, 0.9)).is_some());
	}

	#[test]
	fn an_obstacle_crossing_a_walkable_edge_cuts_only_the_inside_and_can_split_it() {
		// A wall across the whole floor leaves two halves that no path connects.
		let mesh = NavigationMesh::from_areas(&[rectangle([0.0, 0.0], [10.0, 4.0])], &[rectangle([4.0, -1.0], [6.0, 5.0])])
			.expect("a floor split by a wall");

		assert_eq!(mesh.locate_polygon(point(5.0, 2.0)), None);
		assert_eq!(
			mesh.find_path(point(1.0, 2.0), point(9.0, 2.0)),
			Err(NavigationPathError::Unreachable)
		);
	}

	#[test]
	fn walkable_areas_that_touch_connect_even_along_unequal_edges() {
		// The big area's right edge meets two smaller areas' left edges, a T-junction at their shared corner.
		let mesh = NavigationMesh::from_areas(
			&[
				rectangle([0.0, 0.0], [4.0, 4.0]),
				rectangle([4.0, 0.0], [6.0, 2.0]),
				rectangle([4.0, 2.0], [6.0, 4.0]),
			],
			NONE,
		)
		.expect("three touching areas");

		assert!(mesh.find_path(point(1.0, 1.0), point(5.0, 1.0)).is_ok());
		assert!(mesh.find_path(point(1.0, 3.0), point(5.0, 3.0)).is_ok());
		assert!(mesh.find_path(point(5.0, 1.0), point(5.0, 3.0)).is_ok());
	}

	#[test]
	fn a_turned_obstacle_across_a_seam_leaves_both_sides_connected_around_it() {
		// Turned corners land off the axis-aligned grid, so the seam's split edges are collinear only within rounding.
		let (sine, cosine) = 0.5f32.sin_cos();
		let turned = [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]]
			.map(|[x, z]| point(4.0 + x * cosine - z * sine, 2.0 + x * sine + z * cosine));
		let mesh = NavigationMesh::from_areas(
			&[rectangle([0.0, 0.0], [4.0, 4.0]), rectangle([4.0, 0.0], [8.0, 4.0])],
			&[turned],
		)
		.expect("two areas with a turned obstacle on their seam");

		assert_eq!(mesh.locate_polygon(point(4.0, 2.0)), None);
		assert!(mesh.find_path(point(1.0, 2.0), point(7.0, 2.0)).is_ok());
		assert!(mesh.find_path(point(4.0, 0.2), point(4.0, 3.8)).is_ok());
	}

	#[test]
	fn the_earlier_walkable_area_keeps_an_overlap_and_its_height() {
		let low = rectangle([0.0, 0.0], [4.0, 4.0]);
		let high = rectangle([2.0, 2.0], [6.0, 6.0]).map(|corner| Point::new(corner.x(), 1.0, corner.z()));
		let mesh = NavigationMesh::from_areas(&[low, high], NONE).expect("overlapping areas");

		assert_eq!(mesh.project_point(point(3.0, 3.0)).map(|point| point.y()), Some(0.0));
		assert!(mesh.locate_polygon(point(5.0, 5.0)).is_some());
		assert!(mesh.find_path(point(1.0, 1.0), point(5.0, 5.0)).is_ok());
	}

	/// The third_person scene: a 100 m floor, two walls, a ledge, and a cube, each grown by the clearance.
	fn scene_mesh() -> NavigationMesh {
		let block = |x: f32, z: f32, half_x: f32, half_z: f32| {
			rectangle([x - half_x - 0.63, z - half_z - 0.63], [x + half_x + 0.63, z + half_z + 0.63])
		};
		NavigationMesh::from_areas(
			&[rectangle([-50.0, -50.0], [50.0, 50.0])],
			&[
				block(6.0, -12.0, 0.25, 6.0),
				block(6.0, 12.0, 0.25, 6.0),
				block(-10.0, 0.0, 0.25, 20.0),
				block(2.0, -4.0, 0.5, 0.5),
			],
		)
		.expect("the scene")
	}

	#[test]
	fn a_path_across_open_floor_is_the_straight_line_however_the_floor_is_split() {
		let mesh = scene_mesh();
		let length = |path: &[Point]| path.windows(2).map(|pair| distance_xz(pair[0], pair[1])).sum::<f32>();

		// The strip between the ledge and the walls is clear end to end, from either direction, though the floor is
		// split there into pieces whose centers lie far from the way across.
		for (start, target) in [
			(point(-5.0, 30.0), point(-5.0, -30.0)),
			(point(-5.0, -30.0), point(-5.0, 30.0)),
			(point(-8.0, 25.0), point(-8.0, -25.0)),
		] {
			let path = mesh.find_path(start, target).expect("a path");
			let found = length(&path);
			let straight = distance_xz(start, target);
			assert!(
				(found - straight).abs() < 0.01,
				"the path is {found:.1} m for {straight:.1} m straight: {path:?}"
			);
		}
		// The ledge blocks the way along z = 0, so that path goes around its north end.
		let path = mesh.find_path(point(-20.0, 0.0), point(20.0, 0.0)).expect("a path");
		assert!((length(&path) - 63.2).abs() < 0.1);
	}

	#[test]
	fn no_walkable_area_gives_an_empty_mesh() {
		let mesh = NavigationMesh::from_areas(NONE, &[rectangle([0.0, 0.0], [1.0, 1.0])]).expect("an empty mesh");

		assert_eq!(mesh.polygon_count(), 0);
		assert_eq!(mesh.locate_polygon(point(0.5, 0.5)), None);
	}
}

use math::{Point, distance_xz, line_fraction_xz, polygon_area_xz, signed_distance_to_line_xz, split_convex_polygon_xz};

#[cfg(test)]
use super::NavigationPathError;
use super::{NAVIGATION_TOLERANCE, NavigationMesh, NavigationMeshBuildError, NavigationVertexHandle};
