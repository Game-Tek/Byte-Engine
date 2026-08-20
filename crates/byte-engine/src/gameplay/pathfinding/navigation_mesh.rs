//! Indexed navigation meshes and XZ-plane string pulling.

/// A vertex index in a [`NavigationMesh`].
pub type NavigationVertexHandle = u32;

/// The `NavigationPortal` struct carries an oriented polygon opening through an XZ path corridor.
///
/// `left` and `right` are relative to travel through the corridor. Pass ordered portals to
/// [`string_pull`] after adding the start and target positions separately.
pub struct NavigationPortal<Space = WorldSpace> {
	/// The endpoint on the traveler's left.
	pub left: Point<Space>,
	/// The endpoint on the traveler's right.
	pub right: Point<Space>,
}

impl<Space> NavigationPortal<Space> {
	/// Creates a portal oriented relative to the path's travel direction.
	pub fn new(left: Point<Space>, right: Point<Space>) -> Self {
		Self { left, right }
	}
}

impl<Space> Copy for NavigationPortal<Space> {}

impl<Space> Clone for NavigationPortal<Space> {
	fn clone(&self) -> Self {
		*self
	}
}

impl<Space> fmt::Debug for NavigationPortal<Space> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter
			.debug_struct("NavigationPortal")
			.field("left", &self.left)
			.field("right", &self.right)
			.finish()
	}
}

impl<Space> PartialEq for NavigationPortal<Space> {
	fn eq(&self, other: &Self) -> bool {
		self.left == other.left && self.right == other.right
	}
}

/// Describes why a navigation path could not be produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationPathError {
	/// The start contains NaN or infinity.
	NonFiniteStart,
	/// The target contains NaN or infinity.
	NonFiniteTarget,
	/// No polygon covers the start's XZ coordinates.
	StartOutsideMesh,
	/// No polygon covers the target's XZ coordinates.
	TargetOutsideMesh,
	/// The start and target polygons are disconnected.
	Unreachable,
}

impl fmt::Display for NavigationPathError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::NonFiniteStart => formatter.write_str(
				"Invalid navigation path start. The most likely cause is a coordinate containing NaN or infinity.",
			),
			Self::NonFiniteTarget => formatter.write_str(
				"Invalid navigation path target. The most likely cause is a coordinate containing NaN or infinity.",
			),
			Self::StartOutsideMesh => formatter.write_str(
				"Navigation path start is outside the mesh. The most likely cause is a point not covered by any polygon on XZ.",
			),
			Self::TargetOutsideMesh => formatter.write_str(
				"Navigation path target is outside the mesh. The most likely cause is a point not covered by any polygon on XZ.",
			),
			Self::Unreachable => formatter.write_str(
				"Navigation target is unreachable. The most likely cause is disconnected start and target polygons.",
			),
		}
	}
}

impl std::error::Error for NavigationPathError {}

/// Describes why a portal corridor could not be string-pulled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringPullError {
	/// The start contains NaN or infinity.
	NonFiniteStart,
	/// The target contains NaN or infinity.
	NonFiniteTarget,
	/// A portal endpoint contains NaN or infinity.
	NonFinitePortal { portal: usize },
}

impl fmt::Display for StringPullError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Self::NonFiniteStart => formatter
				.write_str("Invalid string-pull start. The most likely cause is a coordinate containing NaN or infinity."),
			Self::NonFiniteTarget => formatter
				.write_str("Invalid string-pull target. The most likely cause is a coordinate containing NaN or infinity."),
			Self::NonFinitePortal { portal } => write!(
				formatter,
				"Invalid string-pull portal {portal}. The most likely cause is an endpoint containing NaN or infinity."
			),
		}
	}
}

impl std::error::Error for StringPullError {}

#[derive(Clone, Copy)]
enum Winding {
	Clockwise,
	CounterClockwise,
}

struct NavigationPolygon<Space> {
	first_vertex: usize,
	vertex_count: usize,
	centroid: Point<Space>,
	minimum_x: f32,
	maximum_x: f32,
	minimum_z: f32,
	maximum_z: f32,
	winding: Winding,
}

struct NavigationAdjacency<Space> {
	neighbor: NodeHandle,
	portal: NavigationPortal<Space>,
}

#[derive(Clone, Copy)]
struct EdgeOwner {
	polygon: NodeHandle,
	from: NavigationVertexHandle,
	to: NavigationVertexHandle,
}

