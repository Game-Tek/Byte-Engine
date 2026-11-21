//! Keeps the area of each piece of a primitive in its coarser detail levels.
//!
//! Simplification collapses the edges of small open pieces, such as leaf cards, inward, so foliage thins out at every
//! coarser level. These functions scale each piece that survives a level back to its full-detail area, about its own
//! center. They only compute positions: [`MeshProcessorSession`](super::MeshProcessorSession) decides which primitives
//! use them and stores them. The preserved area rules of the asset reference define the result.

use super::lightmap_uvs::{find, union};

/// The largest factor a piece grows by, so a piece the simplifier reduced to a sliver doesn't balloon.
const MAX_PIECE_SCALE: f32 = 1.5;

/// The `PieceScratch` struct keeps the working memory of [`pieces`] and [`preserve_area`] across primitives.
#[derive(Default)]
pub(super) struct PieceScratch {
	/// The union-find parent of each welded vertex; roots name pieces.
	parents: Vec<u32>,
	/// The compact piece of each union-find root, or `u32::MAX` before it is numbered.
	piece_of_root: Vec<u32>,
	/// Each piece's area and area-weighted center sum in the level being scaled.
	level_areas: Vec<f32>,
	level_centers: Vec<[f32; 3]>,
}

/// The `Pieces` struct names the piece of each vertex of one primitive and the area each piece should keep.
///
/// Build it once per primitive with [`pieces`], then pass it to [`preserve_area`] for each coarser level.
pub(super) struct Pieces {
	/// The piece of each source vertex.
	piece_of: Vec<u32>,
	/// The area of each piece in the finest level that has triangles of it.
	full_areas: Vec<f32>,
}

/// Splits the primitive whose vertices are `positions` into pieces, the sets of triangles connected through shared
/// corners in any of its `levels`, finest first, and measures the area each piece has in the finest level that has
/// triangles of it.
///
/// That is full detail for every piece a model starts with. A piece that first appears in an authored level keeps that
/// level's area, so the levels simplified from it grow back to it. Corners weld by position, so a seam in UVs or
/// normals doesn't split a piece and open a crack when it scales.
pub(super) fn pieces<'a>(
	positions: &[[f32; 3]],
	levels: impl Iterator<Item = &'a [u32]> + Clone,
	scratch: &mut PieceScratch,
) -> Pieces {
	let adapter = meshopt::VertexDataAdapter::new(bytemuck::cast_slice(positions), 12, 0)
		.expect("positions are three tightly packed floats");
	let welded = meshopt::generate_position_remap(&adapter);

	scratch.parents.clear();
	scratch.parents.extend(0..positions.len() as u32);
	for triangle in levels.clone().flat_map(|triangles| triangles.as_chunks::<3>().0) {
		let [a, b, c] = triangle.map(|corner| welded[corner as usize]);
		union(&mut scratch.parents, a, b);
		union(&mut scratch.parents, a, c);
	}

	// Number the roots in first-seen order, so piece indices stay dense.
	scratch.piece_of_root.clear();
	scratch.piece_of_root.resize(positions.len(), u32::MAX);
	let mut piece_count = 0;
	let piece_of = welded
		.iter()
		.map(|&vertex| {
			let root = find(&mut scratch.parents, vertex) as usize;
			if scratch.piece_of_root[root] == u32::MAX {
				scratch.piece_of_root[root] = piece_count;
				piece_count += 1;
			}
			scratch.piece_of_root[root]
		})
		.collect::<Vec<_>>();

	// Each level measures only the pieces no finer level had triangles of.
	let mut full_areas = vec![0.0; piece_count as usize];
	scratch.level_areas.clear();
	scratch.level_areas.resize(piece_count as usize, 0.0);
	for triangles in levels {
		for triangle in triangles.as_chunks::<3>().0 {
			scratch.level_areas[piece_of[triangle[0] as usize] as usize] += triangle_area(positions, triangle);
		}
		for (full, level) in full_areas.iter_mut().zip(&mut scratch.level_areas) {
			if *full == 0.0 {
				*full = *level;
			}
			*level = 0.0;
		}
	}
	Pieces { piece_of, full_areas }
}

/// Writes to `output` the positions of `vertices` in one coarser level whose triangles are `level_triangles`, each
/// piece that has triangles in the level scaled back toward its full-detail area, and returns the farthest a vertex
/// of the level's triangles moved.
///
/// `positions` and `level_triangles` use source vertex indices, and `vertices` lists the source vertex of each output
/// vertex the level holds, which is the order `output` follows.
pub(super) fn preserve_area(
	positions: &[[f32; 3]],
	pieces: &Pieces,
	level_triangles: &[u32],
	vertices: &[u32],
	scratch: &mut PieceScratch,
	output: &mut Vec<[f32; 3]>,
) -> f32 {
	let piece_count = pieces.full_areas.len();
	scratch.level_areas.clear();
	scratch.level_areas.resize(piece_count, 0.0);
	scratch.level_centers.clear();
	scratch.level_centers.resize(piece_count, [0.0; 3]);
	for triangle in level_triangles.as_chunks::<3>().0 {
		let piece = pieces.piece_of[triangle[0] as usize] as usize;
		let area = triangle_area(positions, triangle);
		scratch.level_areas[piece] += area;
		for corner in triangle {
			for axis in 0..3 {
				scratch.level_centers[piece][axis] += positions[*corner as usize][axis] * area / 3.0;
			}
		}
	}

	// A piece's scale and center, or none when the level has no area of it to scale.
	let transform = |piece: usize| -> Option<(f32, [f32; 3])> {
		let level_area = scratch.level_areas[piece];
		if level_area <= 0.0 {
			return None;
		}
		let scale = (pieces.full_areas[piece] / level_area).sqrt().clamp(1.0, MAX_PIECE_SCALE);
		let center = scratch.level_centers[piece].map(|sum| sum / level_area);
		Some((scale, center))
	};
	let scaled = |vertex: u32| -> [f32; 3] {
		let position = positions[vertex as usize];
		match transform(pieces.piece_of[vertex as usize] as usize) {
			Some((scale, center)) => std::array::from_fn(|axis| center[axis] + (position[axis] - center[axis]) * scale),
			None => position,
		}
	};

	output.clear();
	output.extend(vertices.iter().map(|&vertex| scaled(vertex)));
	// Only corners of the level's triangles change its surface; other vertices of the prefix are drawn by coarser
	// levels, which store their own positions.
	level_triangles
		.iter()
		.map(|&vertex| {
			let (from, to) = (positions[vertex as usize], scaled(vertex));
			(0..3).map(|axis| (to[axis] - from[axis]).powi(2)).sum::<f32>().sqrt()
		})
		.fold(0.0, f32::max)
}

/// Returns the area of `triangle`, three indices into `positions`.
fn triangle_area(positions: &[[f32; 3]], triangle: &[u32; 3]) -> f32 {
	let [a, b, c] = triangle.map(|corner| positions[corner as usize]);
	let (u, v) = (
		[b[0] - a[0], b[1] - a[1], b[2] - a[2]],
		[c[0] - a[0], c[1] - a[1], c[2] - a[2]],
	);
	let cross = [
		u[1] * v[2] - u[2] * v[1],
		u[2] * v[0] - u[0] * v[2],
		u[0] * v[1] - u[1] * v[0],
	];
	0.5 * (cross[0] * cross[0] + cross[1] * cross[1] + cross[2] * cross[2]).sqrt()
}
