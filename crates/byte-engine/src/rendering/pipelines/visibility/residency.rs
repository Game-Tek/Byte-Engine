//! Detail level residency: which primitives load finer levels next, and whose finer levels to release when there is
//! no room.
//!
//! A resource mesh always holds each primitive's coarsest level. Each primitive's finer levels live in a block of their
//! own, which reloads one level finer at a time while any instance wants more detail, and is released whole when
//! another primitive needs the room and no instance wanted those levels in the last frame. Primitives of one mesh hold
//! their levels independently, so a large mesh refines only where the camera is close. [`MeshResidencies`] only
//! decides; the visibility manager carries its decisions out through the loader.

use std::ops::Range;
use std::sync::atomic::{AtomicU8, Ordering};

use utils::hash::HashMap;

use super::geometry::PrimitiveLevels;
use crate::rendering::renderable::mesh::MeshKey;

/// Frames a load waits before asking again when no primitive could give back room.
const RETRY_WITHOUT_ROOM_FRAMES: u64 = 60;

/// The `ResidencySlot` struct names one followed primitive, so the frame loop records wants without hashing.
///
/// Get one from [`MeshResidencies::slot`] and keep it with the primitive's render entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResidencySlot(u32);

/// The `MeshResidency` struct groups one followed mesh's primitives, which load together.
struct MeshResidency {
	key: MeshKey,
	/// The mesh's primitives in [`MeshResidencies::primitives`], in primitive order.
	primitives: Range<usize>,
	/// Whether a load of the mesh is in flight, so no second one starts.
	loading: bool,
	/// Whether the mesh's finer levels failed for a reason more room won't fix, so none is asked for again.
	unavailable: bool,
}

/// The `PrimitiveResidency` struct follows one primitive's resident detail levels and what its instances want.
#[derive(Debug)]
struct PrimitiveResidency {
	/// The primitive's mesh in [`MeshResidencies::meshes`].
	mesh: u32,
	/// The primitive's coarsest level, which its base block holds. Zero leaves nothing finer to load.
	base: u8,
	/// The finest resident level: `base` until a finer block loads.
	finest: u8,
	/// The finest level any instance wanted this frame, or `u8::MAX` when none was drawn. Lanes that rebuild instances
	/// lower it together.
	wanted: AtomicU8,
	/// The last frame some instance wanted a level finer than `base`, which the finer block holds.
	last_wanted_finer: Option<u64>,
	/// The first frame a load that found no room may ask again.
	retry_frame: u64,
}

/// The `MeshResidencies` struct decides detail level loads and releases for every resident resource mesh.
///
/// Each frame, report the level every drawn instance wants with [`Self::want`], then call [`Self::end_frame`] to
/// request the loads. Report each load's outcome per primitive with [`Self::loaded`] or [`Self::without_room`], then
/// close it with [`Self::load_finished`]; report a load that failed outright with [`Self::unavailable`].
#[derive(Default)]
pub(crate) struct MeshResidencies {
	meshes: Vec<MeshResidency>,
	/// Each followed mesh's index in `meshes`.
	mesh_indices: HashMap<MeshKey, u32>,
	primitives: Vec<PrimitiveResidency>,
	/// Primitive lists of finished loads, which the next loads reuse instead of allocating their own.
	spare_lists: Vec<Vec<PrimitiveLevels>>,
	/// The frame the wants being reported belong to.
	frame: u64,
}

impl MeshResidencies {
	/// Starts following a resident mesh whose primitives' coarsest levels are `coarsest`, in primitive order.
	pub(crate) fn add_mesh(&mut self, key: MeshKey, coarsest: impl IntoIterator<Item = u8>) {
		if self.mesh_indices.contains_key(&key) {
			return;
		}
		let mesh = self.meshes.len() as u32;
		let first = self.primitives.len();
		self.primitives.extend(coarsest.into_iter().map(|base| PrimitiveResidency {
			mesh,
			base,
			finest: base,
			wanted: AtomicU8::new(u8::MAX),
			last_wanted_finer: None,
			retry_frame: 0,
		}));
		self.mesh_indices.insert(key, mesh);
		self.meshes.push(MeshResidency {
			key,
			primitives: first..self.primitives.len(),
			loading: false,
			unavailable: false,
		});
	}

	/// Returns the slot of a followed primitive, or `None` when it has nothing finer to load.
	pub(crate) fn slot(&self, key: MeshKey, primitive: u32) -> Option<ResidencySlot> {
		let mesh = &self.meshes[*self.mesh_indices.get(&key)? as usize];
		let slot = mesh.primitives.start + primitive as usize;
		(mesh.primitives.contains(&slot) && self.primitives[slot].base != 0).then_some(ResidencySlot(slot as u32))
	}