/// The `NavigationMesh` struct provides connected convex polygons for XZ-plane path queries.
///
/// Construct a mesh with [`NavigationMesh::new`]. Shared vertex indices define portals, so adjacent
/// polygons must reference the same two indices along their common edge. Paths model a point-sized
/// agent, so authored polygons must already account for required wall clearance. Call
/// [`NavigationMesh::find_path`] next to locate endpoints, search the polygon graph, and string-pull
/// the resulting corridor.
pub struct NavigationMesh<Space = WorldSpace> {
	vertices: Vec<Point<Space>>,
	polygon_vertices: Vec<NavigationVertexHandle>,
	polygons: Vec<NavigationPolygon<Space>>,
	adjacency_offsets: Vec<usize>,
	adjacency: Vec<NavigationAdjacency<Space>>,
}

impl<Space> NavigationMesh<Space> {
	/// Builds a navigation mesh from vertices and convex polygon index loops.
	///
	/// Polygon winding may be clockwise or counterclockwise, but each polygon's XZ projection must
	/// be convex and have nonzero area. Y coordinates do not affect connectivity or funnel decisions.
	/// Call [`Self::find_path`] after construction to query the mesh.
	pub fn new(
		vertices: Vec<Point<Space>>,
		polygon_indices: Vec<Vec<NavigationVertexHandle>>,
	) -> Result<Self, NavigationMeshBuildError> {
		if vertices
			.len()
			.checked_sub(1)
			.is_some_and(|last_vertex| NavigationVertexHandle::try_from(last_vertex).is_err())
		{
			return Err(NavigationMeshBuildError::TooManyVertices);
		}
		if polygon_indices
			.len()
			.checked_sub(1)
			.is_some_and(|last_polygon| NodeHandle::try_from(last_polygon).is_err())
		{
			return Err(NavigationMeshBuildError::TooManyPolygons);
		}

		for (vertex, &point) in vertices.iter().enumerate() {
			if !is_finite(point) {
				return Err(NavigationMeshBuildError::NonFiniteVertex { vertex: vertex as _ });
			}
		}

		let polygon_count = polygon_indices.len();
		let index_count = polygon_indices.iter().map(Vec::len).sum();
		let mut polygon_vertices = Vec::with_capacity(index_count);
		let mut polygons = Vec::with_capacity(polygon_count);

		for (polygon_index, indices) in polygon_indices.into_iter().enumerate() {
			let polygon = polygon_index as NodeHandle;
			if indices.len() < 3 {
				return Err(NavigationMeshBuildError::PolygonTooSmall { polygon });
			}

			for (offset, &vertex) in indices.iter().enumerate() {
				if vertex as usize >= vertices.len() {
					return Err(NavigationMeshBuildError::InvalidVertex { polygon, vertex });
				}
				if indices[..offset].contains(&vertex) {
					return Err(NavigationMeshBuildError::RepeatedVertex { polygon, vertex });
				}
			}

			let area = polygon_area_xz(&vertices, &indices);
			if area == 0.0 {
				return Err(NavigationMeshBuildError::DegeneratePolygon { polygon });
			}
			if !is_convex_xz(&vertices, &indices, area) {
				return Err(NavigationMeshBuildError::NonConvexPolygon { polygon });
			}

			let winding = if area > 0.0 {
				Winding::CounterClockwise
			} else {
				Winding::Clockwise
			};
			let first_vertex = polygon_vertices.len();
			let centroid = polygon_centroid(&vertices, &indices);
			let [minimum_x, maximum_x, minimum_z, maximum_z] = polygon_bounds_xz(&vertices, &indices);
			polygon_vertices.extend_from_slice(&indices);
			polygons.push(NavigationPolygon {
				first_vertex,
				vertex_count: indices.len(),
				centroid,
				minimum_x,
				maximum_x,
				minimum_z,
				maximum_z,
				winding,
			});
		}

		let (adjacency_offsets, adjacency) = build_adjacency(&vertices, &polygon_vertices, &polygons)?;
		Ok(Self {
			vertices,
			polygon_vertices,
			polygons,
			adjacency_offsets,
			adjacency,
		})
	}

	/// Returns all mesh vertices in handle order.
	pub fn vertices(&self) -> &[Point<Space>] {
		&self.vertices
	}

	/// Returns the vertex handles around `polygon`, or [`None`] for an invalid polygon handle.
	pub fn polygon_vertex_handles(&self, polygon: NodeHandle) -> Option<&[NavigationVertexHandle]> {
		let polygon = self.polygons.get(polygon as usize)?;
		Some(self.vertex_handles(polygon))
	}

	/// Returns the arithmetic centroid of `polygon`, or [`None`] for an invalid polygon handle.
	pub fn polygon_centroid(&self, polygon: NodeHandle) -> Option<Point<Space>> {
		self.polygons.get(polygon as usize).map(|polygon| polygon.centroid)
	}

	/// Returns the number of navigation polygons.
	pub fn polygon_count(&self) -> usize {
		self.polygons.len()
	}

