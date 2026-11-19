//! The scene as the editor shows it: the entities the inspector reports that have a place in the world.
//!
//! This module only turns inspector entities into rows. Polling the inspector is the editor's job, and drawing the
//! rows is the shell's.

use crate::core::factory::Handle;
use serde::Deserialize as _;

use crate::gameplay::Transform;
use crate::inspector::{
	CONE_LIGHT_UPDATE_MESSAGE_TYPE, DIRECTIONAL_LIGHT_UPDATE_MESSAGE_TYPE, POINT_LIGHT_UPDATE_MESSAGE_TYPE,
};
use crate::inspector::{InspectedEntity, find_entities};
use crate::rendering::{Camera, ConeLight, DirectionalLight, PointLight};

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
	/// The entity's light settings as authored, when it's a light.
	pub(crate) light: Option<SceneLight>,
	/// The entity's component types, by Rust type name, which pick the application's panels it shows.
	pub(crate) types: Vec<&'static str>,
	/// Every property the inspector tracks for the entity, by name, serialized, which the application's panels read.
	pub(crate) properties: Vec<(&'static str, serde_json::Value)>,
}

/// The `LightKind` enum is which shape of light an entity is, named as its inspector property and level kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LightKind {
	Point,
	Cone,
	Directional,
}

impl LightKind {
	pub(crate) const ALL: [Self; 3] = [Self::Point, Self::Cone, Self::Directional];

	/// The inspector property and level kind that hold the light's settings.
	pub(crate) fn property(self) -> &'static str {
		match self {
			Self::Point => "point_light",
			Self::Cone => "cone_light",
			Self::Directional => "directional_light",
		}
	}

	/// What the properties panel calls it.
	pub(crate) fn title(self) -> &'static str {
		match self {
			Self::Point => "Point light",
			Self::Cone => "Cone light",
			Self::Directional => "Directional light",
		}
	}

	/// The inspector message that replaces its settings.
	pub(crate) fn update_message(self) -> &'static str {
		match self {
			Self::Point => POINT_LIGHT_UPDATE_MESSAGE_TYPE,
			Self::Cone => CONE_LIGHT_UPDATE_MESSAGE_TYPE,
			Self::Directional => DIRECTIONAL_LIGHT_UPDATE_MESSAGE_TYPE,
		}
	}
}

/// The `SceneLight` struct is a light's settings as the panel edits them: its shape, and its update payload as
/// authored, whose fields the panel's light settings name by JSON pointer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SceneLight {
	pub(crate) kind: LightKind,
	pub(crate) payload: serde_json::Value,
}

impl SceneLight {
	/// Returns this light with `payload`, as the engine reads it back, or `None` when the light's rules refuse it, such
	/// as a cone whose inner angle passes its outer one.
	pub(crate) fn with_payload(&self, payload: serde_json::Value) -> Option<Self> {
		// The engine's own types check the payload, so the panel takes exactly the values the game would.
		fn checked<T: serde::de::DeserializeOwned + serde::Serialize>(payload: serde_json::Value) -> Option<serde_json::Value> {
			serde_json::to_value(serde_json::from_value::<T>(payload).ok()?).ok()
		}
		let payload = match self.kind {
			LightKind::Point => checked::<PointLight>(payload),
			LightKind::Cone => checked::<ConeLight>(payload),
			LightKind::Directional => checked::<DirectionalLight>(payload),
		}?;
		Some(Self { kind: self.kind, payload })
	}
}

impl SceneEntity {
	/// The text a row shows: the name, or the type for an unnamed entity.
	pub(crate) fn label(&self) -> &str {
		if self.name.is_empty() { &self.kind } else { &self.name }
	}
}

/// Builds the scene from the inspector's `entities`, keeping only entities with a transform, sorted by label.
///
/// Compare it with the scene the shell holds and write it only when it differs, so components that follow the scene
/// stay asleep; see [`relisted`] for whether the list must be rebuilt.
pub(crate) fn collect(entities: &[InspectedEntity]) -> Vec<SceneEntity> {
	let mut rows = Vec::with_capacity(entities.len());
	for entity in entities {
		let Some(transform) = entity.property("transform").and_then(|transform| Transform::deserialize(transform).ok())
		else {
			continue;
		};
		let name = entity.name().to_string();
		let kind = entity
			.types()
			.first()
			.map(|name| short_type(name))
			.unwrap_or_default()
			.to_string();
		let camera = entity.property("camera").and_then(|camera| Camera::deserialize(camera).ok());
		let light = LightKind::ALL.into_iter().find_map(|kind| {
			let payload = entity.property(kind.property())?.clone();
			Some(SceneLight { kind, payload })
		});
		rows.push(SceneEntity {
			handle: entity.handle(),
			name,
			kind,
			transform,
			camera,
			light,
			types: entity.types().to_vec(),
			properties: entity.properties().to_vec(),
		});
	}
	// A stable order keeps rows from jumping when the inspector reports entities in another order.
	rows.sort_by(|a, b| a.label().cmp(b.label()).then(a.handle.id().cmp(&b.handle.id())));

	rows
}

/// Whether going from the `old` scene to the `new` one changes the rows the list shows: an entity appeared,
/// disappeared, or was renamed. A moved entity or a changed camera changes only values the properties panel reads on
/// its own.
pub(crate) fn relisted(old: &[SceneEntity], new: &[SceneEntity]) -> bool {
	old.len() != new.len()
		|| new
			.iter()
			.zip(old)
			.any(|(new, old)| new.handle != old.handle || new.name != old.name || new.kind != old.kind)
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
