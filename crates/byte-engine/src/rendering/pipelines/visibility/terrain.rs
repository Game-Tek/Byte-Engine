//! Terrain streaming: which nodes of each renderable terrain the camera wants, which ones the frame can draw from the
//! meshes that are resident, and which node meshes nobody needs anymore.
//!
//! [`TerrainCuts`] makes both selections from a node tree alone, so the rules are testable without a GPU.
//! [`TerrainStreams`] keeps every terrain's state across frames and reports what the manager must change in the scene.
//! The rules are specified under [terrains](/docs/reference/rendering#terrains).

#![expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md")]

use std::sync::Arc;

use resource_management::resources::terrain::{Terrain, TerrainNode, TerrainNodeKey, terrain_node_mesh_id};
use utils::hash::HashMap;

use super::lod::LodSpace;
use crate::{
	core::factory::Handle,
	rendering::renderable::mesh::{MeshKey, MeshSource},
};

/// Frames a node mesh stays resident after the last frame a terrain wanted or drew it.
pub(crate) const TERRAIN_RELEASE_FRAMES: u64 = 120;

/// The `NodeCut` enum places one node relative to a cut through a terrain's tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeCut {
	/// An ancestor of the node is in the cut, so the node isn't reached.
	Outside,
	/// The node is in the cut.
	Leaf,
	/// The node is reached and replaced by its children.
	Split,
}

/// The `TerrainCuts` struct holds one terrain entity's two cuts through its node tree, kept between frames so selection
/// allocates only when the terrain changes.
///
/// Each frame, call [`Self::want`] with the camera, then [`Self::draw`] with what is resident.
#[derive(Default)]
pub(crate) struct TerrainCuts {
	/// The nodes the camera wants, by node index.
	wanted: Vec<NodeCut>,
	/// Whether each node is covered: resident, or with every child covered.
	covered: Vec<bool>,
	/// The nodes the frame draws, by node index.
	drawn: Vec<NodeCut>,
}

impl TerrainCuts {
	/// Refines the tree from its root wherever a node's error exceeds what `allowed_error` allows at its bounds, then
	/// refines further until no wanted node borders a refined one whose child along that edge is refined too.
	pub(crate) fn want(&mut self, terrain: &Terrain, allowed_error: impl Fn(&TerrainNode) -> f32) {
		self.wanted.clear();
		self.wanted.resize(terrain.nodes.len(), NodeCut::Outside);
		let root = terrain.root();
		self.wanted[index(terrain, root)] = NodeCut::Leaf;
		refine(terrain, &mut self.wanted, root, &allowed_error);
		// A balancing split can unbalance a coarser neighbor, so it repeats until nothing changes. Only splits are
		// made, so it ends.
		loop {
			let mut changed = false;
			for (node, key) in nodes(terrain) {
				if self.wanted[node] == NodeCut::Leaf
					&& key.level > 0
					&& finer_split_neighbor(terrain, &self.wanted, key, |_| true).is_some()
				{
					self.wanted[node] = NodeCut::Split;
					for child in terrain.children(key) {
						self.wanted[index(terrain, child)] = NodeCut::Leaf;
						refine(terrain, &mut self.wanted, child, &allowed_error);
					}
					changed = true;
				}
			}
			if !changed {
				break;
			}
		}
	}

