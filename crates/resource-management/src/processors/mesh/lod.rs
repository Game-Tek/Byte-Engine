/// The `MeshLodSettings` struct lets an asset choose how many coarser detail levels its meshes get when they are baked.
///
/// Pass it to [`MeshProcessorSession::new`](super::MeshProcessorSession::new). The default suits most meshes; an asset
/// overrides it through the `lods` key of its BEAD file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshLodSettings {
	/// Levels a primitive may hold, counting full detail. One turns generation off. Values above
	/// [`MAX_PRIMITIVE_LODS`] are treated as that limit.
	pub max_levels: u8,
	/// Largest error a level may carry, as a fraction of the primitive's largest extent.
	pub max_error: f32,
	/// Whether vertices on an open border stay in place. Set it for meshes whose primitives meet along a border, so
	/// that neighbors at different levels leave no gap.
	pub lock_borders: bool,
}

impl Default for MeshLodSettings {
	fn default() -> Self {
		Self {
			max_levels: MAX_PRIMITIVE_LODS as u8,
			max_error: 0.25,
			lock_borders: false,
		}
	}
}

/// The `LodLevel` struct carries one simplified triangle list from simplification to meshlet building.
pub(super) struct LodLevel {
	/// Triangle list that indexes the primitive's own vertices, ordered for the vertex cache like full detail.
	pub(super) indices: Vec<u32>,
	/// Estimated largest distance, in model-space units, between this level's surface and full detail.
	pub(super) error: f32,
}

/// Returns the levels coarser than `indices`, finest first. Returns none when the triangle list is too small to be
/// worth simplifying.
///
/// Each level is simplified from `indices` itself instead of from the level before it, so its error measures the
/// distance to `indices` directly. `base_error` is how far `indices` already lies from full detail, zero when it is
/// full detail; each level's error adds it, which bounds the level's distance to full detail. `normals` holds three
/// components per vertex, or nothing; with them, shading changes no more than the surface does. `vertex_locks` is
/// scratch storage.
pub(super) fn simplify_levels(
	indices: &[u32],
	base_error: f32,
	positions: &meshopt::VertexDataAdapter<'_>,
	normals: &[f32],
	vertex_locks: &mut Vec<bool>,
	settings: MeshLodSettings,
) -> Vec<LodLevel> {
	let max_levels = (settings.max_levels as usize).clamp(1, MAX_PRIMITIVE_LODS);
	let mut options = meshopt::SimplifyOptions::Prune;
	if settings.lock_borders {
		options |= meshopt::SimplifyOptions::LockBorder;
	}
	// The simplifier reports error relative to the mesh extent; this converts it to model-space units.
	let scale = meshopt::simplify_scale(positions);
	// The simplifier takes attributes only together with locks, and reads one lock per vertex. None is set.
	vertex_locks.clear();
	vertex_locks.resize(positions.vertex_count, false);
	let normal_weights: &[f32] = if normals.is_empty() { &[] } else { &[LOD_NORMAL_WEIGHT; 3] };

	let mut levels: Vec<LodLevel> = Vec::new();
	while levels.len() + 1 < max_levels {
		let (triangles, error) = levels
			.last()
			.map_or((indices.len() / 3, base_error), |level| (level.indices.len() / 3, level.error));
		if triangles <= LOD_MIN_TRIANGLES {
			break;
		}
		let mut relative_error = 0.0f32;
		let mut simplified = meshopt::simplify_with_attributes_and_locks(
			indices,
			positions,
			normals,
			normal_weights,
			size_of::<[f32; 3]>(),
			vertex_locks,
			(triangles as f32 * LOD_TRIANGLE_RATIO) as usize * 3,
			settings.max_error,
			options,
			Some(&mut relative_error),
		);
		// A level that removes little costs storage and a visible switch for almost no saved work. The simplifier
		// stalls like this at the error limit, on locked borders, and on meshes whose vertices are not welded.
		if simplified.is_empty() || (simplified.len() / 3) as f32 > triangles as f32 * (1.0 - LOD_MIN_REDUCTION) {
			break;
		}
		// The simplifier returns storage as large as its input.
		simplified.shrink_to_fit();
		meshopt::optimize_vertex_cache_in_place(&mut simplified, positions.vertex_count);
		// Selection assumes error never falls as levels get coarser.
		let level = LodLevel {
			indices: simplified,
			error: (relative_error * scale + base_error).max(error),
		};
		match levels.last_mut() {
			// A level that costs no more error than the one before it makes that one useless: selection would
			// always skip it for this one. This happens when both reach the error limit.
			Some(last) if level.error <= last.error => *last = level,
			_ => levels.push(level),
		}
	}
	levels
}

