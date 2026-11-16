//! Baked terrains: a height map split into a tree of node meshes that load one at a time.
//!
//! Author a `.terrain.bead` file and bake it into a [`Terrain`]. Each [`TerrainNode`] names the mesh of one square of
//! the terrain at one level of detail. Start at [`Terrain::root`], and load a node's [`Terrain::children`] where its
//! [`TerrainNode::error`] is too large for the camera, so only the nodes the camera needs are resident.
//!
//! The rules the bake follows are specified in the [terrain reference](/docs/reference/terrain).

/// The `Terrain` struct lets a renderer or streamer find the node meshes that cover a terrain at the detail each part
/// of it needs, without loading any of them first.
///
/// Request it by the `.terrain.bead` file's ID. Its meshes load on their own: request each node's
/// [`TerrainNode::mesh`] as the camera reaches it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Terrain {
	/// The height map's samples along X, then along Z.
	pub samples: [u32; 2],
	/// Meters between neighboring samples.
	pub spacing: f32,
	/// The heights, in meters, of a sample value of 0 and of 1.
	pub heights: [f32; 2],
	/// Quads along each side of a node.
	pub tile_size: u32,
	/// The node grid of each level, from level 0, the finest.
	pub levels: Vec<TerrainLevel>,
	/// Every node, by level from level 0, then by row, then by column.
	pub nodes: Vec<TerrainNode>,
}

super::impl_direct_resource!(Terrain, "Terrain", references = terrain_references);

/// Lists the mesh of every node, in node order.
fn terrain_references(terrain: &Terrain) -> Vec<&str> {
	terrain.nodes.iter().map(|node| node.mesh.as_str()).collect()
}

/// The `TerrainLevel` struct locates one level's nodes in [`Terrain::nodes`], so a node's place in the tree is found
/// by arithmetic instead of a search.
#[derive(
	Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct TerrainLevel {
	/// Nodes along X.
	pub columns: u32,
	/// Nodes along Z.
	pub rows: u32,
	/// The index of the level's first node in [`Terrain::nodes`].
	pub first_node: u32,
}

/// The `TerrainNode` struct describes one square of a terrain at one level of detail, so a streamer can decide whether
/// to draw it, refine it into its children, or leave it unloaded before reading its mesh.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct TerrainNode {
	/// The ID of the node's `Mesh` resource, `<terrain>#nodes/<level>/<column>/<row>`.
	pub mesh: String,
	/// The lowest corner of the box that holds the node and every node below it, in terrain space.
	pub min: [f32; 3],
	/// The highest corner of that box.
	pub max: [f32; 3],
	/// The largest vertical distance, in meters, between the node's surface and the height map below it. Never smaller
	/// than any child's.
	pub error: f32,
}

/// The `TerrainNodeKey` struct names a node by its place in the tree, the way its mesh ID does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerrainNodeKey {
	/// The node's level, 0 for the finest.
	pub level: u32,
	pub column: u32,
	pub row: u32,
}

impl Terrain {
	/// Returns the node at `key`, or `None` when the terrain has no such node.
	pub fn node(&self, key: TerrainNodeKey) -> Option<&TerrainNode> {
		self.nodes.get(self.index(key)?)
	}

	/// Returns where `key` is in [`Self::nodes`], or `None` when the terrain has no such node. A streamer keeps its
	/// per-node state in lists in the same order.
	pub fn index(&self, key: TerrainNodeKey) -> Option<usize> {
		node_index(&self.levels, key)
	}

	/// Returns the key of the root, the coarsest node, which covers the whole terrain.
	///
	/// Next, test it against the camera and descend with [`Self::children`].
	pub fn root(&self) -> TerrainNodeKey {
		TerrainNodeKey {
			level: self.levels.len().saturating_sub(1) as u32,
			column: 0,
			row: 0,
		}
	}

