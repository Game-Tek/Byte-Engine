//! What the inspector adds to and lists from a running application's content: levels spawned from their source, the
//! IDs of stored resources, and, in development builds, the source assets themselves.
//!
//! These need parts the application creates after the inspector, so the application hands them over once they exist:
//! its spawner with [`DefaultInspector::set_spawner`], its resource manager with
//! [`DefaultInspector::set_resource_manager`]. The rules are specified under
//! [Spawning levels](/docs/reference/inspector#spawning-levels), [Resources](/docs/reference/inspector#resources), and
//! [Source assets](/docs/reference/inspector#source-assets).

#![expect(clippy::disallowed_types, reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md")]

use std::sync::Arc;

use resource_management::{
	ResourceManager,
	asset::{ImageSetting, ResolvedImageSettings, handler::implementations::level::bake_level},
	resource::storage_backend::Query,
	resources::mesh::Mesh,
};

use super::DefaultInspector;
use crate::{
	core::{EntityHandle, factory::Handle, message_bus::MessageScope},
	gameplay::{
		Spawner, Transform,
		level::{Loaded, Placement},
	},
};

/// The `Spawned` struct is a level [`DefaultInspector::spawn_level_in`] spawned, kept so a tool can trace the
/// entities a query finds back to the level's source.
pub struct Spawned {
	/// The level's root; deleting it removes everything the spawn added.
	pub root: Handle,
	/// The level and the prefabs it placed, which [`Loaded::placer`] reads.
	pub loaded: Loaded,
	/// The position in the source's `entities` of each entity, by spawn index.
	pub order: Vec<usize>,
}

impl DefaultInspector {
	/// Gives the inspector the spawner levels spawn with, replacing any earlier one.
	///
	/// Give it the same spawner the application spawns its own levels with, so a spawned level knows the
	/// application's kinds. Next, spawn with [`Self::spawn_level`] or [`Self::spawn_resource`].
	pub fn set_spawner(&self, spawner: Arc<Spawner>) {
		*self.spawner.lock() = Some(spawner);
	}

	/// Spawns the level whose source is `source`, the JSON a `.level` file holds, and returns the level's root.
	///
	/// The level's entities take the handles after the root, one each, parents first and otherwise in the source's
	/// order; [`level_entity_handle_from_root`](crate::gameplay::level::level_entity_handle_from_root) finds the
	/// `n`-th from the root, and the entities of the prefabs its instances place follow. Delete the root to remove
	/// them all. Prefabs load from the resource manager. Publishing happens on the calling thread.
	pub fn spawn_level(&self, source: &str) -> Result<Handle, String> {
		self.spawn_level_in(&self.world, source, Placement::default())
			.map(|spawned| spawned.root)
	}

	/// Spawns the level whose source is `source` into `scope`, at `placement`, as [`Self::spawn_level`] spawns into
	/// the world, and returns what a tool needs to trace a spawned entity back to the source.
	///
	/// Use it to show a level apart from the game's world, such as on an editor's preview stage, where no game system
	/// follows its components. Next, find what placed an entity with [`Loaded::placer`] and [`Spawned::order`].
	pub fn spawn_level_in(&self, scope: &MessageScope, source: &str, placement: Placement) -> Result<Spawned, String> {
		const FAILURE: &str = "The level could not be spawned.";
		let spawner = self.spawner(FAILURE)?;
		let (level, payload, order) = bake_level(source.as_bytes()).map_err(|cause| format!("{FAILURE} {cause}"))?;
		let placed = !level.instances.is_empty();
		let mut loaded = Loaded::level("the inspector's level", level, payload);
		// Only a level that places prefabs needs the store.
		if placed {
			loaded = self.on_store(FAILURE, |store| loaded.complete(store))?;
		}
		let root = spawner.spawn(scope, &loaded, placement)?;
		Ok(Spawned { root, loaded, order })
	}

	/// Spawns the baked level or prefab `id` at `transform` and returns the spawn's root.
	///
	/// A prefab takes its parameters' defaults. Use it to drop a stored prefab into a running application, such as
	/// from an editor's palette of [`Self::resources`] of class `Prefab`. Delete the root to remove it.
	pub fn spawn_resource(&self, id: &str, transform: Transform) -> Result<Handle, String> {
		let failure = format!("'{id}' could not be spawned.");
		let spawner = self.spawner(&failure)?;
		let loaded = self.on_store(&failure, |store| Loaded::load(store, id))?;
		spawner.spawn(&self.world, &loaded, Placement::at(transform))
	}