	/// Returns the directed portal from `from` to its adjacent `to` polygon.
	pub fn portal(&self, from: NodeHandle, to: NodeHandle) -> Option<NavigationPortal<Space>> {
		self.adjacencies(from)?
			.iter()
			.find(|edge| edge.neighbor == to)
			.map(|edge| edge.portal)
	}

	/// Locates the polygon under `point` on XZ.
	///
	/// When projected polygons overlap, this chooses the surface whose interpolated Y is closest to
	/// the point. Pass the returned handle to graph-level tools when you need the polygon corridor;
	/// otherwise call [`Self::find_path`] to produce a smoothed path directly.
	pub fn locate_polygon(&self, point: Point<Space>) -> Option<NodeHandle> {
		if !is_finite(point) {
			return None;
		}

		self.polygons
			.iter()
			.enumerate()
			.filter(|(_, polygon)| self.contains_xz(polygon, point))
			.map(|(index, polygon)| {
				let height = self.polygon_surface_height(polygon, point);
				(index as NodeHandle, (height - point.y()).abs())
			})
			.min_by(|(_, left), (_, right)| left.total_cmp(right))
			.map(|(polygon, _)| polygon)
	}

	/// Projects `point` vertically onto the nearest navigation surface at the same XZ coordinates.
	///
	/// The input Y selects between overlapping surfaces. Use this between corners returned by
	/// [`Self::find_path`] when movement must follow uneven terrain continuously.
	pub fn project_point(&self, point: Point<Space>) -> Option<Point<Space>> {
		let polygon = self.locate_polygon(point)?;
		let y = self.polygon_surface_height(&self.polygons[polygon as usize], point);
		Some(Point::new(point.x(), y, point.z()))
	}

	/// Finds and string-pulls a path between two points projected onto the mesh.
	///
	/// The result includes the exact start and target. Intermediate points are portal vertices whose
	/// stored Y coordinates are preserved. XZ controls route visibility; movement systems that must
	/// follow uneven terrain continuously should call [`Self::project_point`] between returned corners.
	pub fn find_path(&self, start: Point<Space>, target: Point<Space>) -> Result<Vec<Point<Space>>, NavigationPathError> {
		let mut path = Vec::new();
		self.find_path_into(start, target, &mut path)?;
		Ok(path)
	}

	/// Finds a path while reusing `path` for the string-pulled output.
	///
	/// Existing output is cleared after both endpoints are validated and a polygon corridor is found.
	/// On error, `path` remains unchanged. Call [`Self::project_point`] between returned corners when
	/// movement must follow uneven terrain continuously.
	pub fn find_path_into<'path>(
		&self,
		start: Point<Space>,
		target: Point<Space>,
		path: &'path mut Vec<Point<Space>>,
	) -> Result<&'path [Point<Space>], NavigationPathError> {
		if !is_finite(start) {
			return Err(NavigationPathError::NonFiniteStart);
		}
		if !is_finite(target) {
			return Err(NavigationPathError::NonFiniteTarget);
		}

		let start_polygon = self.locate_polygon(start).ok_or(NavigationPathError::StartOutsideMesh)?;
		let target_polygon = self.locate_polygon(target).ok_or(NavigationPathError::TargetOutsideMesh)?;
		let polygon_path = a_star(start_polygon, target_polygon, self, |from, to| {
			distance_xz(self.polygons[from as usize].centroid, self.polygons[to as usize].centroid)
		});
		if polygon_path.is_empty() {
			return Err(NavigationPathError::Unreachable);
		}

		string_pull_from_corridor(
			start,
			target,
			polygon_path.len().saturating_sub(1),
			|index| {
				// A* can only traverse adjacency entries, so every corridor pair has a portal.
				self.portal(polygon_path[index], polygon_path[index + 1])
					.expect("navigation adjacency must contain its portal")
			},
			path,
		);
		Ok(path)
	}

	fn vertex_handles(&self, polygon: &NavigationPolygon<Space>) -> &[NavigationVertexHandle] {
		&self.polygon_vertices[polygon.first_vertex..polygon.first_vertex + polygon.vertex_count]
	}

	fn adjacencies(&self, polygon: NodeHandle) -> Option<&[NavigationAdjacency<Space>]> {
		let index = polygon as usize;
		let start = *self.adjacency_offsets.get(index)?;
		let end = *self.adjacency_offsets.get(index + 1)?;
		Some(&self.adjacency[start..end])
	}

	/// Tests an XZ point against every directed half-plane of a convex polygon.
	fn contains_xz(&self, polygon: &NavigationPolygon<Space>, point: Point<Space>) -> bool {
		if point.x() < polygon.minimum_x
			|| point.x() > polygon.maximum_x
			|| point.z() < polygon.minimum_z
			|| point.z() > polygon.maximum_z
		{
			return false;
		}

		let handles = self.vertex_handles(polygon);
		handles
			.iter()
			.copied()
			.zip(handles.iter().copied().cycle().skip(1))
			.all(|(from, to)| {
				let side = signed_area_xz(self.vertices[from as usize], self.vertices[to as usize], point);
				match polygon.winding {
					Winding::CounterClockwise => side >= -orientation_tolerance(side),
					Winding::Clockwise => side <= orientation_tolerance(side),
				}
			})
	}

	/// Interpolates Y from the polygon's triangle fan for stacked-surface selection.
	fn polygon_surface_height(&self, polygon: &NavigationPolygon<Space>, point: Point<Space>) -> f32 {
		let handles = self.vertex_handles(polygon);
		let first = self.vertices[handles[0] as usize];
		for pair in handles[1..].windows(2) {
			let second = self.vertices[pair[0] as usize];
			let third = self.vertices[pair[1] as usize];
			if let Some([first_weight, second_weight, third_weight]) = barycentric_xz(point, first, second, third) {
				return first.y() * first_weight + second.y() * second_weight + third.y() * third_weight;
			}
		}

		polygon.centroid.y()
	}
}

