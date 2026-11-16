//! A mesh or prefab from the palette that the pointer carries until a click or a release places it.
//!
//! This module holds the carry's state and the pure parts of placing it: the level a mesh spawns as, and where a
//! pointer's ray lands when nothing physical is under it. Spawning, moving, and deleting are the editor's, through the
//! inspector.

use crate::core::factory::Handle;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::math::{Point, Ray};

/// The `Load` enum is what a carry holds, which decides when it spawns.
#[derive(Debug)]
pub(crate) enum Load {
	/// A mesh, spawned when the carry starts so it follows the pointer: the spawned level's root, which deleting
	/// removes the mesh by, and the mesh's entity, the one handle after the root.
	Mesh { root: Handle, entity: Handle },
	/// A stored prefab, by ID, spawned only where it's placed: the spawner bakes the placement into each of the
	/// prefab's entities, so they can't follow the pointer as one.
	Prefab(String),
}

/// The `Carry` struct is one mesh or prefab the pointer carries, from the palette until it's placed or cancelled.
#[derive(Debug)]
pub(crate) struct Carry {
	pub(crate) load: Load,
	/// Whether the load was dragged out of the palette, so releasing the button places it. Otherwise it's pinned to the
	/// pointer, and the next press places it.
	pub(crate) held: bool,
	/// The physics query asked for the surface under the pointer, while it waits for its answer.
	pub(crate) query: Option<Handle>,
	/// Where the pointer last carried the load to.
	pub(crate) position: Point,
}

impl Carry {
	/// Starts carrying the mesh spawned as the level whose root is `root`, at `position`.
	pub(crate) fn mesh(root: Handle, position: Point, held: bool) -> Self {
		let entity = level_entity_handle_from_root(root, 0);
		Self::new(Load::Mesh { root, entity }, position, held)
	}

	/// Starts carrying the stored prefab `id`, to spawn where it's placed, from `position`.
	pub(crate) fn prefab(id: &str, position: Point, held: bool) -> Self {
		Self::new(Load::Prefab(id.to_string()), position, held)
	}

	fn new(load: Load, position: Point, held: bool) -> Self {
		Self {
			load,
			held,
			query: None,
			position,
		}
	}
}

/// Returns whether the palette result `resource` is a stored prefab rather than a mesh.
pub(crate) fn is_prefab(resource: &str) -> bool {
	resource.ends_with(".prefab")
}

/// Returns the source of a level of one entity named `name`: the mesh `resource` at `position`.
pub(crate) fn level_source(name: &str, resource: &str, position: Point) -> String {
	serde_json::json!({
		"entities": [{
			"components": {
				"name": name,
				"transform": { "position": position.to_array() },
				"mesh": { "resource": resource },
			}
		}]
	})
	.to_string()
}

/// Returns where `ray` meets the ground, the plane through the world's origin facing up, or `None` when it points
/// level or up.
///
/// The carried mesh lands there while nothing physical is under the pointer.
pub(crate) fn ground_point(ray: Ray) -> Option<Point> {
	let (origin, direction) = (ray.origin(), ray.direction());
	if direction.y() > -1e-4 {
		return None;
	}
	Some(ray.point_at(-origin.y() / direction.y()))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::math::Vector;

	#[test]
	fn a_downward_ray_lands_on_the_ground_and_a_level_one_does_not() {
		let down = Ray::new(Point::new(1.0, 2.0, 0.0), Vector::new(0.0, -1.0, 1.0).normalized().unwrap());
		let landed = ground_point(down).expect("a downward ray meets the ground").to_array();
		for (got, want) in landed.into_iter().zip([1.0, 0.0, 2.0]) {
			assert!((got - want).abs() < 1e-5, "landed at {landed:?}");
		}

		let level = Ray::new(Point::new(0.0, 2.0, 0.0), Vector::new(0.0, 0.0, 1.0).normalized().unwrap());
		assert!(ground_point(level).is_none());
	}

	#[test]
	fn the_spawned_level_holds_one_named_mesh_at_its_position() {
		let source: serde_json::Value =
			serde_json::from_str(&level_source("crate", "props/crate.glb", Point::new(1.0, 0.0, 2.0))).unwrap();
		let components = &source["entities"][0]["components"];
		assert_eq!(components["name"], "crate");
		assert_eq!(components["mesh"]["resource"], "props/crate.glb");
		assert_eq!(components["transform"]["position"], serde_json::json!([1.0, 0.0, 2.0]));
	}
}
