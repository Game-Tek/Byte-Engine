//! Detail level residency: which primitives load finer levels next, and whose finer levels to release when there is
//! no room.
//!
//! A resource mesh always holds each primitive's coarsest level. Each primitive's finer levels live in a block of their
//! own, which reloads one level finer at a time while any instance wants more detail, and is released whole when
//! another primitive needs the room and no instance wanted those levels in the last frame. Primitives of one mesh hold
//! their levels independently, so a large mesh refines only where the camera is close. [`MeshResidencies`] only
//! decides; the visibility manager carries its decisions out through the loader.

use std::collections::{HashMap, HashSet};

use crate::rendering::renderable::mesh::MeshKey;

/// Frames a load waits before asking again when no primitive could give back room.
const RETRY_WITHOUT_ROOM_FRAMES: u64 = 60;

/// The `ResidencySlot` struct names one followed primitive, so the frame loop records wants without hashing.
///
/// Get one from [`MeshResidencies::add_mesh`] or [`MeshResidencies::slot`] and keep it with the primitive's render
/// entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ResidencySlot(u32);

/// The `PrimitiveResidency` struct follows one primitive's resident detail levels and what its instances want.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PrimitiveResidency {
	key: MeshKey,
	primitive: u32,
	/// The primitive's coarsest level, which its base block holds.
	base: u8,
	/// The finest resident level: `base` until a finer block loads.
	finest: u8,
	/// The finest level any instance wanted this frame, or `u8::MAX` when none was drawn.
	wanted: u8,
	/// The last frame some instance wanted a level finer than `base`, which the finer block holds.
	last_wanted_finer: Option<u64>,
	/// Whether the primitive's finer levels failed for a reason more room won't fix, so none is asked for again.
	unavailable: bool,
	/// The first frame a load that found no room may ask again.
	retry_frame: u64,
}

/// The `MeshLevelLoad` struct asks the loader for finer levels of some primitives of one mesh, which load together.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct MeshLevelLoad {
	pub(crate) key: MeshKey,
	/// Each primitive to load, by index in its mesh, with the finest level to load for it.
	pub(crate) primitives: Vec<(u32, u8)>,
}

/// The `MeshResidencies` struct decides detail level loads and releases for every primitive with finer levels to load.
///
/// Each frame, report the level every drawn instance wants with [`Self::want`], then call [`Self::end_frame`] for the
/// loads to request. Report each load's outcome per primitive with [`Self::loaded`] or [`Self::without_room`], then
/// close it with [`Self::load_finished`]; report a load that failed outright with [`Self::unavailable`].
#[derive(Default)]
pub(crate) struct MeshResidencies {
	primitives: Vec<PrimitiveResidency>,
	slots: HashMap<(MeshKey, u32), ResidencySlot>,
	/// Meshes with a load in flight, so no second one starts.
	loading: HashSet<MeshKey>,
	/// The frame the wants being reported belong to.
	frame: u64,
}

impl MeshResidencies {
	/// Starts following a resident mesh whose primitives' coarsest levels are `coarsest`, in primitive order. A
	/// primitive whose coarsest level is full detail has nothing finer to load, so it isn't followed.
	pub(crate) fn add_mesh(&mut self, key: MeshKey, coarsest: impl IntoIterator<Item = u8>) {
		for (primitive, base) in coarsest.into_iter().enumerate() {
			let primitive = primitive as u32;
			if base == 0 || self.slots.contains_key(&(key, primitive)) {
				continue;
			}
			self.slots.insert((key, primitive), ResidencySlot(self.primitives.len() as u32));
			self.primitives.push(PrimitiveResidency {
				key,
				primitive,
				base,
				finest: base,
				wanted: u8::MAX,
				last_wanted_finer: None,
				unavailable: false,
				retry_frame: 0,
			});
		}
	}

	/// Returns the slot of a followed primitive, or `None` when it has nothing finer to load.
	pub(crate) fn slot(&self, key: MeshKey, primitive: u32) -> Option<ResidencySlot> {
		self.slots.get(&(key, primitive)).copied()
	}

	/// Records that an instance of the primitive in `slot` wants `level` this frame.
	pub(crate) fn want(&mut self, slot: ResidencySlot, level: u8) {
		let primitive = &mut self.primitives[slot.0 as usize];
		primitive.wanted = primitive.wanted.min(level);
	}