	/// Records that an instance of the primitive in `slot` wants `level` this frame.
	///
	/// Only the finest level counts, so wants from several threads need no order. Many instances of one primitive
	/// want the same level, so the slot is read first and written only when the level is finer, which keeps lanes
	/// from contending for its cache line.
	pub(crate) fn want(&self, slot: ResidencySlot, level: u8) {
		let wanted = &self.primitives[slot.0 as usize].wanted;
		if level < wanted.load(Ordering::Relaxed) {
			wanted.fetch_min(level, Ordering::Relaxed);
		}
	}

	/// Closes the frame's wants and hands `request` each mesh without a load in flight whose primitives want more
	/// detail, with those primitives in order, each with the level one finer than it holds. `request` returns whether
	/// the load started; a mesh whose load can't start is marked unavailable.
	///
	/// A primitive asks for one level at a time, which keeps each load small and makes detail arrive coarse to fine.
	/// A mesh's primitives load together, so each of its level streams is read once per load.
	pub(crate) fn end_frame(&mut self, mut request: impl FnMut(MeshKey, Vec<PrimitiveLevels>) -> bool) {
		let frame = self.frame;
		self.frame += 1;
		for mesh in &mut self.meshes {
			let mut loads = Vec::new();
			for (primitive, residency) in self.primitives[mesh.primitives.clone()].iter_mut().enumerate() {
				let wanted = std::mem::replace(residency.wanted.get_mut(), u8::MAX);
				if wanted < residency.base {
					residency.last_wanted_finer = Some(frame);
				}
				if wanted >= residency.finest || frame < residency.retry_frame || mesh.loading || mesh.unavailable {
					continue;
				}
				if loads.capacity() == 0 {
					loads = self.spare_lists.pop().unwrap_or_default();
				}
				loads.push(PrimitiveLevels {
					primitive: primitive as u32,
					finest: residency.finest - 1,
					block: None,
				});
			}
			if !loads.is_empty() {
				mesh.loading = request(mesh.key, loads);
				mesh.unavailable = !mesh.loading;
			}
		}
	}

	/// Records that a primitive of `key` now holds levels `finest` and coarser.
	pub(crate) fn loaded(&mut self, key: MeshKey, primitive: u32, finest: u8) {
		if let Some(slot) = self.slot(key, primitive) {
			self.primitives[slot.0 as usize].finest = finest;
		}
	}

	/// Records that a primitive of `key` found no room and returns the primitive whose finer block to release for it,
	/// as its mesh and index, if any. That primitive's instances draw its base level from then on.
	///
	/// The released primitive is the one whose finer levels went unwanted the longest, among those no instance wanted
	/// in the last frame and whose mesh isn't loading. Its ranges free only once frames in flight finish, so the
	/// primitive asks again after `frames_in_flight` frames; without one to release it asks again much later.
	pub(crate) fn without_room(&mut self, key: MeshKey, primitive: u32, frames_in_flight: u64) -> Option<(MeshKey, u32)> {
		let last_frame = self.frame.checked_sub(1);
		let victim = self
			.primitives
			.iter()
			.enumerate()
			.filter(|(_, candidate)| {
				candidate.finest < candidate.base
					&& candidate.last_wanted_finer < last_frame
					&& !self.meshes[candidate.mesh as usize].loading
			})
			.min_by_key(|(_, candidate)| candidate.last_wanted_finer)
			.map(|(index, _)| index);
		let released = victim.map(|index| {
			let victim = &mut self.primitives[index];
			victim.finest = victim.base;
			let mesh = &self.meshes[victim.mesh as usize];
			(mesh.key, (index - mesh.primitives.start) as u32)
		});
		if let Some(slot) = self.slot(key, primitive) {
			self.primitives[slot.0 as usize].retry_frame = self.frame
				+ if released.is_some() {
					frames_in_flight + 1
				} else {
					RETRY_WITHOUT_ROOM_FRAMES
				};
		}
		released
	}

	/// Closes `key`'s load once every primitive's outcome was reported, so the mesh can load again, and keeps the
	/// load's primitive list for a later load to reuse.
	pub(crate) fn load_finished(&mut self, key: MeshKey, mut primitives: Vec<PrimitiveLevels>) {
		if let Some(&mesh) = self.mesh_indices.get(&key) {
			self.meshes[mesh as usize].loading = false;
		}
		if primitives.capacity() != 0 {
			primitives.clear();
			self.spare_lists.push(primitives);
		}
	}

	/// Records that `key`'s finer levels can't load, so none is asked for again, and closes its load.
	pub(crate) fn unavailable(&mut self, key: MeshKey) {
		let Some(&mesh) = self.mesh_indices.get(&key) else {
			return;
		};
		let mesh = &mut self.meshes[mesh as usize];
		mesh.loading = false;
		mesh.unavailable = true;
	}
}

