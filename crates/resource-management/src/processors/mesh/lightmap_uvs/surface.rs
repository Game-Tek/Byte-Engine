//! The topology lightmap UV generation reads: welded vertices, which faces are adjacent through which edges, and the
//! planar regions charts take whole. The Charts table of the asset reference's Lightmap UVs section defines each term.

use utils::hash::HashMap;

use super::{find, union};

/// Marks a corner without an adjacent corner, a face without a chart, and other absent indices.
pub(super) const NONE: u32 = u32::MAX;
/// The most two unit normals of one planar region may differ by, in any component.
const PLANAR_NORMAL_TOLERANCE: f64 = 0.001;
/// The dot product of a face's normal with a projection's normal at or below which the face counts as flipped.
pub(super) const FLIP_EPSILON: f64 = 1e-6;

/// The `Surface` struct is a primitive's triangles as lightmap UV generation sees them, so chart building, mapping,
/// and packing read one shared description of the mesh.
///
/// Corner `c` is index `c` of the triangle list; it starts the edge from its vertex to the next corner's in its face.
pub(super) struct Surface {
	/// Each source vertex's position, in double precision.
	pub(super) positions: Vec<[f64; 3]>,
	/// The welded vertex of each corner: the first source vertex at its position.
	pub(super) corner_vertices: Vec<u32>,
	/// Each face's unit normal, or zero for a face without area.
	pub(super) face_normals: Vec<[f64; 3]>,
	pub(super) face_areas: Vec<f64>,
	/// The corner of the adjacent face that runs the other way along each corner's edge, or [`NONE`] for a boundary
	/// edge.
	pub(super) opposite: Vec<u32>,
	/// The length of each corner's edge.
	pub(super) edge_lengths: Vec<f64>,
	/// The planar region of each face with area, or [`NONE`].
	pub(super) face_regions: Vec<u32>,
	pub(super) regions: Vec<Region>,
	/// The faces of every region, region after region; see [`Region::faces`].
	pub(super) region_faces: Vec<u32>,
	/// The corners of every region whose edge is a boundary edge or borders another region, region after region.
	pub(super) region_edges: Vec<u32>,
	/// For each corner, the lowest-index face with area that shares its edge between the same two welded vertices, in
	/// either direction, which a face without area joins the chart of.
	pub(super) edge_neighbors: Vec<u32>,
}

/// The `Region` struct is one planar region: faces that join a chart together.
pub(super) struct Region {
	/// The range of [`Surface::region_faces`] that holds the region's faces.
	pub(super) faces: std::ops::Range<u32>,
	/// The range of [`Surface::region_edges`] that holds the region's outer corners.
	pub(super) edges: std::ops::Range<u32>,
	pub(super) area: f64,
	/// The area-weighted mean of the region's face normals, normalized.
	pub(super) normal: [f64; 3],
	/// The sum of the region's face normals, each weighted by its face's area.
	pub(super) normal_sum: [f64; 3],
}

impl Surface {
	/// Builds the topology of the triangles `indices` over `positions`.
	pub(super) fn new(positions: &[[f32; 3]], indices: &[u32]) -> Self {
		let positions_adapter = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(positions), 12, 0)
			.expect("positions are three tightly packed floats");
		let welded = meshopt::generate_position_remap(&positions_adapter);
		let positions: Vec<[f64; 3]> = positions.iter().map(|p| p.map(f64::from)).collect();
		let corner_vertices: Vec<u32> = indices.iter().map(|&index| welded[index as usize]).collect();
		let face_count = indices.len() / 3;

		let mut face_normals = Vec::with_capacity(face_count);
		let mut face_areas = Vec::with_capacity(face_count);
		for face in 0..face_count {
			let [a, b, c] = [0, 1, 2].map(|corner| positions[corner_vertices[face * 3 + corner] as usize]);
			let normal = cross(sub(b, a), sub(c, a));
			let length = norm(normal);
			if length > 0.0 && length.is_finite() {
				face_normals.push(scale(normal, 1.0 / length));
				face_areas.push(length / 2.0);
			} else {
				face_normals.push([0.0; 3]);
				face_areas.push(0.0);
			}
		}

		// Each undirected edge between welded vertices: its first corner each way, how many corners use it, and its
		// lowest-index face with area.
		struct Edge {
			forward: u32,
			backward: u32,
			count: u32,
			face: u32,
		}
		let mut edges: HashMap<(u32, u32), Edge> = HashMap::with_capacity_and_hasher(indices.len(), Default::default());
		let mut edge_lengths = Vec::with_capacity(indices.len());
		for corner in 0..indices.len() {
			let (from, to) = (corner_vertices[corner], corner_vertices[next_corner(corner)]);
			edge_lengths.push(norm(sub(positions[to as usize], positions[from as usize])));
			let face = (corner / 3) as u32;
			let edge = edges.entry((from.min(to), from.max(to))).or_insert(Edge {
				forward: NONE,
				backward: NONE,
				count: 0,
				face: NONE,
			});
			if face_areas[face as usize] > 0.0 {
				edge.count += 1;
				edge.face = edge.face.min(face);
				let slot = if from < to { &mut edge.forward } else { &mut edge.backward };
				if *slot == NONE {
					*slot = corner as u32;
				}
			}
		}
		let mut opposite = vec![NONE; indices.len()];
		let mut edge_neighbors = vec![NONE; indices.len()];
		for corner in 0..indices.len() {
			let (from, to) = (corner_vertices[corner], corner_vertices[next_corner(corner)]);
			let edge = &edges[&(from.min(to), from.max(to))];
			edge_neighbors[corner] = edge.face;
			// Adjacent faces share an edge between distinct welded vertices in opposite directions, and no third face
			// shares it.
			if from != to && edge.count == 2 && edge.forward != NONE && edge.backward != NONE {
				let other = if from < to { edge.backward } else { edge.forward };
				if face_areas[corner / 3] > 0.0 && other as usize / 3 != corner / 3 {
					opposite[corner] = other;
				}
			}
		}