	/// Closes the frame's wants and returns, per mesh without a load in flight, the primitives to load one level finer
	/// than they hold, with that level.
	///
	/// A primitive asks for one level at a time, which keeps each load small and makes detail arrive coarse to fine.
	/// A mesh's primitives load together, so each of its level streams is read once per load.
	pub(crate) fn end_frame(&mut self) -> Vec<MeshLevelLoad> {
		let frame = self.frame;
		self.frame += 1;
		let mut loads: Vec<MeshLevelLoad> = Vec::new();
		for primitive in &mut self.primitives {
			let wanted = std::mem::replace(&mut primitive.wanted, u8::MAX);
			if wanted < primitive.base {
				primitive.last_wanted_finer = Some(frame);
			}
			if wanted >= primitive.finest
				|| primitive.unavailable
				|| frame < primitive.retry_frame
				|| self.loading.contains(&primitive.key)
			{
				continue;
			}
			let level = (primitive.primitive, primitive.finest - 1);
			match loads.iter_mut().find(|load| load.key == primitive.key) {
				Some(load) => load.primitives.push(level),
				None => loads.push(MeshLevelLoad {
					key: primitive.key,
					primitives: vec![level],
				}),
			}
		}
		self.loading.extend(loads.iter().map(|load| load.key));
		loads
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
					&& !self.loading.contains(&candidate.key)
			})
			.min_by_key(|(_, candidate)| candidate.last_wanted_finer)
			.map(|(index, _)| index);
		let released = victim.map(|index| {
			let victim = &mut self.primitives[index];
			victim.finest = victim.base;
			(victim.key, victim.primitive)
		});
		if let Some(slot) = self.slot(key, primitive) {
			self.primitives[slot.0 as usize].retry_frame =
				self.frame + if released.is_some() { frames_in_flight + 1 } else { RETRY_WITHOUT_ROOM_FRAMES };
		}
		released
	}

	/// Closes `key`'s load once every primitive's outcome was reported, so the mesh can load again.
	pub(crate) fn load_finished(&mut self, key: MeshKey) {
		self.loading.remove(&key);
	}

	/// Records that `key`'s finer levels can't load, so none is asked for again, and closes its load.
	pub(crate) fn unavailable(&mut self, key: MeshKey) {
		for primitive in self.primitives.iter_mut().filter(|primitive| primitive.key == key) {
			primitive.unavailable = true;
		}
		self.loading.remove(&key);
	}
}

#[cfg(test)]
mod tests {
	use super::{MeshLevelLoad, MeshResidencies};
	use crate::rendering::renderable::mesh::MeshKey;

	const NEAR: MeshKey = MeshKey::Resource(1);
	const FAR: MeshKey = MeshKey::Resource(2);

	fn load(key: MeshKey, primitives: &[(u32, u8)]) -> MeshLevelLoad {
		MeshLevelLoad {
			key,
			primitives: primitives.to_vec(),
		}
	}

	#[test]
	fn wanted_detail_loads_one_level_at_a_time_down_to_the_wanted_level() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(NEAR, [3]);
		let slot = residencies.slot(NEAR, 0).unwrap();

		residencies.want(slot, 1);
		residencies.want(slot, 2);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 2)])]);
		// A mesh with a load in flight is not asked for again.
		residencies.want(slot, 1);
		assert!(residencies.end_frame().is_empty());

		residencies.loaded(NEAR, 0, 2);
		residencies.load_finished(NEAR);
		residencies.want(slot, 1);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 1)])]);
		residencies.loaded(NEAR, 0, 1);
		residencies.load_finished(NEAR);
		residencies.want(slot, 1);
		assert!(residencies.end_frame().is_empty());
	}

	#[test]
	fn primitives_of_one_mesh_refine_independently_in_one_load() {
		let mut residencies = MeshResidencies::default();
		// The second primitive's coarsest level is full detail, so it has nothing to load.
		residencies.add_mesh(NEAR, [2, 0, 3]);
		assert_eq!(residencies.slot(NEAR, 1), None);

		residencies.want(residencies.slot(NEAR, 0).unwrap(), 0);
		residencies.want(residencies.slot(NEAR, 2).unwrap(), 3);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 1)])]);
		residencies.loaded(NEAR, 0, 1);
		residencies.load_finished(NEAR);

		residencies.want(residencies.slot(NEAR, 0).unwrap(), 0);
		residencies.want(residencies.slot(NEAR, 2).unwrap(), 0);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 0), (2, 2)])]);
	}

	#[test]
	fn a_load_without_room_releases_the_longest_unwanted_finer_levels_and_waits_for_them() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(FAR, [2]);
		residencies.add_mesh(NEAR, [2]);
		let (far, near) = (residencies.slot(FAR, 0).unwrap(), residencies.slot(NEAR, 0).unwrap());
		residencies.want(far, 1);
		assert_eq!(residencies.end_frame(), [load(FAR, &[(0, 1)])]);
		residencies.loaded(FAR, 0, 1);
		residencies.load_finished(FAR);

		// The camera turns to the other mesh; the first one is drawn only at its base level now.
		residencies.want(far, 2);
		residencies.want(near, 0);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 1)])]);
		assert_eq!(residencies.without_room(NEAR, 0, 2), Some((FAR, 0)));
		residencies.load_finished(NEAR);

		// The released ranges free after the frames in flight, so the load waits for them.
		for _ in 0..3 {
			residencies.want(near, 0);
			assert!(residencies.end_frame().is_empty());
		}
		residencies.want(near, 0);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 1)])]);
	}

	#[test]
	fn levels_an_instance_wanted_in_the_last_frame_are_never_released() {
		let mut residencies = MeshResidencies::default();
		residencies.add_mesh(FAR, [2]);
		residencies.add_mesh(NEAR, [2]);
		let (far, near) = (residencies.slot(FAR, 0).unwrap(), residencies.slot(NEAR, 0).unwrap());
		residencies.want(far, 1);
		residencies.end_frame();
		residencies.loaded(FAR, 0, 1);
		residencies.load_finished(FAR);

		residencies.want(far, 1);
		residencies.want(near, 1);
		assert_eq!(residencies.end_frame(), [load(NEAR, &[(0, 1)])]);

		// Nothing can give room back, so the load waits and asks again later instead of failing.
		assert_eq!(residencies.without_room(NEAR, 0, 2), None);
		residencies.load_finished(NEAR);
		residencies.want(far, 1);
		residencies.want(near, 1);
		assert!(residencies.end_frame().is_empty());
	}
}
