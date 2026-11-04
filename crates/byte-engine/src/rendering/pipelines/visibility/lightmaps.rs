//! The resident lightmap: which lightmap the world names, and where each spawned primitive's light lies in it.
//!
//! The manager feeds it the world's [`Lightmap`](crate::rendering::Lightmap) components and the loader's resident
//! lightmaps, and reads each instance's record from it when the instance appears or the lightmap changes.

use std::sync::Arc;

use resource_management::resources::lightmap::LightmapInstance;
use utils::hash::HashMap;

use super::shader_data::InstanceLightmap;
use crate::core::factory::Handle;
use crate::gameplay::level::level_entity_handle;

/// The `Lightmaps` struct keeps the one lightmap the world names resident, so instances read their rectangle from it.
#[derive(Default)]
pub(crate) struct Lightmaps {
	/// The entity that holds the `lightmap` kind and the resource its component names.
	requested: Option<(Handle, Arc<str>)>,
	/// The requested lightmap once it loaded.
	resident: Option<Rectangles>,
}

/// The `Rectangles` struct is a loaded lightmap's bindless slot and each lit primitive's rectangle.
struct Rectangles {
	texture: u32,
	/// The scale and offset of each lit primitive, by its entity's handle and its index in the entity's mesh.
	scale_offsets: HashMap<(Handle, u32), [f32; 4]>,
}

impl Lightmaps {
	/// Makes `resource`, held by `holder`, the lightmap the world names, and returns whether it needs loading.
	///
	/// A newer lightmap replaces the one before it; instances keep their rectangles until it loads.
	pub(crate) fn request(&mut self, holder: Handle, resource: Arc<str>) -> bool {
		if self
			.requested
			.as_ref()
			.is_some_and(|(requested_holder, requested)| *requested_holder == holder && *requested == resource)
		{
			return false;
		}
		self.requested = Some((holder, resource));
		true
	}

	/// Adopts a lightmap the loader made resident in bindless slot `texture`, and returns whether instances must read
	/// their rectangles again. A lightmap the world no longer names is ignored.
	///
	/// `holder` is the spawn index of the entity that holds the `lightmap` kind, which leads every instance to its
	/// entity's handle.
	pub(crate) fn adopt(&mut self, id: &str, holder: u32, instances: &[LightmapInstance], texture: u32) -> bool {
		let Some((holder_handle, _)) = self.requested.as_ref().filter(|(_, requested)| requested.as_ref() == id) else {
			return false;
		};
		let scale_offsets = instances
			.iter()
			.map(|instance| {
				let handle = level_entity_handle(*holder_handle, holder, instance.entity);
				((handle, instance.primitive), instance.scale_offset)
			})
			.collect();
		self.resident = Some(Rectangles { texture, scale_offsets });
		true
	}

	/// Forgets the lightmap when `handle` held it, and returns whether instances must read their rectangles again.
	pub(crate) fn forget(&mut self, handle: Handle) -> bool {
		if self.requested.as_ref().is_some_and(|(holder, _)| *holder == handle) {
			self.requested = None;
			return self.resident.take().is_some();
		}
		false
	}

	/// Returns the record of primitive `primitive` of the renderable `handle`: its rectangle in the resident lightmap,
	/// or [`InstanceLightmap::NONE`].
	pub(crate) fn instance(&self, handle: Handle, primitive: u32) -> InstanceLightmap {
		let Some(resident) = &self.resident else {
			return InstanceLightmap::NONE;
		};
		resident
			.scale_offsets
			.get(&(handle, primitive))
			.map_or(InstanceLightmap::NONE, |&scale_offset| InstanceLightmap {
				scale_offset,
				texture: resident.texture,
				padding: [0; 3],
			})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::pipelines::visibility::shader_data::NO_LIGHTMAP_TEXTURE;

	fn instance(entity: u32, primitive: u32, offset: f32) -> LightmapInstance {
		LightmapInstance {
			entity,
			primitive,
			scale_offset: [0.5, 0.5, offset, 0.0],
		}
	}

	#[test]
	fn instances_find_their_rectangles_through_the_holder() {
		let mut lightmaps = Lightmaps::default();
		// The holder is entity 2 of the spawn, which took handles 10 for the root and 11 onward for the entities.
		let holder = Handle::from_id(13);
		assert!(lightmaps.request(holder, "arena.lightmap.bead".into()));
		assert!(
			!lightmaps.request(holder, "arena.lightmap.bead".into()),
			"the same lightmap loads once"
		);

		assert_eq!(lightmaps.instance(Handle::from_id(11), 0), InstanceLightmap::NONE);
		assert!(lightmaps.adopt(
			"arena.lightmap.bead",
			2,
			&[instance(0, 0, 0.0), instance(0, 1, 0.25), instance(4, 0, 0.5)],
			7
		));

		let lit = lightmaps.instance(Handle::from_id(11), 1);
		assert_eq!(lit.texture, 7);
		assert_eq!(lit.scale_offset, [0.5, 0.5, 0.25, 0.0]);
		assert_eq!(lightmaps.instance(Handle::from_id(15), 0).scale_offset[2], 0.5);
		assert_eq!(lightmaps.instance(Handle::from_id(12), 0).texture, NO_LIGHTMAP_TEXTURE);
	}

	#[test]
	fn a_lightmap_the_world_stopped_naming_lights_nothing() {
		let mut lightmaps = Lightmaps::default();
		let holder = Handle::from_id(5);
		lightmaps.request(holder, "old.lightmap.bead".into());
		lightmaps.request(holder, "new.lightmap.bead".into());
		assert!(!lightmaps.adopt("old.lightmap.bead", 0, &[instance(0, 0, 0.0)], 1));

		assert!(lightmaps.adopt("new.lightmap.bead", 0, &[instance(0, 0, 0.0)], 2));
		assert_eq!(lightmaps.instance(holder, 0).texture, 2);
		assert!(lightmaps.forget(holder));
		assert_eq!(lightmaps.instance(holder, 0), InstanceLightmap::NONE);
	}
}