	/// Returns the index of every node [`Self::want`] reached, refined or not, whose meshes the camera needs.
	pub(crate) fn wanted(&self) -> impl Iterator<Item = usize> + '_ {
		self.wanted
			.iter()
			.enumerate()
			.filter(|(_, cut)| **cut != NodeCut::Outside)
			.map(|(node, _)| node)
	}

	/// Picks the nodes the frame draws from the wanted ones, given whether each node's mesh is `resident`, by node
	/// index, then coarsens seams between drawn nodes two levels apart where the coarser mesh is resident.
	///
	/// Call it after [`Self::want`]. Draws nothing until the root is covered.
	pub(crate) fn draw(&mut self, terrain: &Terrain, resident: impl Fn(usize) -> bool) {
		// Children come before their parents in node order, so one pass covers the tree from level 0 up.
		self.covered.clear();
		self.covered.reserve(terrain.nodes.len());
		for (node, key) in terrain
			.levels
			.iter()
			.zip(0..)
			.flat_map(|(grid, level)| keys(grid, level))
			.enumerate()
		{
			let covered =
				resident(node) || (key.level > 0 && terrain.children(key).all(|child| self.covered[index(terrain, child)]));
			self.covered.push(covered);
		}

		self.drawn.clear();
		self.drawn.resize(terrain.nodes.len(), NodeCut::Outside);
		let root = index(terrain, terrain.root());
		if !self.covered[root] {
			return;
		}
		self.drawn[root] = NodeCut::Leaf;
		for (node, key) in nodes(terrain) {
			if self.drawn[node] != NodeCut::Leaf {
				continue;
			}
			// A covered node that isn't resident is covered by its children, so they draw in its place.
			let children_cover = key.level > 0 && terrain.children(key).all(|child| self.covered[index(terrain, child)]);
			if (self.wanted[node] == NodeCut::Split && children_cover) || !resident(node) {
				self.drawn[node] = NodeCut::Split;
				for child in terrain.children(key) {
					self.drawn[index(terrain, child)] = NodeCut::Leaf;
				}
			}
		}

		// A coarsening joins nodes, which can unbalance the seams of the joined node, so it repeats until nothing
		// changes. Only joins are made, so it ends.
		loop {
			let mut changed = false;
			for (node, key) in nodes(terrain) {
				if self.drawn[node] != NodeCut::Leaf {
					continue;
				}
				if let Some(child) = finer_split_neighbor(terrain, &self.drawn, key, &resident) {
					join(terrain, &mut self.drawn, child);
					changed = true;
				}
			}
			if !changed {
				break;
			}
		}
	}

	/// Returns the index of every node the frame draws, after [`Self::draw`].
	pub(crate) fn drawn(&self) -> impl Iterator<Item = usize> + '_ {
		self.drawn
			.iter()
			.enumerate()
			.filter(|(_, cut)| **cut == NodeCut::Leaf)
			.map(|(node, _)| node)
	}

	/// Returns whether node `node` is drawn, after [`Self::draw`].
	fn draws(&self, node: usize) -> bool {
		self.drawn.get(node) == Some(&NodeCut::Leaf)
	}
}

/// Splits the wanted leaf `key` and its descendants wherever their error exceeds what `allowed_error` allows.
fn refine(terrain: &Terrain, wanted: &mut [NodeCut], key: TerrainNodeKey, allowed_error: &impl Fn(&TerrainNode) -> f32) {
	let node = &terrain.nodes[index(terrain, key)];
	if key.level == 0 || node.error <= allowed_error(node) {
		return;
	}
	wanted[index(terrain, key)] = NodeCut::Split;
	for child in terrain.children(key) {
		wanted[index(terrain, child)] = NodeCut::Leaf;
		refine(terrain, wanted, child, allowed_error);
	}
}

/// Makes the split node `key` a leaf of `cut`, and every node below it unreached.
fn join(terrain: &Terrain, cut: &mut [NodeCut], key: TerrainNodeKey) {
	cut[index(terrain, key)] = NodeCut::Leaf;
	let mut below = terrain.children(key).collect::<smallvec::SmallVec<[TerrainNodeKey; 16]>>();
	while let Some(key) = below.pop() {
		let node = index(terrain, key);
		if cut[node] != NodeCut::Outside {
			cut[node] = NodeCut::Outside;
			below.extend(terrain.children(key));
		}
	}
}