/// Returns an estimate of the largest distance between the surfaces of two triangle lists over the same vertices: the
/// farthest any vertex of either list lies from the other list's triangles.
///
/// Measures an authored detail level against full detail, since such a level is modeled separately and carries no
/// error of its own. Vertices stand in for their surfaces, so a deviation in the middle of a large triangle can go
/// unseen. Returns infinity when exactly one list has no triangles.
pub(super) fn level_distance(positions: &[[f32; 3]], full: &[u32], level: &[u32]) -> f32 {
	if full.is_empty() || level.is_empty() {
		return if full.is_empty() && level.is_empty() { 0.0 } else { f32::INFINITY };
	}
	let one_way = |from: &[u32], to: &[u32]| {
		let grid = TriangleGrid::new(positions, to);
		let mut visited = vec![false; positions.len()];
		let mut farthest = 0.0f32;
		for &vertex in from {
			if !std::mem::replace(&mut visited[vertex as usize], true) {
				farthest = farthest.max(grid.distance(positions[vertex as usize]));
			}
		}
		farthest
	};
	one_way(level, full).max(one_way(full, level))
}

/// The `TriangleGrid` struct buckets a triangle list into a uniform grid so [`level_distance`] can find the triangles
/// near a point without testing them all.
struct TriangleGrid<'a> {
	positions: &'a [[f32; 3]],
	indices: &'a [u32],
	origin: [f32; 3],
	cell: f32,
	dimensions: [usize; 3],
	/// Where each cell's triangles start in `triangles`, with one extra entry at the end.
	starts: Vec<u32>,
	triangles: Vec<u32>,
}

impl<'a> TriangleGrid<'a> {
	/// Buckets every triangle of `indices` into each cell its bounding box overlaps.
	fn new(positions: &'a [[f32; 3]], indices: &'a [u32]) -> Self {
		let triangle_count = indices.len() / 3;
		let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
		for &vertex in indices {
			for axis in 0..3 {
				min[axis] = min[axis].min(positions[vertex as usize][axis]);
				max[axis] = max[axis].max(positions[vertex as usize][axis]);
			}
		}
		let extent = (0..3).map(|axis| max[axis] - min[axis]).fold(0.0f32, f32::max);
		// About one cell per triangle along the cube root of the count, so a cell holds a few triangles.
		let cell = (extent / (triangle_count as f32).cbrt().max(1.0)).max(f32::MIN_POSITIVE);
		let dimensions = std::array::from_fn(|axis| (((max[axis] - min[axis]) / cell) as usize + 1).min(256));
		let mut grid = Self {
			positions,
			indices,
			origin: min,
			cell,
			dimensions,
			starts: Vec::new(),
			triangles: Vec::new(),
		};
		// Counts each cell's triangles, turns the counts into starts, then fills the cells.
		let mut starts = vec![0u32; dimensions.iter().product::<usize>() + 1];
		for triangle in 0..triangle_count {
			grid.for_each_cell_of(triangle, |cell| starts[cell + 1] += 1);
		}
		for cell in 1..starts.len() {
			starts[cell] += starts[cell - 1];
		}
		let mut cursor = starts.clone();
		let mut triangles = vec![0u32; *starts.last().unwrap_or(&0) as usize];
		for triangle in 0..triangle_count {
			grid.for_each_cell_of(triangle, |cell| {
				triangles[cursor[cell] as usize] = triangle as u32;
				cursor[cell] += 1;
			});
		}
		grid.starts = starts;
		grid.triangles = triangles;
		grid
	}

	fn cell_of(&self, point: [f32; 3]) -> [usize; 3] {
		std::array::from_fn(|axis| {
			(((point[axis] - self.origin[axis]) / self.cell).max(0.0) as usize).min(self.dimensions[axis] - 1)
		})
	}

