//! A mesh, prefab, or light from the palette that the pointer carries until a click or a release places it.
//!
//! This module holds the carry's state and the pure parts of placing it: the lights the palette offers, the level a
//! mesh or light spawns as, and where a pointer's ray lands when nothing physical is under it. Spawning, moving, and
//! deleting are the editor's, through the inspector.

use crate::core::factory::Handle;
use crate::gameplay::Transform;
use crate::gameplay::level::level_entity_handle_from_root;
use crate::math::{Orientation, Point, Ray, Vector};

/// The `Light` struct is one kind of light the palette places, with the settings a new one starts with.
#[derive(Debug, PartialEq)]
pub(crate) struct Light {
	/// What the palette calls it.
	pub(crate) name: &'static str,
	/// The level kind that spawns it, such as `point_light`.
	pub(crate) kind: &'static str,
	/// The kind's payload, as a level writes it.
	pub(crate) settings: &'static str,
	/// How high above the surface under the pointer it's carried and placed, in meters.
	pub(crate) lift: f32,
	/// Which way it faces, as `[x, y, z, w]`; a light shines along its local +Z.
	pub(crate) orientation: [f32; 4],
}

/// Every kind of light the engine has, as the palette offers them: a warm bulb above the floor, a spot shining
/// straight down from overhead, and a daylight sun pitched 60 degrees down.
pub(crate) const LIGHTS: [Light; 3] = [
	Light {
		name: "Point light",
		kind: "point_light",
		settings: r#"{ "color": { "kelvin": 3000 }, "intensity": { "luminous_flux": { "lumens": 800 } } }"#,
		lift: 1.5,
		orientation: [0.0, 0.0, 0.0, 1.0],
	},
	Light {
		name: "Cone light",
		kind: "cone_light",
		settings: r#"{ "color": { "kelvin": 4000 }, "intensity": { "luminous_intensity": { "candela": 900 } }, "inner_angle": 0.3, "outer_angle": 0.5 }"#,
		lift: 3.0,
		// A quarter turn about +X turns +Z to face down.
		orientation: [std::f32::consts::FRAC_1_SQRT_2, 0.0, 0.0, std::f32::consts::FRAC_1_SQRT_2],
	},
	Light {
		name: "Directional light",
		kind: "directional_light",
		settings: r#"{ "color": { "kelvin": 6500 }, "intensity": { "illuminance": { "lux": 10000 } } }"#,
		lift: 5.0,
		// 60 degrees about +X: half angles of 30 degrees.
		orientation: [0.5, 0.0, 0.0, 0.866_025_4],
	},
];

/// The `Load` enum is what a carry holds, which decides when it spawns.
#[derive(Debug)]
pub(crate) enum Load {
	/// A mesh or light, spawned when the carry starts so it follows the pointer: the spawned level's root, which
	/// deleting removes it by, and its entity, the one handle after the root.
	Spawned { root: Handle, entity: Handle },
	/// A stored prefab, by ID, spawned only where it's placed: the spawner bakes the placement into each of the
	/// prefab's entities, so they can't follow the pointer as one.
	Prefab(String),
}

/// The `Carry` struct is one mesh, prefab, or light the pointer carries, from the palette until it's placed or
/// cancelled.
#[derive(Debug)]
pub(crate) struct Carry {
	pub(crate) load: Load,
	/// Whether the load was dragged out of the palette, so releasing the button places it. Otherwise it's pinned to the
	/// pointer, and the next press places it.
	pub(crate) held: bool,
	/// The physics query asked for the surface under the pointer, while it waits for its answer.
	pub(crate) query: Option<Handle>,
	/// The surface point the pointer last carried the load to.
	pub(crate) position: Point,
	/// How high above that point the load sits, and which way it faces; a light's, or none for a mesh or prefab.
	pub(crate) light: Option<&'static Light>,
}

impl Carry {
	/// Starts carrying the mesh or `light` spawned as the level whose root is `root`, from `position`.
	pub(crate) fn spawned(root: Handle, position: Point, light: Option<&'static Light>, held: bool) -> Self {
		let entity = level_entity_handle_from_root(root, 0);
		Self {
			load: Load::Spawned { root, entity },
			held,
			query: None,
			position,
			light,
		}
	}

	/// Starts carrying the stored prefab `id`, to spawn where it's placed, from `position`.
	pub(crate) fn prefab(id: &str, position: Point, held: bool) -> Self {
		Self {
			load: Load::Prefab(id.to_string()),
			held,
			query: None,
			position,
			light: None,
		}
	}
}

/// Where a load sits over the surface point `position`: a `light` lifted and turned as it says, anything else on the
/// point, facing as authored.
pub(crate) fn placed(position: Point, light: Option<&Light>) -> Transform {
	match light {
		Some(light) => Transform::from_position(position + Vector::new(0.0, light.lift, 0.0))
			.rotation(Orientation::from_unit_array_unchecked(light.orientation)),
		None => Transform::from_position(position),
	}
}

/// Returns the source of a level of one entity named `name` at `transform`, holding the component `kind` with the
/// payload `settings`, such as a mesh's `{ "resource": … }` or a light's.
pub(crate) fn level_source(name: &str, transform: &Transform, kind: &str, settings: serde_json::Value) -> String {
	serde_json::json!({
		"entities": [{
			"components": {
				"name": name,
				"transform": {
					"position": transform.get_position().to_array(),
					"orientation": transform.get_orientation().to_array(),
				},
				kind: settings,
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
	fn the_spawned_level_holds_one_named_entity_with_its_component() {
		let transform = Transform::from_position(Point::new(1.0, 0.0, 2.0));
		let settings = serde_json::json!({ "resource": "props/crate.glb" });
		let source: serde_json::Value = serde_json::from_str(&level_source("crate", &transform, "mesh", settings)).unwrap();
		let components = &source["entities"][0]["components"];
		assert_eq!(components["name"], "crate");
		assert_eq!(components["mesh"]["resource"], "props/crate.glb");
		assert_eq!(components["transform"]["position"], serde_json::json!([1.0, 0.0, 2.0]));
	}

	#[test]
	fn every_light_spawns_as_its_kind_facing_as_set() {
		use resource_management::asset::handler::implementations::level::bake_level;

		use crate::gameplay::level::{Loaded, Placement};

		let world = crate::core::message_bus::MessageScope::standalone("world");
		let spawner = crate::gameplay::Spawner::with_engine_kinds();
		for light in &LIGHTS {
			let transform = placed(Point::origin(), Some(light));
			let settings = serde_json::from_str(light.settings).unwrap();
			let source = level_source(light.name, &transform, light.kind, settings);
			let (level, payload) = bake_level(source.as_bytes()).unwrap_or_else(|error| panic!("{}: {error}", light.name));
			spawner
				.spawn(&world, &Loaded::level("light.level", level, payload), Placement::default())
				.unwrap_or_else(|error| panic!("{} didn't spawn: {error}", light.name));
		}
		let down: Vector = placed(Point::origin(), Some(&LIGHTS[1]))
			.get_orientation()
			.rotate_vector(Vector::new(0.0, 0.0, 1.0));
		assert!(down.y() < -0.999, "the cone light shines down, not along {down:?}");
	}
}