#[cfg(test)]
mod tests {
	use super::MeshResidencies;
	use crate::rendering::renderable::mesh::MeshKey;

	const NEAR: MeshKey = MeshKey::Resource(1);
	const FAR: MeshKey = MeshKey::Resource(2);

	/// Ends the frame and returns each requested load as its mesh and its primitives with the level each loads.
	fn end_frame(residencies: &mut MeshResidencies) -> Vec<(MeshKey, Vec<(u32, u8)>)> {
		let mut loads = Vec::new();
		residencies.end_frame(|key, primitives| {
			loads.push((
				key,
				primitives.iter().map(|levels| (levels.primitive, levels.finest)).collect(),
			));
			true
		});
		loads
	}

	fn load(key: MeshKey, primitives: &[(u32, u8)]) -> (MeshKey, Vec<(u32, u8)>) {
		(key, primitives.to_vec())
	}

	#[test]
	fn wanted_detail_loads_one_level_at_a_time_down_to_the_wanted_level() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(NEAR, [3]);
		let slot = residencies.slot(NEAR, 0).unwrap();

		residencies.want(slot, 1);
		residencies.want(slot, 2);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 2)])]);
		// A mesh with a load in flight is not asked for again.
		residencies.want(slot, 1);
		assert!(end_frame(&mut residencies).is_empty());

		residencies.loaded(NEAR, 0, 2);
		residencies.load_finished(NEAR, Vec::new());
		residencies.want(slot, 1);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 1)])]);
		residencies.loaded(NEAR, 0, 1);
		residencies.load_finished(NEAR, Vec::new());
		residencies.want(slot, 1);
		assert!(end_frame(&mut residencies).is_empty());
	}

	#[test]
	fn primitives_of_one_mesh_refine_independently_in_one_load() {
		let mut residencies = MeshResidencies::default();
		// The second primitive's coarsest level is full detail, so it has nothing to load.
		residencies.add_mesh(NEAR, [2, 0, 3]);
		assert_eq!(residencies.slot(NEAR, 1), None);

		residencies.want(residencies.slot(NEAR, 0).unwrap(), 0);
		residencies.want(residencies.slot(NEAR, 2).unwrap(), 3);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 1)])]);
		residencies.loaded(NEAR, 0, 1);
		residencies.load_finished(NEAR, Vec::new());

		residencies.want(residencies.slot(NEAR, 0).unwrap(), 0);
		residencies.want(residencies.slot(NEAR, 2).unwrap(), 0);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 0), (2, 2)])]);
	}

	#[test]
	fn a_load_without_room_releases_the_longest_unwanted_finer_levels_and_waits_for_them() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(FAR, [2]);
		residencies.add_mesh(NEAR, [2]);
		let (far, near) = (residencies.slot(FAR, 0).unwrap(), residencies.slot(NEAR, 0).unwrap());
		residencies.want(far, 1);
		assert_eq!(end_frame(&mut residencies), [load(FAR, &[(0, 1)])]);
		residencies.loaded(FAR, 0, 1);
		residencies.load_finished(FAR, Vec::new());

		// The camera turns to the other mesh; the first one is drawn only at its base level now.
		residencies.want(far, 2);
		residencies.want(near, 0);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 1)])]);
		assert_eq!(residencies.without_room(NEAR, 0, 2), Some((FAR, 0)));
		residencies.load_finished(NEAR, Vec::new());

		// The released ranges free after the frames in flight, so the load waits for them.
		for _ in 0..3 {
			residencies.want(near, 0);
			assert!(end_frame(&mut residencies).is_empty());
		}
		residencies.want(near, 0);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 1)])]);
	}

	#[test]
	fn levels_an_instance_wanted_in_the_last_frame_are_never_released() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(FAR, [2]);
		residencies.add_mesh(NEAR, [2]);
		let (far, near) = (residencies.slot(FAR, 0).unwrap(), residencies.slot(NEAR, 0).unwrap());
		residencies.want(far, 1);
		end_frame(&mut residencies);
		residencies.loaded(FAR, 0, 1);
		residencies.load_finished(FAR, Vec::new());

		residencies.want(far, 1);
		residencies.want(near, 1);
		assert_eq!(end_frame(&mut residencies), [load(NEAR, &[(0, 1)])]);

		// Nothing can give room back, so the load waits and asks again later instead of failing.
		assert_eq!(residencies.without_room(NEAR, 0, 2), None);
		residencies.load_finished(NEAR, Vec::new());
		residencies.want(far, 1);
		residencies.want(near, 1);
		assert!(end_frame(&mut residencies).is_empty());
	}
}