/// Returns a child, which `eligible` accepts by node index, of a split neighbor of `key` at `key`'s level that lies
/// along their shared edge and is split itself, so its children border `key` two levels finer.
fn finer_split_neighbor(
	terrain: &Terrain,
	cut: &[NodeCut],
	key: TerrainNodeKey,
	eligible: impl Fn(usize) -> bool,
) -> Option<TerrainNodeKey> {
	// Each side as the neighbor's offset, then which of the neighbor's child columns and rows faces `key`.
	const SIDES: [((i64, i64), (Option<u32>, Option<u32>)); 4] = [
		((1, 0), (Some(0), None)),
		((-1, 0), (Some(1), None)),
		((0, 1), (None, Some(0))),
		((0, -1), (None, Some(1))),
	];
	SIDES.into_iter().find_map(|((dx, dz), (facing_column, facing_row))| {
		let neighbor = TerrainNodeKey {
			level: key.level,
			column: u32::try_from(i64::from(key.column) + dx).ok()?,
			row: u32::try_from(i64::from(key.row) + dz).ok()?,
		};
		if cut[terrain.index(neighbor)?] != NodeCut::Split {
			return None;
		}
		terrain.children(neighbor).find(|child| {
			facing_column.is_none_or(|column| child.column == neighbor.column * 2 + column)
				&& facing_row.is_none_or(|row| child.row == neighbor.row * 2 + row)
				&& cut[index(terrain, *child)] == NodeCut::Split
				&& eligible(index(terrain, *child))
		})
	})
}

/// Returns every node with its key, from the root down: by level from the coarsest, then by row, then by column.
fn nodes(terrain: &Terrain) -> impl Iterator<Item = (usize, TerrainNodeKey)> + '_ {
	terrain
		.levels
		.iter()
		.zip(0..terrain.levels.len() as u32)
		.rev()
		.flat_map(|(grid, level)| {
			keys(grid, level)
				.zip(grid.first_node as usize..)
				.map(|(key, node)| (node, key))
		})
}

/// Returns the keys of one level's nodes, by row, then by column.
fn keys(grid: &resource_management::resources::terrain::TerrainLevel, level: u32) -> impl Iterator<Item = TerrainNodeKey> {
	let columns = grid.columns;
	(0..grid.rows).flat_map(move |row| (0..columns).map(move |column| TerrainNodeKey { level, column, row }))
}

/// Returns the index of a key that names a node of `terrain`.
fn index(terrain: &Terrain, key: TerrainNodeKey) -> usize {
	terrain.index(key).expect("the key names a node of the terrain")
}

/// The `TerrainStreams` struct keeps every renderable terrain's node tree, cuts, and drawn instances across frames, and
/// when each node mesh was last needed, so the visibility pipeline loads and releases node meshes as the camera moves.
///
/// Add each terrain entity with [`Self::add`], adopt each loaded tree with [`Self::adopt`], and each frame call
/// [`Self::select`], then try [`Self::idle_meshes`] for meshes to release.
#[derive(Default)]
pub(crate) struct TerrainStreams {
	/// Each terrain resource's tree and node meshes, which every entity of the resource shares.
	resources: Vec<TerrainResource>,
	/// Each resource's index in `resources`, by resource ID.
	resource_indices: HashMap<Arc<str>, usize>,
	entities: HashMap<Handle, TerrainEntity>,
	/// Every node mesh some terrain requested and that wasn't released since, as its resource and node index.
	requested: Vec<(u32, u32)>,
}

/// The `TerrainResource` struct is one loaded terrain tree and the mesh of each of its nodes.
#[derive(Default)]
struct TerrainResource {
	terrain: Option<Terrain>,
	meshes: Vec<NodeMesh>,
}

/// The `NodeMesh` struct follows one node's mesh: what to request it by and when it was last needed.
struct NodeMesh {
	id: Arc<str>,
	key: MeshKey,
	/// The last frame a terrain wanted or drew the node, or `None` while it isn't requested.
	last_needed: Option<u64>,
}

/// The `TerrainEntity` struct is one renderable terrain: its resource, its cuts, and the instance of each node it
/// draws.
struct TerrainEntity {
	resource: usize,
	cuts: TerrainCuts,
	/// The instance that draws each node, by node index.
	instances: HashMap<u32, Handle>,
}

/// The `TerrainChange` enum is one change [`TerrainStreams::select`] asks of the scene.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerrainChange {
	/// Add a renderable `instance` of the resident mesh `mesh`, at the transform of the terrain entity `terrain`.
	Add {
		terrain: Handle,
		instance: Handle,
		mesh: MeshKey,
	},
	/// Remove the renderable `instance`.
	Remove(Handle),
}