impl<Space> Graph<()> for NavigationMesh<Space> {
	fn node_count(&self) -> usize {
		self.polygons.len()
	}

	fn neighbors(&self, node: NodeHandle) -> impl Iterator<Item = NodeHandle> + '_ {
		self.adjacencies(node).into_iter().flatten().map(|edge| edge.neighbor)
	}
}

/// Pulls a shortest XZ polyline taut through an ordered portal corridor.
///
/// The result includes `start` and `target`, omits redundant collinear portal points, and preserves
/// the Y coordinate of every emitted input point. Portals must already be oriented left-to-right in
/// the direction of travel. Use [`NavigationMesh::find_path`] when the corridor should be searched
/// and oriented from indexed navigation geometry.
pub fn string_pull<Space>(
	start: Point<Space>,
	target: Point<Space>,
	portals: &[NavigationPortal<Space>],
) -> Result<Vec<Point<Space>>, StringPullError> {
	let mut path = Vec::new();
	string_pull_into(start, target, portals, &mut path)?;
	Ok(path)
}

/// Pulls a portal corridor while reusing `path` for output storage.
///
/// Existing output is cleared only after all inputs are validated. On error, `path` remains
/// unchanged. Use this in repeated path queries to avoid allocating a new output vector.
pub fn string_pull_into<'path, Space>(
	start: Point<Space>,
	target: Point<Space>,
	portals: &[NavigationPortal<Space>],
	path: &'path mut Vec<Point<Space>>,
) -> Result<&'path [Point<Space>], StringPullError> {
	if !is_finite(start) {
		return Err(StringPullError::NonFiniteStart);
	}
	if !is_finite(target) {
		return Err(StringPullError::NonFiniteTarget);
	}
	if let Some(portal) = portals
		.iter()
		.position(|portal| !is_finite(portal.left) || !is_finite(portal.right))
	{
		return Err(StringPullError::NonFinitePortal { portal });
	}

	string_pull_from_corridor(start, target, portals.len(), |index| portals[index], path);
	Ok(path)
}

/// Applies the simple-stupid-funnel restart rule without revalidating mesh-owned points.
fn string_pull_from_corridor<Space>(
	start: Point<Space>,
	target: Point<Space>,
	portal_count: usize,
	mut portal_at: impl FnMut(usize) -> NavigationPortal<Space>,
	path: &mut Vec<Point<Space>>,
) {
	path.clear();
	path.reserve(portal_count.min(8) + 2);
	path.push(start);

	let mut apex = start;
	let mut left = start;
	let mut right = start;
	let mut left_index = 0;
	let mut right_index = 0;
	let mut portal_index = 1;
	let corridor_point_count = portal_count + 2;

	while portal_index < corridor_point_count {
		let portal = if portal_index <= portal_count {
			portal_at(portal_index - 1)
		} else {
			NavigationPortal::new(target, target)
		};

		if signed_area_xz(apex, right, portal.right) >= 0.0 {
			if same_xz(apex, right) || signed_area_xz(apex, left, portal.right) < 0.0 {
				right = portal.right;
				right_index = portal_index;
			} else {
				push_distinct(path, left);
				apex = left;
				left = apex;
				right = apex;
				portal_index = left_index + 1;
				right_index = left_index;
				continue;
			}
		}

		if signed_area_xz(apex, left, portal.left) <= 0.0 {
			if same_xz(apex, left) || signed_area_xz(apex, right, portal.left) > 0.0 {
				left = portal.left;
				left_index = portal_index;
			} else {
				push_distinct(path, right);
				apex = right;
				left = apex;
				right = apex;
				portal_index = right_index + 1;
				left_index = right_index;
				continue;
			}
		}

		portal_index += 1;
	}

	push_distinct(path, target);
}

