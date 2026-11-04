/// The `Lightmap` struct names the baked lightmap that lights a spawned level's static meshes.
///
/// A level publishes it through its `lightmap` kind, on the entity that holds the kind, so most applications never
/// create one. The visibility pipeline loads the lightmap and lights every primitive the lightmap gives a rectangle,
/// on the entities of the same spawn, when `render.gi.technique` is `baked`. See the
/// [lightmap format](/docs/reference/assets#lightmaps) and [lightmap lookups](/docs/reference/rendering#lightmaps).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lightmap {
	resource_id: std::sync::Arc<str>,
}

impl Lightmap {
	/// Creates a lightmap backed by the named `.lightmap.bead` resource.
	///
	/// Publish it on the level entity that the lightmap's bake found the `lightmap` kind on, through
	/// [`crate::core::message_bus::MessageScope::factory`], so the pipeline finds the entities the lightmap names.
	pub fn new(resource_id: impl Into<std::sync::Arc<str>>) -> Self {
		Self {
			resource_id: resource_id.into(),
		}
	}

	/// Returns the baked lightmap resource this component names.
	pub fn resource_id(&self) -> &str {
		&self.resource_id
	}
}