impl TerrainStreams {
	/// Starts following the terrain entity `entity`, which draws the `Terrain` resource `id`. Returns whether the
	/// resource is new, so its tree must be requested.
	///
	/// Creation messages are upserts, so an entity that already draws a terrain gives back its instances through
	/// `changes` first.
	pub(crate) fn add(&mut self, entity: Handle, id: Arc<str>, changes: &mut Vec<TerrainChange>) -> bool {
		self.remove(entity, changes);
		let new = !self.resource_indices.contains_key(&id);
		let resource = *self.resource_indices.entry(id).or_insert_with(|| {
			self.resources.push(TerrainResource::default());
			self.resources.len() - 1
		});
		self.entities.insert(
			entity,
			TerrainEntity {
				resource,
				cuts: TerrainCuts::default(),
				instances: HashMap::default(),
			},
		);
		new
	}

	/// Stops following `entity`, and asks through `changes` to remove every instance it draws. Does nothing for an
	/// entity that isn't a terrain.
	pub(crate) fn remove(&mut self, entity: Handle, changes: &mut Vec<TerrainChange>) {
		if let Some(entity) = self.entities.remove(&entity) {
			changes.extend(entity.instances.into_values().map(TerrainChange::Remove));
		}
	}

	/// Adopts the loaded tree of the resource `id`, whose node meshes every entity of it draws from the next
	/// [`Self::select`].
	///
	/// A tree that replaces one, as a rebake's reload does, can name other nodes, so the entities that drew the old
	/// one give back their instances through `changes` and select again.
	pub(crate) fn adopt(&mut self, id: &str, terrain: Terrain, changes: &mut Vec<TerrainChange>) {
		let Some(&resource) = self.resource_indices.get(id) else {
			return;
		};
		for entity in self.entities.values_mut().filter(|entity| entity.resource == resource) {
			changes.extend(entity.instances.drain().map(|(_, instance)| TerrainChange::Remove(instance)));
		}
		let meshes = (0..terrain.levels.len() as u32)
			.flat_map(|level| keys(&terrain.levels[level as usize], level))
			.map(|key| {
				let id: Arc<str> = terrain_node_mesh_id(id, key).into();
				NodeMesh {
					key: MeshSource::Resource(id.clone()).key(),
					id,
					last_needed: None,
				}
			})
			.collect();
		self.resources[resource] = TerrainResource {
			terrain: Some(terrain),
			meshes,
		};
		// A reloaded tree can name other nodes, so every entity of it starts over.
		self.requested.retain(|(requested, _)| *requested as usize != resource);
	}