	/// Returns the spawner the application gave, or the error that says to give one.
	fn spawner(&self, failure: &str) -> Result<Arc<Spawner>, String> {
		self.spawner.lock().clone().ok_or_else(|| {
			format!("{failure} The most likely cause is that the application gave the inspector no spawner. Call `DefaultInspector::set_spawner` with the application's spawner at setup.")
		})
	}

	/// Gives the inspector the resource manager [`Self::resources`] lists resources from. Only the first call takes
	/// effect, since an application has one resource manager.
	pub fn set_resource_manager(&self, resources: EntityHandle<ResourceManager>) {
		let _ = self.resource_manager.set(resources);
	}

	/// Returns the ID of every stored resource of `class`, such as `Mesh`, in ID order.
	///
	/// It reads the resource store on a thread of its own and waits for it, so call it when a list is wanted, such as
	/// when an editor opens its asset palette, not every frame.
	pub fn resources(&self, class: &str) -> Result<Vec<String>, String> {
		self.on_store("Resources could not be listed.", |store| query_all(store, class))
	}

	/// Returns the stored mesh `id` without its geometry: its primitives, with their counts, bounds, and materials,
	/// such as to describe the mesh in an editor or frame it with [`Mesh::bounding_box`].
	///
	/// It reads the store on a thread of its own and waits for it, like [`Self::resources`], and a development build
	/// bakes the mesh first when it's missing or stale. See [Resources](/docs/reference/inspector#resources).
	pub fn mesh(&self, id: &str) -> Result<Mesh, String> {
		let failure = format!("The mesh '{id}' could not be read.");
		self.on_store(&failure, |store| async {
			let mesh = store
				.request::<Mesh>(id)
				.await
				.map_err(|error| format!("{failure} {error}"))?;
			Ok(mesh.into_resource())
		})
	}

	/// Returns the ID of every source asset whose name ends in `.{extension}`, such as `bemx`, in ID order, in a
	/// development build.
	///
	/// It reads the asset directory on a thread of its own and waits for it, like [`Self::resources`]. Next, read one
	/// with [`Self::read_source_asset`]. The rules are specified under
	/// [Source assets](/docs/reference/inspector#source-assets).
	pub fn source_assets(&self, extension: &str) -> Result<Vec<String>, String> {
		self.on_store("Source assets could not be listed.", |store| async move {
			let suffix = format!(".{extension}");
			let mut ids = with_assets(store, async |assets| assets.sources().await).await?;
			ids.retain(|id| id.ends_with(&suffix));
			Ok(ids)
		})
	}

	/// Returns every folder under the asset directory that holds a source asset, in a development build: `/` for the
	/// asset directory itself, then each folder's ID with a trailing `/`, such as `levels/`, in ID order.
	///
	/// Use it to let a person pick where a new source asset goes, then write it with [`Self::write_source_asset`].
	pub fn source_folders(&self) -> Result<Vec<String>, String> {
		self.on_store("Source folders could not be listed.", |store| async move {
			Ok(folders_of(&with_assets(store, async |assets| assets.sources().await).await?))
		})
	}

	/// Returns the bytes of the source asset `id`, unchanged, in a development build: a `.bemx` graph's text or a PNG
	/// image's encoded pixels alike. Next, write a changed text back with [`Self::write_source_asset`].
	pub fn read_source_asset(&self, id: &str) -> Result<Vec<u8>, String> {
		self.on_store("The source asset could not be read.", |store| async move {
			let id = id.to_owned();
			with_assets(store, async move |assets| assets.read_source(&id).await).await
		})
	}

	/// Replaces the text of the source asset `id`, creating it when missing, in a development build.
	///
	/// The application then bakes again what read it, as after any source change. Only in-process tools write: the
	/// HTTP inspector has no authentication, so it never calls this.
	pub fn write_source_asset(&self, id: &str, text: &str) -> Result<(), String> {
		self.on_store("The source asset could not be written.", |store| async move {
			let (id, text) = (id.to_owned(), text.to_owned());
			with_assets(store, async move |assets| assets.write_source(&id, text.as_bytes()).await).await
		})
	}

