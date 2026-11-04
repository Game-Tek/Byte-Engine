//! The meshes and lights of a level, as the lightmap bake traces them.
//!
//! The bake spawns the level into a world of its own with the engine's level kinds, so it reads the same components
//! the renderer reads when the level spawns in an application: the same meshes, the same world transforms, and the
//! same lights, encoded as the renderer encodes them.

use resource_management::resources::level::Level;
use resource_management::{ResourceManager, resource::read_target::ReadTargetsMut};

use crate::core::{
	factory::{CreateMessage, Handle},
	listener::{DefaultListener, Listener},
	message_bus::MessageScope,
};
use crate::gameplay::{Spawner, Transform};
use crate::rendering::lights::Lights;
use crate::rendering::pipelines::visibility::{
	scene::light_data, shader_data::LightData, shadow_selection::LightShadow,
};
use crate::rendering::renderable::mesh::{MeshSource, RenderableMesh};
use crate::rendering::{ConeLight, DirectionalLight, Lightmap, PointLight};

/// The `LevelScene` struct is everything a lightmap bake reads from its level.
///
/// [`collect_level`] makes it. The bake lights every primitive of [`Self::meshes`] that has lightmap UVs, and stores
/// [`Self::holder`] so the renderer finds the entities again when the level spawns.
pub(crate) struct LevelScene {
	/// The spawn index of the entity that holds the `lightmap` kind naming the lightmap being baked.
	pub(crate) holder: u32,
	/// Every mesh entity, in ascending spawn index.
	pub(crate) meshes: Vec<SceneMesh>,
	/// Every light, encoded as the renderer uploads it, without shadow maps.
	pub(crate) lights: Vec<LightData>,
}

/// The `SceneMesh` struct is one mesh entity of a level: where it spawns and what it draws.
pub(crate) struct SceneMesh {
	/// The entity's spawn index.
	pub(crate) entity: u32,
	pub(crate) source: MeshSource,
	/// The entity's world transform; identity when it has none, as the renderer draws it.
	pub(crate) transform: Transform,
}

/// Loads the baked level `id` and its payload from `resources`, or returns the most likely cause of why it can't.
pub(crate) async fn load_level(resources: &ResourceManager, id: &str) -> Result<(Level, Vec<u8>), String> {
	let mut level = resources
		.request::<Level>(id)
		.await
		.map_err(|error| format!("that the level '{id}' isn't baked ({error})"))?;
	let payload = level
		.load(ReadTargetsMut::backing_storage())
		.await
		.map_err(|_| format!("that the payload of the level '{id}' can't be read"))?;
	let payload = payload
		.buffer()
		.ok_or_else(|| format!("that the payload of the level '{id}' isn't held in memory"))?
		.to_vec();
	Ok((level.resource().clone(), payload))
}