		// Planar regions: adjacent faces whose normals agree within the tolerance.
		let mut parents: Vec<u32> = (0..face_count as u32).collect();
		for corner in 0..indices.len() {
			let other = opposite[corner];
			if other == NONE {
				continue;
			}
			let (face, other_face) = (corner / 3, other as usize / 3);
			let close = (0..3).all(|axis| {
				(face_normals[face][axis] - face_normals[other_face][axis]).abs() <= PLANAR_NORMAL_TOLERANCE
			});
			if close {
				union(&mut parents, face as u32, other_face as u32);
			}
		}
		let mut face_regions = vec![NONE; face_count];
		// Roots are face indices, so the region of each root is found by index.
		let mut region_of_root = vec![NONE; face_count];
		let mut region_sizes: Vec<u32> = Vec::new();
		for face in 0..face_count {
			if face_areas[face] <= 0.0 {
				continue;
			}
			let root = find(&mut parents, face as u32) as usize;
			if region_of_root[root] == NONE {
				region_of_root[root] = region_sizes.len() as u32;
				region_sizes.push(0);
			}
			let region = region_of_root[root];
			region_sizes[region as usize] += 1;
			face_regions[face] = region;
		}
		// Regions list their faces in index order, then their outer corners.
		let mut starts = Vec::with_capacity(region_sizes.len());
		let mut total = 0u32;
		for &size in &region_sizes {
			starts.push(total);
			total += size;
		}
		let mut cursors = starts.clone();
		let mut region_faces = vec![0u32; total as usize];
		for face in 0..face_count {
			let region = face_regions[face];
			if region != NONE {
				region_faces[cursors[region as usize] as usize] = face as u32;
				cursors[region as usize] += 1;
			}
		}
		let mut regions = Vec::with_capacity(region_sizes.len());
		let mut region_edges = Vec::new();
		for (region, &start) in starts.iter().enumerate() {
			let faces = start..start + region_sizes[region];
			let edges_start = region_edges.len() as u32;
			let mut area = 0.0;
			let mut normal_sum = [0.0; 3];
			for &face in &region_faces[faces.start as usize..faces.end as usize] {
				let face = face as usize;
				area += face_areas[face];
				normal_sum = add(normal_sum, scale(face_normals[face], face_areas[face]));
				for corner in face * 3..face * 3 + 3 {
					let other = opposite[corner];
					if other == NONE || face_regions[other as usize / 3] != region as u32 {
						region_edges.push(corner as u32);
					}
				}
			}
			regions.push(Region {
				faces,
				edges: edges_start..region_edges.len() as u32,
				area,
				normal: normalize(normal_sum),
				normal_sum,
			});
		}

		Self {
			positions,
			corner_vertices,
			face_normals,
			face_areas,
			opposite,
			edge_lengths,
			face_regions,
			regions,
			region_faces,
			region_edges,
			edge_neighbors,
		}
	}

	pub(super) fn face_count(&self) -> usize {
		self.face_areas.len()
	}

	/// Returns the faces of `region`.
	pub(super) fn faces_of(&self, region: u32) -> &[u32] {
		let faces = &self.regions[region as usize].faces;
		&self.region_faces[faces.start as usize..faces.end as usize]
	}

	/// Returns the outer corners of `region`: those whose edge is a boundary edge or borders another region.
	pub(super) fn edges_of(&self, region: u32) -> &[u32] {
		let edges = &self.regions[region as usize].edges;
		&self.region_edges[edges.start as usize..edges.end as usize]
	}

	/// Returns the position of corner `corner`'s welded vertex.
	pub(super) fn corner_position(&self, corner: usize) -> [f64; 3] {
		self.positions[self.corner_vertices[corner] as usize]
	}

	/// Returns the welded vertices at the two ends of corner `corner`'s edge.
	pub(super) fn edge_vertices(&self, corner: usize) -> [u32; 2] {
		[self.corner_vertices[corner], self.corner_vertices[next_corner(corner)]]
	}

	/// Returns whether face `face` faces `normal` closely enough to keep its orientation when projected along it. A
	/// face without area always does.
	pub(super) fn faces_toward(&self, face: u32, normal: [f64; 3]) -> bool {
		let face = face as usize;
		self.face_areas[face] <= 0.0 || dot(self.face_normals[face], normal) > FLIP_EPSILON
	}

	/// Returns the mean of face `face`'s corners.
	pub(super) fn face_center(&self, face: usize) -> [f64; 3] {
		let sum = add(
			add(self.corner_position(face * 3), self.corner_position(face * 3 + 1)),
			self.corner_position(face * 3 + 2),
		);
		scale(sum, 1.0 / 3.0)
	}
}

/// Returns the corner after `corner` in its face, where its edge ends.
pub(super) fn next_corner(corner: usize) -> usize {
	corner - corner % 3 + (corner + 1) % 3
}

pub(super) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

pub(super) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

pub(super) fn scale(a: [f64; 3], factor: f64) -> [f64; 3] {
	a.map(|value| value * factor)
}

pub(super) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

pub(super) fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

pub(super) fn norm(a: [f64; 3]) -> f64 {
	dot(a, a).sqrt()
}

/// Returns `a` scaled to unit length, or zero for a zero vector.
pub(super) fn normalize(a: [f64; 3]) -> [f64; 3] {
	let length = norm(a);
	if length > 0.0 { scale(a, 1.0 / length) } else { [0.0; 3] }
}