	/// Returns what the standalone image source `id` bakes with: its semantic, the settings in effect, and whether it
	/// has a sidecar of its own, in a development build. Next, change one with [`Self::set_image_setting`]. The rules
	/// are specified under [Source assets](/docs/reference/inspector#source-assets).
	pub fn image_settings(&self, id: &str) -> Result<ResolvedImageSettings, String> {
		self.on_store("The image settings could not be read.", |store| async move {
			let id = id.to_owned();
			with_assets(store, async move |assets| assets.image_settings(&id).await).await
		})
	}

	/// Sets `setting` in the sidecar of the standalone image source `id`, creating the sidecar when it's missing, in a
	/// development build. The application then bakes the image again, as after any source change.
	pub fn set_image_setting(&self, id: &str, setting: ImageSetting) -> Result<(), String> {
		self.on_store("The image setting could not be changed.", |store| async move {
			let id = id.to_owned();
			with_assets(store, async move |assets| assets.set_image_setting(&id, setting).await).await
		})
	}

	/// Runs `operation` on the application's resource manager on a thread and runtime of its own, and waits for it.
	///
	/// The store reads files asynchronously, so its operations run off any runtime the caller may already be driving.
	/// `failure` begins the errors that keep it from running at all.
	fn on_store<'s, T: Send, F>(
		&'s self,
		failure: &str,
		operation: impl FnOnce(&'s ResourceManager) -> F + Send,
	) -> Result<T, String>
	where
		F: Future<Output = Result<T, String>>,
	{
		let store = self.resource_manager.get().ok_or_else(|| {
			format!(
				"{failure} The most likely cause is that the application has no resource manager. Use the inspector of a graphics application."
			)
		})?;
		std::thread::scope(|scope| {
			scope
				.spawn(|| {
					let runtime = compio::runtime::Runtime::new().map_err(|error| {
						format!(
							"{failure} The most likely cause is that the system ran out of file descriptors or threads: {error}. Close other applications and try again."
						)
					})?;
					runtime.block_on(operation(store))
				})
				.join()
				.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
		})
	}
}

/// Runs `operation` with the asset manager that reaches `store`'s source assets, on the thread that keeps it, or
/// returns the error a build without one gives.
async fn with_assets<T: Send + 'static>(
	store: &ResourceManager,
	operation: impl AsyncFnOnce(&resource_management::asset::manager::AssetManager) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
	#[cfg(debug_assertions)]
	{
		store
			.on_assets(async move |workers| operation(workers.manager()).await)
			.await
			.unwrap_or_else(|| {
				Err("The application bakes nothing on demand. The most likely cause is that it runs without an asset directory. Set `assets.path` to the source asset directory.".to_string())
			})
	}
	#[cfg(not(debug_assertions))]
	{
		let _ = (store, operation);
		Err("Source assets are unavailable. The most likely cause is a release build, which doesn't read source assets. Use a development build.".to_string())
	}
}

/// Reads every page of the IDs of `class`'s resources, sorted.
async fn query_all(resources: &ResourceManager, class: &str) -> Result<Vec<String>, String> {
	let mut ids = Vec::new();
	let mut query = Query::new(class);
	loop {
		let page = resources.query_ids(query.clone()).await.map_err(|error| {
			format!(
				"Resources could not be listed. The most likely cause is an unreadable resource store: {error:?}. Check the resources directory."
			)
		})?;
		ids.extend(page.items);
		match page.cursor {
			Some(cursor) => query.cursor = Some(cursor),
			None => break,
		}
	}
	ids.sort_unstable();
	Ok(ids)
}