/// Spawns `level`, whose loaded payload is `payload`, into a world of its own and collects its meshes and lights, and
/// the entity whose `lightmap` kind names `lightmap_id`.
///
/// Kinds the engine doesn't know are skipped, since only the engine's kinds draw or light anything. Returns the most
/// likely cause of what is wrong with the level.
pub(crate) fn collect_level(lightmap_id: &str, level: &Level, payload: &[u8]) -> Result<LevelScene, String> {
	let mut spawner = Spawner::with_engine_kinds();
	for kind in &level.kinds {
		// An application kind holds nothing the bake reads; an engine kind is already registered and stays.
		let _ = spawner.ignore(kind.name.clone());
	}
	let world = MessageScope::standalone("lightmap bake");
	// Listeners see only what is published after they exist, so they come first.
	let mut meshes = world.factory::<RenderableMesh>().listener();
	let mut transforms = world.factory::<Transform>().listener();
	let mut directional_lights = world.factory::<DirectionalLight>().listener();
	let mut point_lights = world.factory::<PointLight>().listener();
	let mut cone_lights = world.factory::<ConeLight>().listener();
	let mut lightmaps = world.factory::<Lightmap>().listener();
	let root = spawner
		.spawn(&world, level, payload)
		.map_err(|error| format!("that the level doesn't spawn: {error}"))?;
	// A spawn places entity `n` at the `n`-th handle after the root.
	let entity = |handle: Handle| handle.id() - root.id() - 1;

	let transforms = drain(&mut transforms);
	let transform_of = |handle: Handle| {
		transforms
			.iter()
			.find(|(transform_handle, _)| *transform_handle == handle)
			.map_or_else(Transform::default, |(_, transform)| transform.clone())
	};

	let holders = drain(&mut lightmaps)
		.into_iter()
		.filter(|(_, lightmap)| lightmap.resource_id() == lightmap_id)
		.map(|(handle, _)| entity(handle))
		.collect::<Vec<_>>();
	let holder = match holders.as_slice() {
		[holder] => *holder,
		[] => {
			return Err(format!(
				"that no entity of the level has a `lightmap` component naming '{lightmap_id}', so the renderer would never load it"
			));
		}
		_ => {
			return Err(format!(
				"that {} entities of the level have a `lightmap` component naming '{lightmap_id}', where one must",
				holders.len()
			));
		}
	};

	let meshes = drain(&mut meshes)
		.into_iter()
		.map(|(handle, mesh)| SceneMesh {
			entity: entity(handle),
			source: mesh.source().clone(),
			transform: transform_of(handle),
		})
		.collect();

	let mut lights = Vec::new();
	let mut add_light = |handle: Handle, light: Lights| {
		lights.push(light_data(&light, &transform_of(handle), LightShadow::None, None));
	};
	for (handle, light) in drain(&mut directional_lights) {
		add_light(handle, light.into());
	}
	for (handle, light) in drain(&mut point_lights) {
		add_light(handle, light.into());
	}
	for (handle, light) in drain(&mut cone_lights) {
		add_light(handle, light.into());
	}

	Ok(LevelScene { holder, meshes, lights })
}

/// Reads every component `listener` saw created, with its entity's handle, in publication order.
fn drain<T: Clone + Send + Sync + 'static>(listener: &mut DefaultListener<CreateMessage<T>>) -> Vec<(Handle, T)> {
	std::iter::from_fn(|| listener.read())
		.map(|creation| (creation.handle(), creation.into_data()))
		.collect()
}

#[cfg(test)]
mod tests {
	use resource_management::asset::handler::implementations::level::bake_level;

	use math::Point;

	use super::collect_level;
	use crate::gameplay::Transform;
	use crate::rendering::renderable::mesh::MeshSource;

	const LEVEL: &str = r#"{ entities: [
		{ components: { name: "sun", directional_light: { color: { kelvin: 5500 }, intensity: { illuminance: { lux: 1000 } } } } },
		{ components: { transform: { position: [0, 0, 2] }, mesh: { box: { half_extents: [1, 1, 1] } } } },
		{ components: { mesh: { resource: "crate.glb" }, team: 3 } },
		{ components: { lightmap: "arena.lightmap.bead" } },
	] }"#;

	#[test]
	fn a_level_yields_its_meshes_lights_and_lightmap_holder_by_spawn_index() {
		let (level, payload) = bake_level(LEVEL.as_bytes()).expect("the test level should bake");

		let scene = collect_level("arena.lightmap.bead", &level, &payload).expect("the level should be collected");

		assert_eq!(scene.holder, 3);
		assert_eq!(scene.lights.len(), 1);
		let entities: Vec<_> = scene.meshes.iter().map(|mesh| mesh.entity).collect();
		assert_eq!(entities, [1, 2]);
		assert_eq!(scene.meshes[0].transform, Transform::from_position(Point::new(0.0, 0.0, 2.0)));
		assert!(matches!(&scene.meshes[1].source, MeshSource::Resource(id) if id.as_ref() == "crate.glb"));
	}

	#[test]
	fn a_level_that_doesnt_name_the_lightmap_fails() {
		let (level, payload) = bake_level(LEVEL.as_bytes()).expect("the test level should bake");

		assert!(collect_level("other.lightmap.bead", &level, &payload).is_err());
	}
}