	/// Returns the instances the terrain entity `entity` draws, which follow its transform.
	pub(crate) fn instances(&self, entity: Handle) -> impl Iterator<Item = Handle> + '_ {
		self.entities
			.get(&entity)
			.into_iter()
			.flat_map(|entity| entity.instances.values().copied())
	}

	/// Selects every terrain's nodes for this frame, `frame`, and asks through `changes` for the instances to add and
	/// remove.
	///
	/// `space` returns the camera in a terrain entity's space, or `None` to skip it. `request` asks for a node mesh by
	/// its source, once each time a terrain starts needing a mesh that isn't resident; `resident` says whether a mesh is
	/// resident now. A node mesh whose load fails isn't asked for again until it is released or its tree is adopted
	/// again.
	pub(crate) fn select(
		&mut self,
		frame: u64,
		space: impl Fn(Handle) -> Option<LodSpace>,
		mut request: impl FnMut(MeshSource),
		resident: impl Fn(MeshKey) -> bool,
		changes: &mut Vec<TerrainChange>,
	) {
		for (&handle, entity) in &mut self.entities {
			let resource = &mut self.resources[entity.resource];
			let (Some(terrain), Some(space)) = (resource.terrain.as_ref(), space(handle)) else {
				continue;
			};
			entity.cuts.want(terrain, |node| space.allowed_error(node.min, node.max));
			for node in entity.cuts.wanted() {
				let mesh = &mut resource.meshes[node];
				// The loader keeps a requested mesh loading until it is resident, so a node is asked for only when it
				// starts being needed, and frames that keep wanting it clone nothing.
				if mesh.last_needed.replace(frame).is_none() {
					self.requested.push((entity.resource as u32, node as u32));
					if !resident(mesh.key) {
						request(MeshSource::Resource(mesh.id.clone()));
					}
				}
			}
			entity.cuts.draw(terrain, |node| resident(resource.meshes[node].key));

			let cuts = &entity.cuts;
			entity.instances.retain(|&node, &mut instance| {
				let keep = cuts.draws(node as usize);
				if !keep {
					changes.push(TerrainChange::Remove(instance));
				}
				keep
			});
			for node in cuts.drawn() {
				let mesh = &mut resource.meshes[node];
				// A drawn node finer than the wanted ones is still needed, so it isn't released under the frame.
				if mesh.last_needed.replace(frame).is_none() {
					self.requested.push((entity.resource as u32, node as u32));
				}
				entity.instances.entry(node as u32).or_insert_with(|| {
					let instance = Handle::new();
					changes.push(TerrainChange::Add {
						terrain: handle,
						instance,
						mesh: mesh.key,
					});
					instance
				});
			}
		}
	}

	/// Returns each requested node mesh that no terrain needed for [`TERRAIN_RELEASE_FRAMES`] frames before `frame`, other
	/// than a root, as its slot and mesh. Release it, then report it with [`Self::released`].
	pub(crate) fn idle_meshes(&self, frame: u64) -> impl Iterator<Item = (usize, MeshKey)> + '_ {
		self.requested
			.iter()
			.enumerate()
			.filter_map(move |(slot, &(resource, node))| {
				let resource = &self.resources[resource as usize];
				let terrain = resource.terrain.as_ref()?;
				let mesh = &resource.meshes[node as usize];
				let idle = mesh.last_needed? + TERRAIN_RELEASE_FRAMES < frame;
				(idle && node as usize != terrain.index(terrain.root())?).then_some((slot, mesh.key))
			})
	}

	/// Forgets the requested node mesh in `slot`, from [`Self::idle_meshes`], once it was released, so it is requested
	/// again when a terrain wants it. Report released slots from the highest down, since each report moves the last one.
	pub(crate) fn released(&mut self, slot: usize) {
		let (resource, node) = self.requested.swap_remove(slot);
		self.resources[resource as usize].meshes[node as usize].last_needed = None;
	}
}

#[cfg(test)]
mod tests {
	use std::collections::HashSet;

	use resource_management::resources::terrain::{Terrain, TerrainLevel, TerrainNode, TerrainNodeKey};

	use super::{TERRAIN_RELEASE_FRAMES, TerrainChange, TerrainCuts, TerrainStreams};
	use crate::{
		core::factory::Handle,
		rendering::{pipelines::visibility::lod::LodSpace, renderable::mesh::MeshSource},
	};

