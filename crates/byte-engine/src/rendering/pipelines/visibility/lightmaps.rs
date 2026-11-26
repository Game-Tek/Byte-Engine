//! The resident lightmap: which lightmap the world names, which slots of the scene set's `lightmaps` its pages take,
//! and where each spawned primitive's light lies in them.
//!
//! The manager feeds it the world's [`Lightmap`](crate::rendering::Lightmap) components and the loader's loaded
//! lightmaps, binds each page in the slot it hands out, and reads each instance's record from it when the instance
//! appears or the lightmap changes.

use resource_management::resources::lightmap::LightmapInstance;
use utils::hash::HashMap;

use super::layout::MAX_LIGHTMAPS;
use super::shader_data::InstanceLightmap;
use crate::core::factory::Handle;
use crate::gameplay::level::level_entity_handle;

/// The `Lightmaps` struct keeps the one lightmap the world names resident, so instances read their rectangle from it.
///
/// It owns the slots of the scene set's `lightmaps`: a lightmap's pages take the slots the resident lightmap leaves
/// free when it loads, and give them back when a newer lightmap replaces it or its holder is deleted.
#[derive(Default)]
pub(crate) struct Lightmaps {
	/// The entity that holds the `lightmap` kind and the resource its component names.
	requested: Option<(Handle, String)>,
	/// The requested lightmap once it loaded.
	resident: Option<Rectangles>,
}

/// The `Rectangles` struct is a loaded lightmap's slots in the scene set's `lightmaps` and each lit primitive's
/// rectangle.
struct Rectangles {
	/// The slot of each page.
	pages: Vec<u32>,
	/// The slot of the page and the scale and offset of each lit primitive, by its entity's handle and its index in
	/// the entity's mesh.
	scale_offsets: HashMap<(Handle, u32), (u32, [f32; 4])>,
}

impl Lightmaps {
	/// Makes `resource`, held by `holder`, the lightmap the world names, and returns its ID when it needs loading.
	///
	/// A newer lightmap replaces the one before it; instances keep their rectangles until it loads.
	pub(crate) fn request(&mut self, holder: Handle, resource: String) -> Option<&str> {
		if self
			.requested
			.as_ref()
			.is_some_and(|(requested_holder, requested)| *requested_holder == holder && *requested == resource)
		{
			return None;
		}
		let (_, requested) = self.requested.insert((holder, resource));
		Some(requested)
	}

	/// Adopts a loaded lightmap of `page_count` pages, and returns the slot of `lightmaps` to bind each page in. The
	/// slots of the lightmap it replaces become free. Returns `None` for a lightmap the world no longer names, or for
	/// one with more pages than the free slots hold, and leaves the resident lightmap as it was.
	///
	/// `holder` is the spawn index of the entity that holds the `lightmap` kind, which leads every instance to its
	/// entity's handle. Once the pages are bound, instances must read their rectangles again.
	pub(crate) fn adopt(
		&mut self,
		id: &str,
		holder: u32,
		instances: &[LightmapInstance],
		page_count: usize,
	) -> Option<&[u32]> {
		let (holder_handle, _) = self.requested.as_ref().filter(|(_, requested)| requested == id)?;
		// The resident lightmap still lights the scene while the new one loads, so its slots stay taken until now.
		let taken = self.resident.as_ref().map_or(&[][..], |resident| &resident.pages);
		let pages: Vec<u32> = (0..MAX_LIGHTMAPS as u32)
			.filter(|slot| !taken.contains(slot))
			.take(page_count)
			.collect();
		if pages.len() < page_count {
			log::error!(
				"Lightmap '{id}' has {page_count} pages, more than the {} free slots of the {MAX_LIGHTMAPS} the visibility pipeline holds, so it lights nothing. The most likely cause is a lightmap baked with an older engine, or by hand. Bake the lightmap again with `beld bake`.",
				pages.len()
			);
			return None;
		}
		let scale_offsets = instances
			.iter()
			.filter_map(|instance| {
				let slot = *pages.get(instance.page as usize)?;
				let handle = level_entity_handle(*holder_handle, holder, instance.entity);
				Some(((handle, instance.primitive), (slot, instance.scale_offset)))
			})
			.collect();
		let resident = self.resident.insert(Rectangles { pages, scale_offsets });
		Some(&resident.pages)
	}

	/// Forgets the lightmap when `handle` held it, and returns whether instances must read their rectangles again.
	pub(crate) fn forget(&mut self, handle: Handle) -> bool {
		if self.requested.as_ref().is_some_and(|(holder, _)| *holder == handle) {
			self.requested = None;
			return self.resident.take().is_some();
		}
		false
	}