/// The folders that hold the files `ids` name, at any depth: `/` for the top folder, then each folder's path with a
/// trailing `/`, in order, each once.
fn folders_of(ids: &[String]) -> Vec<String> {
	let mut folders = vec!["/".to_owned()];
	for id in ids {
		// Every `/` in an ID ends a folder that holds it: `a/b/c.png` is in `a/` and `a/b/`.
		for (end, _) in id.match_indices('/') {
			folders.push(id[..=end].to_owned());
		}
	}
	folders[1..].sort_unstable();
	folders.dedup();
	folders
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::Configuration;
	use crate::core::message_bus::MessageBus;
	use crate::metrics::Metrics;

	/// An inspector over a fresh world, and the world.
	fn inspector() -> (DefaultInspector, crate::core::message_bus::MessageScope) {
		let message_bus = MessageBus::default();
		message_bus.observe().expect("attach test message observer");
		let world = message_bus.new_scope("content-test-world");
		let inspector = DefaultInspector::new(
			crate::core::channel::DefaultChannel::new(),
			Configuration::new(),
			world.clone(),
			Arc::new(Metrics::new()),
		);
		(inspector, world)
	}

	const LEVEL: &str = r#"{ "entities": [
		{ "id": "crate", "components": { "name": "crate", "transform": { "position": [1, 2, 3] } } },
		{ "parent": "crate", "components": { "name": "lid", "transform": {} } },
	] }"#;

	#[test]
	fn a_spawned_level_places_its_entities_after_its_root() {
		let (inspector, _world) = inspector();
		inspector.set_spawner(Arc::new(Spawner::with_engine_kinds()));

		let root = inspector.spawn_level(LEVEL).expect("spawn the level");

		let entities = inspector.entities(None, None);
		let named = |name: &str| {
			entities
				.iter()
				.find(|entity| entity.name() == name)
				.map(|entity| entity.handle())
		};
		assert_eq!(named("crate"), Some(Handle::from_id(root.id() + 1)));
		assert_eq!(named("lid"), Some(Handle::from_id(root.id() + 2)));
	}

	#[test]
	fn a_level_spawns_nothing_without_a_spawner_or_when_it_does_not_bake() {
		let (inspector, _world) = inspector();
		assert!(inspector.spawn_level(LEVEL).unwrap_err().contains("set_spawner"));

		inspector.set_spawner(Arc::new(Spawner::with_engine_kinds()));
		assert!(inspector.spawn_level(r#"{ "entities": [{ "parent": "missing" }] }"#).is_err());
		assert!(inspector.entities(None, None).is_empty());
	}

	#[test]
	fn source_assets_are_listed_by_extension_read_as_bytes_and_written() {
		let (inspector, _world) = inspector();
		let directory = std::env::temp_dir().join(format!(
			"inspector-source-assets-{}-{:?}",
			std::process::id(),
			std::time::SystemTime::now()
		));
		std::fs::create_dir_all(directory.join("materials")).unwrap();
		std::fs::write(directory.join("materials/floor.bemx"), "{ nodes: [] }").unwrap();
		std::fs::write(directory.join("materials/floor.png"), [0u8, 159, 146, 150]).unwrap();
		let store = ResourceManager::hosted(resource_management::resource::storage_backend::MemoryStorageBackend::new());
		store.install_assets(
			resource_management::asset::storage_backend::FileStorageBackend::new(directory.clone()),
			|_| {},
		);
		inspector.set_resource_manager(EntityHandle::from(store));

		assert_eq!(inspector.source_assets("bemx").unwrap(), ["materials/floor.bemx"]);
		assert_eq!(inspector.read_source_asset("materials/floor.bemx").unwrap(), b"{ nodes: [] }");
		assert_eq!(
			inspector.read_source_asset("materials/floor.png").unwrap(),
			[0u8, 159, 146, 150]
		);

		inspector.write_source_asset("materials/wall.bemx", "{ nodes: [1] }").unwrap();
		assert_eq!(
			inspector.source_assets("bemx").unwrap(),
			["materials/floor.bemx", "materials/wall.bemx"]
		);
		assert_eq!(inspector.read_source_asset("materials/wall.bemx").unwrap(), b"{ nodes: [1] }");
		assert!(inspector.write_source_asset("../outside.bemx", "{}").is_err());

		std::fs::remove_dir_all(directory).unwrap();
	}

	#[test]
	fn folders_list_the_top_folder_then_every_folder_that_holds_a_file_once() {
		let ids = ["b/c/d.png", "a.bemx", "b/e.level", "b/c/f.png"].map(String::from);
		assert_eq!(folders_of(&ids), ["/", "b/", "b/c/"]);
	}

	#[test]
	fn resources_are_not_listed_without_a_resource_manager() {
		let (inspector, _world) = inspector();
		assert!(inspector.resources("Mesh").is_err());
		assert!(inspector.source_assets("bemx").is_err());
		assert!(inspector.write_source_asset("materials/floor.bemx", "{}").is_err());
	}
}