	/// Returns a terrain whose levels have `grids` columns and rows from level 0, where a level `k` node is a square of
	/// side `2^k` at its place on its level's grid and strays by `2^k − 1`, and its mesh names its key.
	fn terrain(grids: &[(u32, u32)]) -> Terrain {
		let mut first_node = 0;
		let levels = grids
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
		let nodes = levels
			.iter()
			.zip(0..)
			.flat_map(|(grid, level)| super::keys(grid, level))
			.map(|key| {
				let size = (1u32 << key.level) as f32;
				let (x, z) = (key.column as f32 * size, key.row as f32 * size);
				TerrainNode {
					mesh: format!("{}/{}/{}", key.level, key.column, key.row),
					min: [x, 0.0, z],
					max: [x + size, 0.0, z + size],
					error: size - 1.0,
				}
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

	/// A square tree of four levels with 8 by 8 nodes at level 0.
	fn square() -> Terrain {
		terrain(&[(8, 8), (4, 4), (2, 2), (1, 1)])
	}

	/// Returns the meshes of a list of node indices, which name their keys.
	fn named(terrain: &Terrain, nodes: impl Iterator<Item = usize>) -> HashSet<String> {
		nodes.map(|node| terrain.nodes[node].mesh.clone()).collect()
	}

	/// Returns a node's level, column, and row from its mesh name.
	fn key_of(node: &TerrainNode) -> TerrainNodeKey {
		let [level, column, row] = [0, 1, 2].map(|part| node.mesh.split('/').nth(part).unwrap().parse().unwrap());
		TerrainNodeKey { level, column, row }
	}

	/// Asserts that every level 0 square is drawn exactly once and that drawn neighbors are at most one level apart.
	fn assert_balanced_cover(terrain: &Terrain, cuts: &TerrainCuts) {
		let grid = terrain.levels[0];
		let mut levels = vec![vec![None; grid.columns as usize]; grid.rows as usize];
		for node in cuts.drawn() {
			let key = key_of(&terrain.nodes[node]);
			let size = 1usize << key.level;
			// Coarser nodes of uneven grids reach past the last square; those parts cover nothing.
			for line in levels.iter_mut().skip(key.row as usize * size).take(size) {
				for square in line.iter_mut().skip(key.column as usize * size).take(size) {
					assert_eq!(*square, None, "two drawn nodes cover one square");
					*square = Some(key.level);
				}
			}
		}
		for (row, line) in levels.iter().enumerate() {
			for (column, level) in line.iter().enumerate() {
				let level = level.unwrap_or_else(|| panic!("square ({column}, {row}) isn't drawn"));
				for (right, below) in [(column + 1, row), (column, row + 1)] {
					if let Some(Some(other)) = levels.get(below).and_then(|line| line.get(right)) {
						assert!(
							level.abs_diff(*other) <= 1,
							"levels {level} and {other} meet at ({column}, {row})"
						);
					}
				}
			}
		}
	}

	/// Allows no error in a node whose bounds hold `point` on the XZ plane, and any error elsewhere.
	fn exact_at(point: [f32; 2]) -> impl Fn(&TerrainNode) -> f32 {
		move |node| {
			let inside = (node.min[0]..node.max[0]).contains(&point[0]) && (node.min[2]..node.max[2]).contains(&point[1]);
			if inside { 0.0 } else { f32::INFINITY }
		}
	}

	#[test]
	fn nodes_refine_where_their_error_exceeds_the_allowance() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();

		cuts.want(&terrain, exact_at([0.5, 0.5]));
		cuts.draw(&terrain, |_| true);

		let drawn = named(&terrain, cuts.drawn());
		assert!(drawn.contains("0/0/0") && drawn.contains("0/1/1") && drawn.contains("1/1/0") && drawn.contains("2/1/1"));
		assert!(!drawn.contains("2/0/0"));
		assert_balanced_cover(&terrain, &cuts);
	}

	#[test]
	fn a_wanted_node_beside_one_two_levels_finer_refines() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();

		// Only nodes holding a point just inside the first level 2 node's right edge refine, so level 0 meets the
		// level 2 node to its right until balance refines that one too.
		cuts.want(&terrain, exact_at([3.9, 0.1]));
		cuts.draw(&terrain, |_| true);

		let drawn = named(&terrain, cuts.drawn());
		assert!(drawn.contains("0/3/0"));
		assert!(!drawn.contains("2/1/0") && drawn.contains("1/2/0"));
		assert_balanced_cover(&terrain, &cuts);
	}

	#[test]
	fn every_node_on_the_way_to_a_wanted_leaf_is_wanted() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();

		cuts.want(&terrain, exact_at([0.5, 0.5]));

		let wanted = named(&terrain, cuts.wanted());
		for mesh in ["3/0/0", "2/0/0", "1/0/0", "0/0/0", "2/1/1"] {
			assert!(wanted.contains(mesh), "{mesh} isn't wanted");
		}
		assert!(!wanted.contains("1/2/2"));
	}

	#[test]
	fn nothing_draws_until_the_root_is_covered() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();
		cuts.want(&terrain, |_| 0.0);

		cuts.draw(&terrain, |node| terrain.nodes[node].mesh == "0/0/0");
		assert_eq!(cuts.drawn().count(), 0);

