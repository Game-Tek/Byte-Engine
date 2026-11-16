//! The scene as the editor shows it: the entities the inspector reports that have a place in the world.
//!
//! This module only turns inspector entities into rows. Polling the inspector is the editor's job, and drawing the
//! rows is the shell's.

use crate::core::factory::Handle;
use crate::gameplay::Transform;
use crate::inspector::{InspectedEntity, InspectedValue, find_entities};
use crate::rendering::Camera;

/// The `SceneEntity` struct is one row of the scene overview and what the properties panel shows when it is selected.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneEntity {
	/// The identity the inspector and posted messages address the entity by.
	pub(crate) handle: Handle,
	/// The entity's attached name, or empty when it has none.
	pub(crate) name: String,
	/// The entity's first type without its module path, such as `Camera`.
	pub(crate) kind: String,
	/// The entity's place in the world, whole, so a move keeps its scale and orientation.
	pub(crate) transform: Transform,
	/// The entity's camera settings, when it's a camera.
	pub(crate) camera: Option<Camera>,
}

impl SceneEntity {
	/// The text a row shows: the name, or the type for an unnamed entity.
	pub(crate) fn label(&self) -> &str {
		if self.name.is_empty() { &self.kind } else { &self.name }
	}
}

/// Rewrites `scene` from the inspector's `entities`, keeping only entities with a transform, sorted by label.
///
/// Returns whether the rows changed in a way the list must be rebuilt for: an entity appeared, disappeared, or was
/// renamed. A moved entity or a changed camera changes only values the properties panel reads on its own.
pub(crate) fn collect(entities: &[InspectedEntity], scene: &mut Vec<SceneEntity>) -> bool {
	let previous = scene.len();
	// The new rows are built apart from the old ones, so they can be compared before they replace them.
	let mut rows = Vec::with_capacity(entities.len());
	for entity in entities {
		let Some(InspectedValue::Transform(transform)) = entity.property("transform") else {
			continue;
		};
		let name = entity.name().to_string();
		let kind = entity
			.types()
			.first()
			.map(|name| short_type(name))
			.unwrap_or_default()
			.to_string();
		let camera = match entity.property("camera") {
			Some(InspectedValue::Camera(camera)) => Some(camera.clone()),
			_ => None,
		};
		rows.push(SceneEntity {
			handle: entity.handle(),
			name,
			kind,
			transform: transform.clone(),
			camera,
		});
	}
	// A stable order keeps rows from jumping when the inspector reports entities in another order.
	rows.sort_by(|a, b| a.label().cmp(b.label()).then(a.handle.id().cmp(&b.handle.id())));

	let changed = rows.len() != previous
		|| rows
			.iter()
			.zip(scene.iter())
			.any(|(new, old)| new.handle != old.handle || new.name != old.name || new.kind != old.kind);
	*scene = rows;
	changed
}

/// Returns the handles of the `scene` entities the overview lists for `search`, in the order it lists them.
///
/// An empty search lists the whole scene in its order. Any other search is the inspector's entity search,
/// [`find_entities`], over `entities`: the same one the console's entities view uses, best match first.
pub(crate) fn shown(entities: &[InspectedEntity], scene: &[SceneEntity], search: &str) -> Vec<Handle> {
	if search.trim().is_empty() {
		return scene.iter().map(|entity| entity.handle).collect();
	}
	find_entities(entities, search)
		.into_iter()
		.map(|entity| entity.handle())
		// The overview lists only entities with a place in the world.
		.filter(|handle| scene.iter().any(|entity| entity.handle == *handle))
		.collect()
}

/// Cuts a Rust type name to its last path segment and drops its generic arguments, so
/// `my_game::npc::Npc<f32>` becomes `Npc`.
fn short_type(name: &str) -> &str {
	let name = name.split('<').next().unwrap_or(name);
	name.rsplit("::").next().unwrap_or(name)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn short_types_drop_paths_and_generics() {
		assert_eq!(short_type("my_game::npc::Npc<alloc::string::String>"), "Npc");
		assert_eq!(short_type("Camera"), "Camera");
	}
}