	fn cell_index(&self, [x, y, z]: [usize; 3]) -> usize {
		(z * self.dimensions[1] + y) * self.dimensions[0] + x
	}

	fn corners(&self, triangle: usize) -> [[f32; 3]; 3] {
		std::array::from_fn(|corner| self.positions[self.indices[triangle * 3 + corner] as usize])
	}

	/// Calls `visit` with every cell the triangle's bounding box overlaps.
	fn for_each_cell_of(&self, triangle: usize, mut visit: impl FnMut(usize)) {
		let corners = self.corners(triangle);
		let bound = |pick: fn(f32, f32) -> f32, start: f32| {
			std::array::from_fn(|axis| corners.iter().map(|corner| corner[axis]).fold(start, pick))
		};
		let (low, high) = (self.cell_of(bound(f32::min, f32::INFINITY)), self.cell_of(bound(f32::max, f32::NEG_INFINITY)));
		for z in low[2]..=high[2] {
			for y in low[1]..=high[1] {
				for x in low[0]..=high[0] {
					visit(self.cell_index([x, y, z]));
				}
			}
		}
	}

	/// Returns the distance from `point` to the nearest triangle, searching shells of cells outward from the point's
	/// cell until no farther shell can hold a nearer triangle.
	fn distance(&self, point: [f32; 3]) -> f32 {
		let center = self.cell_of(point);
		let max_radius = self.dimensions.iter().copied().max().unwrap_or(1);
		let mut nearest = f32::INFINITY;
		for radius in 0..=max_radius {
			let low: [usize; 3] = std::array::from_fn(|axis| center[axis].saturating_sub(radius));
			let high: [usize; 3] = std::array::from_fn(|axis| (center[axis] + radius).min(self.dimensions[axis] - 1));
			self.visit_shell(low, high, point, &mut nearest);
			// Every cell of the next shell lies at least `radius` cells from the point.
			if nearest <= radius as f32 * self.cell {
				break;
			}
		}
		nearest
	}

	/// Visits the cells on the surface of the box `low..=high`, skipping its interior.
	///
	/// A shell has `O(r²)` cells, so walking its six faces keeps the search at `O(r²)` per radius instead of testing
	/// the `O(r³)` cells of the whole box.
	fn visit_shell(&self, low: [usize; 3], high: [usize; 3], point: [f32; 3], nearest: &mut f32) {
		let (x0, x1) = (low[0], high[0]);
		let (y0, y1) = (low[1], high[1]);
		let (z0, z1) = (low[2], high[2]);
		// The two z faces.
		for z in [z0, z1] {
			for y in y0..=y1 {
				for x in x0..=x1 {
					self.visit_cell([x, y, z], point, nearest);
				}
			}
		}
		// The two y faces, between the z faces.
		for z in (z0 + 1)..z1 {
			for y in [y0, y1] {
				for x in x0..=x1 {
					self.visit_cell([x, y, z], point, nearest);
				}
			}
		}
		// The two x faces, between the z and y faces.
		for z in (z0 + 1)..z1 {
			for y in (y0 + 1)..y1 {
				for x in [x0, x1] {
					self.visit_cell([x, y, z], point, nearest);
				}
			}
		}
	}

	/// Lowers `nearest` to the closest triangle in `cell`.
	fn visit_cell(&self, cell: [usize; 3], point: [f32; 3], nearest: &mut f32) {
		let index = self.cell_index(cell);
		for &triangle in &self.triangles[self.starts[index] as usize..self.starts[index + 1] as usize] {
			*nearest = (*nearest).min(point_triangle_distance(point, self.corners(triangle as usize)));
		}
	}
}