/// Builds contiguous directed adjacency records from shared undirected index edges.
fn build_adjacency<Space>(
	vertices: &[Point<Space>],
	polygon_vertices: &[NavigationVertexHandle],
	polygons: &[NavigationPolygon<Space>],
) -> Result<(Vec<usize>, Vec<NavigationAdjacency<Space>>), NavigationMeshBuildError> {
	let mut edge_uses = HashMap::new();
	let mut connections = Vec::new();

	for (polygon_index, polygon) in polygons.iter().enumerate() {
		let polygon_handle = polygon_index as NodeHandle;
		let handles = &polygon_vertices[polygon.first_vertex..polygon.first_vertex + polygon.vertex_count];
		for (from, to) in handles.iter().copied().zip(handles.iter().copied().cycle().skip(1)) {
			let key = if from < to { (from, to) } else { (to, from) };
			let owner = EdgeOwner {
				polygon: polygon_handle,
				from,
				to,
			};
			match edge_uses.entry(key) {
				Entry::Vacant(entry) => {
					entry.insert(Some(owner));
				}
				Entry::Occupied(mut entry) => match *entry.get() {
					Some(first_owner) => {
						connections.push((first_owner, owner));
						entry.insert(None);
					}
					None => {
						return Err(NavigationMeshBuildError::NonManifoldEdge {
							first: key.0,
							second: key.1,
						});
					}
				},
			}
		}
	}

	let mut directed = Vec::with_capacity(connections.len() * 2);
	for (first, second) in connections {
		let first_portal = oriented_portal_handles(&polygons[first.polygon as usize], first.from, first.to);
		let second_portal = oriented_portal_handles(&polygons[second.polygon as usize], second.from, second.to);
		if first_portal != [second_portal[1], second_portal[0]] {
			return Err(NavigationMeshBuildError::InconsistentSharedEdge {
				first: first.from.min(first.to),
				second: first.from.max(first.to),
			});
		}
		directed.push((
			first.polygon,
			NavigationAdjacency {
				neighbor: second.polygon,
				portal: NavigationPortal::new(vertices[first_portal[0] as usize], vertices[first_portal[1] as usize]),
			},
		));
		directed.push((
			second.polygon,
			NavigationAdjacency {
				neighbor: first.polygon,
				portal: NavigationPortal::new(vertices[second_portal[0] as usize], vertices[second_portal[1] as usize]),
			},
		));
	}
	// Grouping by source polygon gives every graph query one contiguous neighbor range.
	directed.sort_unstable_by_key(|(polygon, edge)| (*polygon, edge.neighbor));

	let mut adjacency_offsets = vec![0; polygons.len() + 1];
	for &(polygon, _) in &directed {
		adjacency_offsets[polygon as usize + 1] += 1;
	}
	for index in 1..adjacency_offsets.len() {
		adjacency_offsets[index] += adjacency_offsets[index - 1];
	}
	let adjacency = directed.into_iter().map(|(_, edge)| edge).collect();
	Ok((adjacency_offsets, adjacency))
}

fn oriented_portal_handles<Space>(
	polygon: &NavigationPolygon<Space>,
	from: NavigationVertexHandle,
	to: NavigationVertexHandle,
) -> [NavigationVertexHandle; 2] {
	match polygon.winding {
		Winding::CounterClockwise => [to, from],
		Winding::Clockwise => [from, to],
	}
}

fn polygon_area_xz<Space>(vertices: &[Point<Space>], indices: &[NavigationVertexHandle]) -> f32 {
	indices
		.iter()
		.copied()
		.zip(indices.iter().copied().cycle().skip(1))
		.map(|(from, to)| {
			let from = vertices[from as usize];
			let to = vertices[to as usize];
			from.x() * to.z() - to.x() * from.z()
		})
		.sum::<f32>()
		* 0.5
}

fn is_convex_xz<Space>(vertices: &[Point<Space>], indices: &[NavigationVertexHandle], polygon_area: f32) -> bool {
	let corners_are_convex = indices
		.iter()
		.copied()
		.zip(indices.iter().copied().cycle().skip(1))
		.zip(indices.iter().copied().cycle().skip(2))
		.all(|((first, second), third)| {
			let corner = signed_area_xz(vertices[first as usize], vertices[second as usize], vertices[third as usize]);
			corner == 0.0 || corner.is_sign_positive() == polygon_area.is_sign_positive()
		});
	if !corners_are_convex {
		return false;
	}

	// Consistent local turns alone can still describe a self-intersecting star polygon.
	for first_edge in 0..indices.len() {
		let first_next = (first_edge + 1) % indices.len();
		for second_edge in first_edge + 1..indices.len() {
			let second_next = (second_edge + 1) % indices.len();
			if first_edge == second_next || first_next == second_edge {
				continue;
			}

			if segments_intersect_xz(
				vertices[indices[first_edge] as usize],
				vertices[indices[first_next] as usize],
				vertices[indices[second_edge] as usize],
				vertices[indices[second_next] as usize],
			) {
				return false;
			}
		}
	}

	true
}

