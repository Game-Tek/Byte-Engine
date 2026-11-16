//! What the inspector adds to and lists from a running application's content: levels spawned from their source, and
//! the IDs of stored resources.
//!
//! Both need parts the application creates after the inspector, so the application hands them over once they exist:
//! its spawner with [`DefaultInspector::set_spawner`], its resource manager with
//! [`DefaultInspector::set_resource_manager`]. The rules are specified under
//! [Spawning levels](/docs/reference/inspector#spawning-levels) and [Resources](/docs/reference/inspector#resources).

use std::sync::Arc;

use resource_management::{
	ResourceManager, asset::handler::implementations::level::bake_level, resource::storage_backend::Query,
};

use super::DefaultInspector;
use crate::{
	core::{EntityHandle, factory::Handle},
	gameplay::Spawner,
};

impl DefaultInspector {
	/// Gives the inspector the spawner levels spawn with, replacing any earlier one.
	///
	/// Give it the same spawner the application spawns its own levels with, so a spawned level knows the
	/// application's kinds. Next, spawn with [`Self::spawn_level`].
	pub fn set_spawner(&self, spawner: Arc<Spawner>) {
		*self.spawner.lock() = Some(spawner);
	}

	/// Spawns the level whose source is `source`, the JSON a `.level` file holds, and returns the level's root.
	///
	/// The level's entities take the handles after the root, one each, parents first and otherwise in the source's
	/// order; [`level_entity_handle_from_root`](crate::gameplay::level::level_entity_handle_from_root) finds the
	/// `n`-th from the root. Delete the root to remove them all. Publishing happens on
	/// the calling thread.
	pub fn spawn_level(&self, source: &str) -> Result<Handle, String> {
		let spawner = self.spawner.lock().clone().ok_or_else(|| {
			"The level could not be spawned. The most likely cause is that the application gave the inspector no spawner. Call `DefaultInspector::set_spawner` with the application's spawner at setup.".to_string()
		})?;
		let (level, payload) =
			bake_level(source.as_bytes()).map_err(|cause| format!("The level could not be spawned. {cause}"))?;
		spawner.spawn(&self.world, &level, &payload)
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
		let resources = self.resource_manager.get().ok_or_else(|| {
			"Resources could not be listed. The most likely cause is that the application has no resource manager. List resources from a graphics application.".to_string()
		})?;
		// The store reads files asynchronously, so the query runs on a runtime of its own, off any runtime the caller
		// may already be driving.
		std::thread::scope(|scope| {
			scope
				.spawn(|| {
					let runtime = compio::runtime::Runtime::new().map_err(|error| {
						format!(
							"Resources could not be listed. The most likely cause is that the system ran out of file descriptors or threads: {error}. Close other applications and try again."
						)
					})?;
					runtime.block_on(query_all(resources, class))
				})
				.join()
				.unwrap_or_else(|panic| std::panic::resume_unwind(panic))
		})
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
		{ "parent": "crate", "components": { "name": "lid" } },
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
	fn resources_are_not_listed_without_a_resource_manager() {
		let (inspector, _world) = inspector();
		assert!(inspector.resources("Mesh").is_err());
	}
}