	/// Returns the keys of the nodes one level finer that together cover `key`. Level 0 nodes have none.
	pub fn children(&self, key: TerrainNodeKey) -> impl Iterator<Item = TerrainNodeKey> + '_ {
		child_keys(&self.levels, key)
	}
}

/// Returns the keys of the nodes of `levels` one level finer than `key` that together cover it.
pub(crate) fn child_keys(levels: &[TerrainLevel], key: TerrainNodeKey) -> impl Iterator<Item = TerrainNodeKey> + '_ {
	let finer = key
		.level
		.checked_sub(1)
		.and_then(|level| Some((level, levels.get(level as usize)?)));
	finer.into_iter().flat_map(move |(level, grid)| {
		[(0, 0), (1, 0), (0, 1), (1, 1)].into_iter().filter_map(move |(x, z)| {
			let (column, row) = (key.column * 2 + x, key.row * 2 + z);
			(column < grid.columns && row < grid.rows).then_some(TerrainNodeKey { level, column, row })
		})
	})
}

/// Returns where `key` is in the node list `levels` describes, or `None` when there's no such node.
pub(crate) fn node_index(levels: &[TerrainLevel], key: TerrainNodeKey) -> Option<usize> {
	let grid = levels.get(key.level as usize)?;
	(key.column < grid.columns && key.row < grid.rows).then(|| (grid.first_node + key.row * grid.columns + key.column) as usize)
}

/// Returns the ID of the mesh of the node at `key` of the terrain `terrain`.
pub fn terrain_node_mesh_id(terrain: &str, key: TerrainNodeKey) -> String {
	format!(
		"{terrain}#{TERRAIN_NODES_FRAGMENT_PREFIX}{}/{}/{}",
		key.level, key.column, key.row
	)
}

/// The fragment prefix that names one node's mesh, as `nodes/<level>/<column>/<row>`.
pub const TERRAIN_NODES_FRAGMENT_PREFIX: &str = "nodes/";

#[cfg(test)]
mod tests {
	use super::{Terrain, TerrainLevel, TerrainNode, TerrainNodeKey};

	fn terrain(levels: &[(u32, u32)]) -> Terrain {
		let mut first_node = 0;
		let levels = levels
			.iter()
			.map(|&(columns, rows)| {
				let level = TerrainLevel {
					columns,
					rows,
					first_node,
				};
				first_node += columns * rows;
				level
			})
			.collect::<Vec<_>>();
		let nodes = (0..first_node)
			.map(|index| TerrainNode {
				mesh: index.to_string(),
				min: [0.0; 3],
				max: [0.0; 3],
				error: 0.0,
			})
			.collect();
		Terrain {
			samples: [0; 2],
			spacing: 1.0,
			heights: [0.0, 1.0],
			tile_size: 4,
			levels,
			nodes,
		}
	}

	fn key(level: u32, column: u32, row: u32) -> TerrainNodeKey {
		TerrainNodeKey { level, column, row }
	}

	#[test]
	fn nodes_are_found_by_level_row_and_column() {
		let terrain = terrain(&[(3, 2), (2, 1), (1, 1)]);

		assert_eq!(terrain.node(key(0, 2, 1)).unwrap().mesh, "5");
		assert_eq!(terrain.node(key(1, 1, 0)).unwrap().mesh, "7");
		assert_eq!(terrain.node(terrain.root()).unwrap().mesh, "8");
		assert!(terrain.node(key(0, 3, 0)).is_none());
		assert!(terrain.node(key(3, 0, 0)).is_none());
	}

	#[test]
	fn children_cover_their_parent_and_stop_at_the_grid_edge() {
		let terrain = terrain(&[(3, 2), (2, 1), (1, 1)]);

		assert_eq!(
			terrain.children(key(1, 1, 0)).collect::<Vec<_>>(),
			[key(0, 2, 0), key(0, 2, 1)]
		);
		assert_eq!(terrain.children(terrain.root()).count(), 2);
		assert_eq!(terrain.children(key(0, 0, 0)).count(), 0);
	}
}