fn polygon_centroid<Space>(vertices: &[Point<Space>], indices: &[NavigationVertexHandle]) -> Point<Space> {
	let [x, y, z] = indices.iter().fold([0.0; 3], |[x, y, z], &vertex| {
		let point = vertices[vertex as usize];
		[x + point.x(), y + point.y(), z + point.z()]
	});
	let count = indices.len() as f32;
	Point::new(x / count, y / count, z / count)
}

fn polygon_bounds_xz<Space>(vertices: &[Point<Space>], indices: &[NavigationVertexHandle]) -> [f32; 4] {
	indices.iter().fold(
		[f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY],
		|[minimum_x, maximum_x, minimum_z, maximum_z], &vertex| {
			let point = vertices[vertex as usize];
			[
				minimum_x.min(point.x()),
				maximum_x.max(point.x()),
				minimum_z.min(point.z()),
				maximum_z.max(point.z()),
			]
		},
	)
}

fn orientation_tolerance(value: f32) -> f32 {
	f32::EPSILON * 16.0 * value.abs().max(1.0)
}

fn same_xz<Space>(first: Point<Space>, second: Point<Space>) -> bool {
	let x = second.x() - first.x();
	let z = second.z() - first.z();
	x * x + z * z <= 1.0e-12
}

fn push_distinct<Space>(path: &mut Vec<Point<Space>>, point: Point<Space>) {
	if path.last().is_none_or(|&last| last != point) {
		path.push(point);
	}
}

/// Describes why indexed geometry could not form a navigation mesh.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavigationMeshBuildError {
	/// The vertex count exceeds the handle range.
	TooManyVertices,
	/// The polygon count exceeds the handle range.
	TooManyPolygons,
	/// A vertex contains NaN or infinity.
	NonFiniteVertex { vertex: NavigationVertexHandle },
	/// A polygon contains fewer than three vertices.
	PolygonTooSmall { polygon: NodeHandle },
	/// A polygon references a missing vertex.
	InvalidVertex {
		polygon: NodeHandle,
		vertex: NavigationVertexHandle,
	},
	/// A polygon references one vertex more than once.
	RepeatedVertex {
		polygon: NodeHandle,
		vertex: NavigationVertexHandle,
	},
	/// A polygon has zero projected area on XZ.
	DegeneratePolygon { polygon: NodeHandle },
	/// A polygon's XZ boundary is concave or self-intersecting.
	NonConvexPolygon { polygon: NodeHandle },
	/// More than two polygons share an edge.
	NonManifoldEdge {
		first: NavigationVertexHandle,
		second: NavigationVertexHandle,
	},
	/// Two polygons sharing an edge have projected interiors on the same side.
	InconsistentSharedEdge {
		first: NavigationVertexHandle,
		second: NavigationVertexHandle,
	},
}

impl fmt::Display for NavigationMeshBuildError {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match *self {
			Self::TooManyVertices => formatter.write_str(
				"Invalid navigation mesh. The most likely cause is geometry with more vertices than 32-bit handles can address.",
			),
			Self::TooManyPolygons => formatter.write_str(
				"Invalid navigation mesh. The most likely cause is geometry with more polygons than 32-bit handles can address.",
			),
			Self::NonFiniteVertex { vertex } => write!(
				formatter,
				"Invalid navigation mesh vertex {vertex}. The most likely cause is a coordinate containing NaN or infinity."
			),
			Self::PolygonTooSmall { polygon } => write!(
				formatter,
				"Invalid navigation polygon {polygon}. The most likely cause is a polygon containing fewer than three vertices."
			),
			Self::InvalidVertex { polygon, vertex } => write!(
				formatter,
				"Invalid vertex {vertex} in navigation polygon {polygon}. The most likely cause is an index outside the vertex buffer."
			),
			Self::RepeatedVertex { polygon, vertex } => write!(
				formatter,
				"Invalid repeated vertex {vertex} in navigation polygon {polygon}. The most likely cause is a malformed polygon index loop."
			),
			Self::DegeneratePolygon { polygon } => write!(
				formatter,
				"Invalid navigation polygon {polygon}. The most likely cause is a polygon with zero projected area on XZ."
			),
			Self::NonConvexPolygon { polygon } => write!(
				formatter,
				"Invalid navigation polygon {polygon}. The most likely cause is a concave or self-intersecting XZ boundary."
			),
			Self::NonManifoldEdge { first, second } => write!(
				formatter,
				"Invalid navigation edge {first}-{second}. The most likely cause is more than two polygons sharing one edge."
			),
			Self::InconsistentSharedEdge { first, second } => write!(
				formatter,
				"Invalid shared navigation edge {first}-{second}. The most likely cause is adjacent polygon interiors lying on the same side."
			),
		}
	}
}

