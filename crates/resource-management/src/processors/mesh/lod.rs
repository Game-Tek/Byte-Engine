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
/// distance to full detail directly. `normals` holds three components per vertex, or nothing; with them, shading
/// changes no more than the surface does. `vertex_locks` is scratch storage.
pub(super) fn simplify_levels(
	indices: &[u32],
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
			.map_or((indices.len() / 3, 0.0), |level| (level.indices.len() / 3, level.error));
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
			error: (relative_error * scale).max(error),
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

/// Share of the previous level's triangles that each level aims to keep.
const LOD_TRIANGLE_RATIO: f32 = 0.5;
/// Smallest share of the previous level's triangles a level must remove to be kept.
const LOD_MIN_REDUCTION: f32 = 0.25;
/// Triangle count at or below which a level gets no coarser one, because it already fits in about two meshlets.
const LOD_MIN_TRIANGLES: usize = 2 * MESHLET_MAX_TRIANGLES;
/// Weight of each normal component against position error, which is measured relative to the mesh extent.
const LOD_NORMAL_WEIGHT: f32 = 0.5;

use super::packing::MESHLET_MAX_TRIANGLES;
use crate::resources::mesh::MAX_PRIMITIVE_LODS;