/// Returns the distance from `point` to the closest point of triangle `[a, b, c]`.
fn point_triangle_distance(point: [f32; 3], [a, b, c]: [[f32; 3]; 3]) -> f32 {
	let sub = |u: [f32; 3], v: [f32; 3]| [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
	let dot = |u: [f32; 3], v: [f32; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
	let along = |origin: [f32; 3], direction: [f32; 3], t: f32| {
		[origin[0] + direction[0] * t, origin[1] + direction[1] * t, origin[2] + direction[2] * t]
	};
	// Closest point by Voronoi region, after Ericson's Real-Time Collision Detection, section 5.1.5.
	let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(point, a));
	let (d1, d2) = (dot(ab, ap), dot(ac, ap));
	let closest = if d1 <= 0.0 && d2 <= 0.0 {
		a
	} else {
		let bp = sub(point, b);
		let (d3, d4) = (dot(ab, bp), dot(ac, bp));
		let cp = sub(point, c);
		let (d5, d6) = (dot(ab, cp), dot(ac, cp));
		let (vc, vb, va) = (d1 * d4 - d3 * d2, d5 * d2 - d1 * d6, d3 * d6 - d5 * d4);
		if d3 >= 0.0 && d4 <= d3 {
			b
		} else if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
			along(a, ab, d1 / (d1 - d3))
		} else if d6 >= 0.0 && d5 <= d6 {
			c
		} else if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
			along(a, ac, d2 / (d2 - d6))
		} else if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
			along(b, sub(c, b), (d4 - d3) / ((d4 - d3) + (d5 - d6)))
		} else {
			let sum = va + vb + vc;
			// A triangle without area has no interior; its corner is as good as any point of it.
			if sum == 0.0 {
				a
			} else {
				let on_ab = along(a, ab, vb / sum);
				along(on_ab, ac, vc / sum)
			}
		}
	};
	let offset = sub(point, closest);
	dot(offset, offset).sqrt()
}

/// Share of the previous level's triangles that each level aims to keep.
const LOD_TRIANGLE_RATIO: f32 = 0.5;
/// Smallest share of the previous level's triangles a level must remove to be kept.
pub(super) const LOD_MIN_REDUCTION: f32 = 0.25;
/// Triangle count at or below which a level gets no coarser one, because it already fits in about two meshlets.
const LOD_MIN_TRIANGLES: usize = 2 * MESHLET_MAX_TRIANGLES;
/// Weight of each normal component against position error, which is measured relative to the mesh extent.
const LOD_NORMAL_WEIGHT: f32 = 0.5;

use super::packing::MESHLET_MAX_TRIANGLES;
use crate::resources::mesh::MAX_PRIMITIVE_LODS;

#[cfg(test)]
mod tests {
	use super::*;

	/// The grid search must measure the same distance as testing every triangle.
	#[test]
	fn level_distance_matches_brute_force() {
		const SIDE: usize = 12;
		let mut positions = Vec::new();
		for z in 0..SIDE {
			for x in 0..SIDE {
				positions.push([x as f32 * 0.7, ((x * 7 + z * 3) % 5) as f32 * 0.3, z as f32 * 0.9]);
			}
		}
		let mut full = Vec::new();
		for z in 0..SIDE - 1 {
			for x in 0..SIDE - 1 {
				let a = (z * SIDE + x) as u32;
				let b = a + 1;
				let c = a + SIDE as u32;
				let d = c + 1;
				full.extend([a, b, c, b, d, c]);
			}
		}
		// A coarser level drops every other quad, so the two lists differ.
		let level = full.chunks_exact(6).step_by(2).flatten().copied().collect::<Vec<_>>();
		let brute_one_way = |from: &[u32], to: &[u32]| {
			let mut seen = vec![false; positions.len()];
			let mut farthest = 0.0f32;
			for &vertex in from {
				if !std::mem::replace(&mut seen[vertex as usize], true) {
					let point = positions[vertex as usize];
					let nearest = to
						.chunks_exact(3)
						.map(|triangle| {
							let corners = [triangle[0] as usize, triangle[1] as usize, triangle[2] as usize]
								.map(|vertex| positions[vertex]);
							point_triangle_distance(point, corners)
						})
						.fold(f32::INFINITY, f32::min);
					farthest = farthest.max(nearest);
				}
			}
			farthest
		};
		let expected = brute_one_way(&level, &full).max(brute_one_way(&full, &level));
		let found = level_distance(&positions, &full, &level);
		let tolerance = 1e-4 * expected.max(1.0);
		assert!(
			(found - expected).abs() <= tolerance,
			"level distance found {found}, brute force found {expected}"
		);
	}
}