impl std::error::Error for NavigationMeshBuildError {}

#[cfg(test)]
mod tests {
	use super::*;

	fn point(x: f32, z: f32) -> Point {
		Point::new(x, 0.0, z)
	}

	fn l_shaped_mesh() -> NavigationMesh {
		NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(2.0, 0.0),
				Point::new(2.0, 2.0, 2.0),
				point(0.0, 2.0),
				point(4.0, 0.0),
				point(4.0, 2.0),
				point(4.0, 4.0),
				point(2.0, 4.0),
			],
			vec![vec![0, 1, 2, 3], vec![1, 4, 5, 2], vec![2, 5, 6, 7]],
		)
		.expect("valid L-shaped navigation mesh")
	}

	#[test]
	fn builds_directed_portals_from_shared_index_edges() {
		let mesh = l_shaped_mesh();

		assert_eq!(mesh.polygon_count(), 3);
		assert_eq!(
			mesh.portal(0, 1),
			Some(NavigationPortal::new(Point::new(2.0, 2.0, 2.0), point(2.0, 0.0)))
		);
		assert_eq!(
			mesh.portal(1, 0),
			Some(NavigationPortal::new(point(2.0, 0.0), Point::new(2.0, 2.0, 2.0)))
		);
		assert_eq!(mesh.neighbors(1).collect::<Vec<_>>(), [0, 2]);
	}

	#[test]
	fn string_pull_removes_portals_from_a_straight_corridor() {
		let start = point(0.0, 0.0);
		let target = point(4.0, 0.0);
		let portals = [
			NavigationPortal::new(point(1.0, 1.0), point(1.0, -1.0)),
			NavigationPortal::new(point(2.0, 1.0), point(2.0, -1.0)),
			NavigationPortal::new(point(3.0, 1.0), point(3.0, -1.0)),
		];

		assert_eq!(string_pull(start, target, &portals).unwrap(), [start, target]);
	}

	#[test]
	fn string_pull_emits_the_tight_corner_with_its_stored_height() {
		let start = point(0.5, 1.0);
		let target = point(3.0, 3.5);
		let corner = Point::new(2.0, 2.0, 2.0);
		let portals = [
			NavigationPortal::new(corner, point(2.0, 0.0)),
			NavigationPortal::new(corner, point(4.0, 2.0)),
		];

		assert_eq!(string_pull(start, target, &portals).unwrap(), [start, corner, target]);
	}

	#[test]
	fn string_pull_routes_through_a_zero_width_portal() {
		let start = point(0.0, 0.0);
		let target = point(2.0, 0.0);
		let gate = Point::new(1.0, 3.0, 1.0);
		let portals = [NavigationPortal::new(gate, gate)];

		assert_eq!(string_pull(start, target, &portals).unwrap(), [start, gate, target]);
	}

	#[test]
	fn string_pull_reuses_output_and_preserves_it_on_error() {
		let start = point(0.0, 0.0);
		let target = point(2.0, 0.0);
		let portals = [NavigationPortal::new(point(1.0, 1.0), point(1.0, -1.0))];
		let mut path = Vec::with_capacity(4);
		path.push(point(-1.0, -1.0));
		let allocation = path.as_ptr();

		assert_eq!(string_pull_into(start, target, &portals, &mut path).unwrap(), [start, target]);
		assert_eq!(path.as_ptr(), allocation);

		let previous = path.clone();
		assert_eq!(
			string_pull_into(Point::new(f32::NAN, 0.0, 0.0), target, &portals, &mut path),
			Err(StringPullError::NonFiniteStart)
		);
		assert_eq!(path, previous);
	}

	#[test]
	fn finds_and_string_pulls_both_directions_through_a_turn() {
		let mesh = l_shaped_mesh();
		let start = point(0.5, 1.0);
		let target = point(3.0, 3.5);
		let corner = Point::new(2.0, 2.0, 2.0);

		assert_eq!(mesh.find_path(start, target).unwrap(), [start, corner, target]);
		assert_eq!(mesh.find_path(target, start).unwrap(), [target, corner, start]);
	}

	#[test]
	fn clockwise_polygons_produce_the_same_oriented_corridor() {
		let mesh = NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(2.0, 0.0),
				Point::new(2.0, 2.0, 2.0),
				point(0.0, 2.0),
				point(4.0, 0.0),
				point(4.0, 2.0),
				point(4.0, 4.0),
				point(2.0, 4.0),
			],
			vec![vec![3, 2, 1, 0], vec![2, 5, 4, 1], vec![7, 6, 5, 2]],
		)
		.unwrap();
		let start = point(0.5, 1.0);
		let target = point(3.0, 3.5);
		let corner = Point::new(2.0, 2.0, 2.0);

		assert_eq!(mesh.find_path(start, target).unwrap(), [start, corner, target]);
	}

	#[test]
	fn returns_a_direct_path_inside_one_polygon() {
		let mesh = l_shaped_mesh();
		let start = point(0.25, 0.25);
		let target = point(1.5, 1.0);

		assert_eq!(mesh.find_path(start, target).unwrap(), [start, target]);
	}

	#[test]
	fn locates_the_nearest_height_when_xz_projections_overlap() {
		let mesh = NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(2.0, 0.0),
				point(2.0, 2.0),
				point(0.0, 2.0),
				Point::new(0.0, 10.0, 0.0),
				Point::new(2.0, 10.0, 0.0),
				Point::new(2.0, 10.0, 2.0),
				Point::new(0.0, 10.0, 2.0),
			],
			vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
		)
		.unwrap();

		assert_eq!(mesh.locate_polygon(Point::new(1.0, 1.0, 1.0)), Some(0));
		assert_eq!(mesh.locate_polygon(Point::new(1.0, 9.0, 1.0)), Some(1));
		assert_eq!(
			mesh.project_point(Point::new(1.0, 9.0, 1.0)),
			Some(Point::new(1.0, 10.0, 1.0))
		);
	}

	#[test]
	fn reports_disconnected_polygons_as_unreachable() {
		let mesh = NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(1.0, 0.0),
				point(1.0, 1.0),
				point(0.0, 1.0),
				point(2.0, 0.0),
				point(3.0, 0.0),
				point(3.0, 1.0),
				point(2.0, 1.0),
			],
			vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]],
		)
		.unwrap();

		assert_eq!(
			mesh.find_path(point(0.5, 0.5), point(2.5, 0.5)),
			Err(NavigationPathError::Unreachable)
		);
	}

	#[test]
	fn rejects_non_convex_and_non_manifold_geometry() {
		let non_convex = NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(2.0, 0.0),
				point(1.0, 1.0),
				point(2.0, 2.0),
				point(0.0, 2.0),
			],
			vec![vec![0, 1, 2, 3, 4]],
		);
		assert_eq!(
			non_convex.err(),
			Some(NavigationMeshBuildError::NonConvexPolygon { polygon: 0 })
		);

		let non_manifold = NavigationMesh::new(
			vec![
				point(0.0, 0.0),
				point(2.0, 0.0),
				point(1.0, 1.0),
				point(1.0, -1.0),
				point(1.0, 2.0),
			],
			vec![vec![0, 1, 2], vec![1, 0, 3], vec![0, 1, 4]],
		);
		assert_eq!(
			non_manifold.err(),
			Some(NavigationMeshBuildError::NonManifoldEdge { first: 0, second: 1 })
		);
	}

	#[test]
	fn rejects_self_intersections_and_same_side_shared_edges() {
		let self_intersecting = NavigationMesh::new(
			vec![
				point(0.0, 3.0),
				point(2.85, 0.93),
				point(1.76, -2.43),
				point(-1.76, -2.43),
				point(-2.85, 0.93),
			],
			vec![vec![0, 2, 4, 1, 3]],
		);
		assert_eq!(
			self_intersecting.err(),
			Some(NavigationMeshBuildError::NonConvexPolygon { polygon: 0 })
		);

		let same_side = NavigationMesh::new(
			vec![point(0.0, 0.0), point(2.0, 0.0), point(1.0, 1.0), point(1.0, 2.0)],
			vec![vec![0, 1, 2], vec![0, 1, 3]],
		);
		assert_eq!(
			same_side.err(),
			Some(NavigationMeshBuildError::InconsistentSharedEdge { first: 0, second: 1 })
		);
	}

	#[test]
	fn rejects_non_finite_query_and_portal_points() {
		let mesh = l_shaped_mesh();
		assert_eq!(
			mesh.find_path(Point::new(f32::NAN, 0.0, 0.0), point(1.0, 1.0)),
			Err(NavigationPathError::NonFiniteStart)
		);

		let portals = [NavigationPortal::new(point(1.0, 1.0), Point::new(1.0, f32::INFINITY, -1.0))];
		assert_eq!(
			string_pull(point(0.0, 0.0), point(2.0, 0.0), &portals),
			Err(StringPullError::NonFinitePortal { portal: 0 })
		);
	}
}

use std::{
	collections::{hash_map::Entry, HashMap},
	fmt,
};

use math::{barycentric_xz, distance_xz, is_finite, segments_intersect_xz, signed_area_xz, Point, WorldSpace};

use super::{a_star, Graph, NodeHandle};