	/// Returns the record of primitive `primitive` of the renderable `handle`: its page's slot and its rectangle in the
	/// resident lightmap, or [`InstanceLightmap::NONE`].
	pub(crate) fn instance(&self, handle: Handle, primitive: u32) -> InstanceLightmap {
		let Some(resident) = &self.resident else {
			return InstanceLightmap::NONE;
		};
		resident
			.scale_offsets
			.get(&(handle, primitive))
			.map_or(InstanceLightmap::NONE, |&(slot, scale_offset)| {
				InstanceLightmap::new(slot, scale_offset)
			})
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn instance(entity: u32, primitive: u32, page: u32, offset: f32) -> LightmapInstance {
		LightmapInstance {
			entity,
			primitive,
			page,
			scale_offset: [0.5, 0.5, offset, 0.0],
		}
	}

	#[test]
	fn instances_find_their_rectangles_through_the_holder() {
		let mut lightmaps = Lightmaps::default();
		// The holder is entity 2 of the spawn, which took handles 10 for the root and 11 onward for the entities.
		let holder = Handle::from_id(13);
		assert!(lightmaps.request(holder, "arena.lightmap.bead".into()).is_some());
		assert!(
			lightmaps.request(holder, "arena.lightmap.bead".into()).is_none(),
			"the same lightmap loads once"
		);

		assert_eq!(lightmaps.instance(Handle::from_id(11), 0), InstanceLightmap::NONE);
		let pages = lightmaps
			.adopt(
				"arena.lightmap.bead",
				2,
				&[instance(0, 0, 0, 0.0), instance(0, 1, 0, 0.25), instance(4, 0, 0, 0.5)],
				1,
			)
			.expect("the lightmap is wanted")
			.to_vec();

		assert_eq!(
			lightmaps.instance(Handle::from_id(11), 1),
			InstanceLightmap::new(pages[0], [0.5, 0.5, 0.25, 0.0])
		);
		assert_eq!(
			lightmaps.instance(Handle::from_id(15), 0),
			InstanceLightmap::new(pages[0], [0.5, 0.5, 0.5, 0.0])
		);
		assert_eq!(lightmaps.instance(Handle::from_id(12), 0).lightmap(), None);
	}

	#[test]
	fn instances_read_the_slot_of_their_own_page() {
		let mut lightmaps = Lightmaps::default();
		let holder = Handle::from_id(1);
		lightmaps.request(holder, "quarry.lightmap.bead".into());
		let pages = lightmaps
			.adopt(
				"quarry.lightmap.bead",
				0,
				&[instance(1, 0, 0, 0.0), instance(2, 0, 1, 0.0)],
				2,
			)
			.expect("the lightmap is wanted")
			.to_vec();

		assert_eq!(pages.len(), 2);
		assert_ne!(pages[0], pages[1]);
		assert_eq!(lightmaps.instance(Handle::from_id(2), 0).lightmap(), Some(pages[0]));
		assert_eq!(lightmaps.instance(Handle::from_id(3), 0).lightmap(), Some(pages[1]));
	}

	#[test]
	fn a_replaced_lightmap_gives_its_slots_to_the_next_one() {
		let mut lightmaps = Lightmaps::default();
		let holder = Handle::from_id(1);
		// Each lightmap takes the most pages one may have, so the slots run out unless replaced ones come back.
		let page_count = MAX_LIGHTMAPS / 2;
		let mut previous = Vec::new();
		for bake in 0..4 {
			let id = format!("bake{bake}.lightmap.bead");
			lightmaps.request(holder, id.clone());
			let pages = lightmaps
				.adopt(&id, 0, &[instance(0, 0, 0, 0.0)], page_count)
				.expect("the slots of the lightmap before it came back")
			.to_vec();
			assert!(
				pages.iter().all(|slot| !previous.contains(slot)),
				"the lightmap being replaced still binds its slots while the new one loads"
			);
			previous = pages;
		}
		assert!(lightmaps.forget(holder));
		assert_eq!(lightmaps.instance(holder, 0), InstanceLightmap::NONE);
	}

	#[test]
	fn a_lightmap_the_world_stopped_naming_lights_nothing() {
		let mut lightmaps = Lightmaps::default();
		let holder = Handle::from_id(5);
		lightmaps.request(holder, "old.lightmap.bead".into());
		lightmaps.request(holder, "new.lightmap.bead".into());
		assert!(
			lightmaps
				.adopt("old.lightmap.bead", 0, &[instance(0, 0, 0, 0.0)], 1)
				.is_none()
		);

		let pages = lightmaps
			.adopt("new.lightmap.bead", 0, &[instance(0, 0, 0, 0.0)], 1)
			.expect("the lightmap is wanted")
			.to_vec();
		assert_eq!(lightmaps.instance(holder, 0).lightmap(), Some(pages[0]));
		assert!(lightmaps.forget(holder));
		assert_eq!(lightmaps.instance(holder, 0), InstanceLightmap::NONE);
	}
}
