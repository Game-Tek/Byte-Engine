//! Renderable terrain payloads.
//!
//! Use [`RenderableTerrain`] to draw a baked terrain. Its transform is published separately through the world
//! transform channel, as for a [`RenderableMesh`](super::mesh::RenderableMesh).

use std::sync::Arc;

/// The `RenderableTerrain` struct lets gameplay draw a baked terrain without choosing which of its node meshes load.
///
/// Create one through [`crate::core::message_bus::MessageScope::factory`] after installing the visibility pipeline,
/// and give its entity a transform. The pipeline loads and draws the nodes the camera needs, as the
/// [terrain rules](/docs/reference/rendering#terrains) specify. Bake the terrain first, as
/// [Bake a terrain](/docs/use/resources/terrain) shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderableTerrain {
	// Shared, not owned: the bus clones this ID into every listener, and the render thread, loader tasks, and inspector
	// each keep it in long-lived maps. A numeric key cannot replace the string, since debug builds bake from it on demand.
	resource_id: Arc<str>,
}

impl RenderableTerrain {
	/// Creates a renderable terrain that draws the `Terrain` resource baked from the `.terrain.bead` asset `id`.
	pub fn resource(id: impl Into<Arc<str>>) -> Self {
		Self { resource_id: id.into() }
	}

	/// Returns the ID of the `Terrain` resource this terrain draws.
	pub fn resource_id(&self) -> &Arc<str> {
		&self.resource_id
	}
}
