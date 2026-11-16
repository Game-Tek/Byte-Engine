//! A mesh the palette spawned and the pointer carries until a click or a release places it.
//!
//! This module holds the carried mesh's state and the pure parts of placing it: the level it spawns as, and where a
//! pointer's ray lands when nothing physical is under it. Spawning, moving, and deleting it are the editor's, through
//! the inspector.

use crate::core::factory::Handle;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::math::{Point, Ray};

/// The `Carry` struct is one mesh the pointer carries, from its spawn until it's placed or cancelled.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Carry {
	/// The spawned level's root; deleting it removes the mesh.
	pub(crate) root: Handle,
	/// The mesh's entity, the one handle after the root.
	pub(crate) entity: Handle,
	/// Whether the mesh was dragged out of the palette, so releasing the button places it. Otherwise it's pinned to the
	/// pointer, and the next press places it.
	pub(crate) held: bool,
	/// The physics query asked for the surface under the pointer, while it waits for its answer.
	pub(crate) query: Option<Handle>,
	/// Where the mesh was last moved to.
	pub(crate) position: Point,
}

impl Carry {
	/// Starts carrying the mesh spawned as the level whose root is `root`, at `position`.
	pub(crate) fn new(root: Handle, position: Point, held: bool) -> Self {
		Self {
			root,
			entity: level_entity_handle_from_root(root, 0),
			held,
			query: None,
			position,
		}
	}
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