		// Level 0 covers every node above it, the root included.
		cuts.draw(&terrain, |node| terrain.nodes[node].mesh.starts_with("0/"));
		assert_balanced_cover(&terrain, &cuts);
		assert_eq!(cuts.drawn().count(), 64);
	}

	#[test]
	fn a_node_draws_until_all_its_children_are_resident() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();
		cuts.want(&terrain, |_| 0.0);

		cuts.draw(&terrain, |node| terrain.nodes[node].mesh != "0/1/1");

		let drawn = named(&terrain, cuts.drawn());
		assert!(drawn.contains("1/0/0") && !drawn.contains("0/0/0"));
		assert!(drawn.contains("0/2/0"));
		assert_balanced_cover(&terrain, &cuts);
	}

	#[test]
	fn finer_nodes_keep_drawing_until_a_coarser_one_replaces_them() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();
		cuts.want(&terrain, |_| f32::INFINITY);

		// The camera wants only the root, which isn't resident yet, but level 0 still is.
		cuts.draw(&terrain, |node| terrain.nodes[node].mesh.starts_with("0/"));
		assert_eq!(cuts.drawn().count(), 64);

		cuts.draw(&terrain, |node| {
			let mesh = &terrain.nodes[node].mesh;
			mesh.starts_with("3/") || mesh.starts_with("0/")
		});
		assert_eq!(named(&terrain, cuts.drawn()), HashSet::from(["3/0/0".to_string()]));
	}

	#[test]
	fn a_seam_two_levels_apart_coarsens_where_the_coarser_mesh_is_resident() {
		let terrain = square();
		let mut cuts = TerrainCuts::default();
		cuts.want(&terrain, exact_at([3.9, 0.1]));

		// The wanted level 1 nodes right of x = 4 and their level 0 nodes aren't resident, so the level 2 node there
		// draws itself, two levels coarser than the level 0 nodes left of the seam.
		cuts.draw(&terrain, |node| {
			let node = &terrain.nodes[node];
			node.min[0] < 4.0 || node.max[0] - node.min[0] >= 4.0
		});

		let drawn = named(&terrain, cuts.drawn());
		assert!(drawn.contains("2/1/0") && drawn.contains("1/1/0") && !drawn.contains("0/3/0"));
		assert_balanced_cover(&terrain, &cuts);
	}

	#[test]
	fn uneven_grids_balance_against_the_neighbors_they_have() {
		let terrain = terrain(&[(5, 3), (3, 2), (2, 1), (1, 1)]);
		let mut cuts = TerrainCuts::default();

		cuts.want(&terrain, exact_at([4.5, 2.5]));
		cuts.draw(&terrain, |_| true);

		assert!(named(&terrain, cuts.drawn()).contains("0/4/2"));
		assert_balanced_cover(&terrain, &cuts);
	}

	/// Selects frame `frame` for the camera `eye`, which allows `slope` error per unit of distance, with `resident` the
	/// resident meshes, and returns the requested meshes and the changes.
	fn select(
		streams: &mut TerrainStreams,
		frame: u64,
		eye: [f32; 3],
		slope: f32,
		resident: &HashSet<String>,
	) -> (HashSet<String>, Vec<TerrainChange>) {
		let mut requested = HashSet::new();
		let mut changes = Vec::new();
		let keys = resident
			.iter()
			.map(|id| MeshSource::Resource(id.as_str().into()).key())
			.collect::<HashSet<_>>();
		streams.select(
			frame,
			|_| Some(LodSpace::new(eye, slope, 0.0)),
			|source| {
				if let MeshSource::Resource(id) = source {
					requested.insert(id.to_string());
				}
			},
			|key| keys.contains(&key),
			&mut changes,
		);
		(requested, changes)
	}

	/// Returns the node mesh IDs of a terrain resource `id` for the meshes `nodes` name.
	fn ids(id: &str, nodes: &[&str]) -> HashSet<String> {
		nodes.iter().map(|node| format!("{id}#nodes/{node}")).collect()
	}

	#[test]
	fn streams_request_wanted_meshes_and_add_and_remove_instances_as_the_camera_moves() {
		let mut streams = TerrainStreams::default();
		let entity = Handle::new();
		assert!(streams.add(entity, "hills".into(), &mut Vec::new()));
		streams.adopt("hills", square(), &mut Vec::new());

		// Nothing is resident, so only requests go out.
		let (requested, changes) = select(&mut streams, 0, [0.5, 0.0, 0.5], 0.1, &HashSet::new());
		assert!(requested.contains("hills#nodes/3/0/0") && requested.contains("hills#nodes/0/0/0"));
		assert!(changes.is_empty());

		// Once everything is resident, the camera's cut draws.
		let everything = streams.resources[0]
			.meshes
			.iter()
			.map(|mesh| mesh.id.to_string())
			.collect::<HashSet<_>>();
		let (_, changes) = select(&mut streams, 1, [0.5, 0.0, 0.5], 0.1, &everything);
		let added = changes
			.iter()
			.filter(|change| matches!(change, TerrainChange::Add { terrain, .. } if *terrain == entity))
			.count();
		assert_eq!(added, streams.instances(entity).count());
		assert!(added > 1);

		// A camera far away draws the root alone, so every other instance goes.
		let (_, changes) = select(&mut streams, 2, [1000.0, 0.0, 1000.0], 0.1, &everything);
		let removed = changes
			.iter()
			.filter(|change| matches!(change, TerrainChange::Remove(_)))
			.count();
		assert_eq!(removed, added);
		assert_eq!(streams.instances(entity).count(), 1);
	}

	#[test]
	fn meshes_nobody_needs_go_idle_after_the_release_frames_except_the_root() {
		let mut streams = TerrainStreams::default();
		streams.add(Handle::new(), "hills".into(), &mut Vec::new());
		streams.adopt("hills", square(), &mut Vec::new());
		let everything = streams.resources[0]
			.meshes
			.iter()
			.map(|mesh| mesh.id.to_string())
			.collect::<HashSet<_>>();
		select(&mut streams, 0, [0.5, 0.0, 0.5], 0.1, &everything);

		// From frame 1 the camera is far away, so only the root stays needed.
		for frame in 1..=TERRAIN_RELEASE_FRAMES {
			select(&mut streams, frame, [1000.0, 0.0, 1000.0], 0.1, &everything);
			assert_eq!(streams.idle_meshes(frame).count(), 0);
		}
		let frame = TERRAIN_RELEASE_FRAMES + 1;
		let idle = streams.idle_meshes(frame).collect::<Vec<_>>();
		let root = MeshSource::Resource("hills#nodes/3/0/0".into()).key();
		assert!(!idle.is_empty() && idle.iter().all(|(_, mesh)| *mesh != root));

		// Released meshes are requested again when the camera returns.
		let mut slots = idle.iter().map(|(slot, _)| *slot).collect::<Vec<_>>();
		slots.sort_unstable_by(|a, b| b.cmp(a));
		for slot in slots {
			streams.released(slot);
		}
		assert_eq!(streams.idle_meshes(frame).count(), 0);
		let (requested, _) = select(&mut streams, frame, [0.5, 0.0, 0.5], 0.1, &ids("hills", &["3/0/0"]));
		assert!(requested.contains("hills#nodes/0/0/0"));
	}

	#[test]
	fn deleting_a_terrain_entity_removes_every_instance_it_draws() {
		let mut streams = TerrainStreams::default();
		let entity = Handle::new();
		streams.add(entity, "hills".into(), &mut Vec::new());
		streams.adopt("hills", square(), &mut Vec::new());
		let everything = streams.resources[0]
			.meshes
			.iter()
			.map(|mesh| mesh.id.to_string())
			.collect::<HashSet<_>>();
		select(&mut streams, 0, [0.5, 0.0, 0.5], 0.1, &everything);
		let instances = streams.instances(entity).collect::<HashSet<_>>();

		let mut changes = Vec::new();
		streams.remove(entity, &mut changes);

		let removed = changes
			.into_iter()
			.map(|change| match change {
				TerrainChange::Remove(instance) => instance,
				TerrainChange::Add { .. } => panic!("a removal adds nothing"),
			})
			.collect::<HashSet<_>>();
		assert_eq!(removed, instances);
	}
}
