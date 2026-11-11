//! Event-driven instance lifecycle, independent of shared mesh and material residency.

use smallvec::SmallVec;
use utils::{StableVecHandle, hash::HashMap};

use crate::core::factory::Handle;
use crate::rendering::renderable::mesh::MeshKey;

/// The `RenderableState` enum retains only the state an instance currently owns.
enum RenderableState {
	Waiting { mesh: MeshKey, index: usize },
	Resident(SmallVec<[StableVecHandle; 1]>),
}

/// The `Renderables` struct owns each instance's lifecycle and indexes waiters by their shared mesh.
///
/// Remove an existing handle before replacing it. Mesh completion touches only its waiters; removing a waiter
/// updates the moved waiter's index, so neither operation scans unrelated instances.
#[derive(Default)]
pub(crate) struct Renderables {
	instances: HashMap<Handle, RenderableState>,
	waiting: HashMap<MeshKey, Vec<Handle>>,
}

impl Renderables {
	/// Inserts a new instance waiting for `mesh`, and returns whether it is the mesh's only waiter.
	pub(crate) fn wait(&mut self, handle: Handle, mesh: MeshKey) -> bool {
		let waiting = self.waiting.entry(mesh).or_default();
		let state = RenderableState::Waiting {
			mesh,
			index: waiting.len(),
		};
		assert!(
			self.instances.insert(handle, state).is_none(),
			"Remove an instance before replacing it."
		);
		waiting.push(handle);
		waiting.len() == 1
	}

	/// Makes one of this mesh's waiting instances resident and returns its handle, whose primitives must be registered
	/// next, or `None` once no instance waits for the mesh.
	///
	/// A resident mesh's waiters are adopted a few at a time, so a mesh that tens of thousands of instances wait for
	/// doesn't hold one frame for all of them. Each call takes the last waiter, so no other waiter moves.
	pub(crate) fn adopt_waiter(&mut self, mesh: MeshKey) -> Option<Handle> {
		let waiting = self.waiting.get_mut(&mesh)?;
		let handle = waiting
			.pop()
			.expect("A mesh's waiter list is removed when its last waiter leaves.");
		if waiting.is_empty() {
			self.waiting.remove(&mesh);
		}
		let state = self
			.instances
			.get_mut(&handle)
			.expect("A mesh waiter must have an instance record.");
		assert!(matches!(state, RenderableState::Waiting { mesh: key, .. } if *key == mesh));
		*state = RenderableState::Resident(SmallVec::new());
		Some(handle)
	}

	/// Returns one instance to remove through normal cleanup after its mesh fails to load.
	pub(crate) fn waiter(&self, mesh: MeshKey) -> Option<Handle> {
		self.waiting.get(&mesh)?.last().copied()
	}

	/// Records one primitive slot owned by a resident instance.
	pub(crate) fn add_primitive(&mut self, handle: Handle, slot: StableVecHandle) {
		let Some(RenderableState::Resident(primitives)) = self.instances.get_mut(&handle) else {
			panic!("Only a resident instance can own primitive slots.");
		};
		primitives.push(slot);
	}

	/// Returns the primitive slots an instance currently owns, or none while it waits or after deletion.
	pub(crate) fn primitives(&self, handle: Handle) -> &[StableVecHandle] {
		match self.instances.get(&handle) {
			Some(RenderableState::Resident(primitives)) => primitives,
			_ => &[],
		}
	}

	/// Removes an instance and returns its owned primitive slots for release. An unknown handle owns nothing.
	pub(crate) fn remove(&mut self, handle: Handle) -> Option<SmallVec<[StableVecHandle; 1]>> {
		match self.instances.remove(&handle)? {
			RenderableState::Resident(primitives) => Some(primitives),
			RenderableState::Waiting { mesh, index } => {
				let waiting = self
					.waiting
					.get_mut(&mesh)
					.expect("A waiting instance must belong to its mesh's waiters.");
				assert_eq!(waiting.swap_remove(index), handle);
				if let Some(moved) = waiting.get(index) {
					let Some(RenderableState::Waiting { index: moved_index, .. }) = self.instances.get_mut(moved) else {
						panic!("A moved mesh waiter must still be waiting.");
					};
					*moved_index = index;
				}
				if waiting.is_empty() {
					self.waiting.remove(&mesh);
				}
				Some(SmallVec::new())
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use utils::StableVec;

	use super::*;

	/// Adopts every waiter of `mesh`, in the order they started waiting.
	fn mesh_ready(instances: &mut Renderables, mesh: MeshKey) -> Vec<Handle> {
		let mut adopted = std::iter::from_fn(|| instances.adopt_waiter(mesh)).collect::<Vec<_>>();
		adopted.reverse();
		adopted
	}

	const FIRST: MeshKey = MeshKey::Resource(1);
	const SECOND: MeshKey = MeshKey::Resource(2);
	const THIRD: MeshKey = MeshKey::Resource(3);

	#[test]
	fn completion_adopts_only_instances_waiting_for_its_mesh() {
		let mut instances = Renderables::default();
		let [a, b, c] = std::array::from_fn(|_| Handle::new());
		instances.wait(a, FIRST);
		instances.wait(b, SECOND);
		instances.wait(c, FIRST);
		assert_eq!(mesh_ready(&mut instances, FIRST), [a, c]);
		let mut slots = StableVec::<()>::default();
		let slot = slots.push(());
		instances.add_primitive(a, slot);
		assert_eq!(instances.primitives(a), [slot]);
		assert!(instances.primitives(b).is_empty());
		assert_eq!(mesh_ready(&mut instances, SECOND), [b]);
		assert!(mesh_ready(&mut instances, FIRST).is_empty());
	}

	#[test]
	fn replacing_a_waiter_cannot_be_revived_by_its_old_mesh() {
		let mut instances = Renderables::default();
		let [a, b, c] = std::array::from_fn(|_| Handle::new());
		for handle in [a, b, c] {
			instances.wait(handle, FIRST);
		}
		assert!(instances.remove(a).unwrap().is_empty());
		instances.wait(a, SECOND);
		// Removing the waiter moved into the vacated slot must also work.
		assert!(instances.remove(c).unwrap().is_empty());
		assert_eq!(mesh_ready(&mut instances, FIRST), [b]);
		assert_eq!(mesh_ready(&mut instances, SECOND), [a]);
	}

	#[test]
	fn resident_replacement_returns_only_its_owned_primitives_once() {
		let mut instances = Renderables::default();
		let handle = Handle::new();
		let mut slots = StableVec::<()>::default();
		let owned = [slots.push(()), slots.push(())];
		instances.wait(handle, FIRST);
		assert_eq!(instances.adopt_waiter(FIRST), Some(handle));
		for slot in owned {
			instances.add_primitive(handle, slot);
		}
		assert_eq!(instances.remove(handle).unwrap().as_slice(), owned);
		instances.wait(handle, SECOND);
		assert!(instances.primitives(handle).is_empty());
		assert_eq!(mesh_ready(&mut instances, SECOND), [handle]);
		assert!(instances.remove(handle).unwrap().is_empty());
		assert!(instances.remove(handle).is_none());
	}

	#[test]
	fn deletion_before_completion_does_not_recreate_the_instance() {
		let mut instances = Renderables::default();
		let handle = Handle::new();
		instances.wait(handle, FIRST);
		instances.remove(handle).unwrap();
		assert!(mesh_ready(&mut instances, FIRST).is_empty());
		assert!(instances.remove(handle).is_none());
	}

	#[test]
	fn failed_mesh_forgets_only_its_waiters_and_allows_recreation() {
		let mut instances = Renderables::default();
		let [a, b, resident] = std::array::from_fn(|_| Handle::new());
		instances.wait(a, FIRST);
		instances.wait(b, SECOND);
		instances.wait(resident, THIRD);
		assert_eq!(instances.adopt_waiter(THIRD), Some(resident));
		let mut dropped = 0;
		while let Some(handle) = instances.waiter(FIRST) {
			instances.remove(handle).unwrap();
			dropped += 1;
		}
		assert_eq!(dropped, 1);
		assert!(instances.remove(a).is_none());
		assert!(instances.remove(resident).is_some());
		assert_eq!(mesh_ready(&mut instances, SECOND), [b]);
		instances.wait(a, FIRST);
		assert_eq!(mesh_ready(&mut instances, FIRST), [a]);
	}

	#[test]
	fn quarry_sized_shared_mesh_keeps_every_live_instance_through_replacements() {
		let mut instances = Renderables::default();
		let handles: Vec<_> = (0..63_000).map(|_| Handle::new()).collect();
		for &handle in &handles {
			assert!(instances.remove(handle).is_none());
			instances.wait(handle, FIRST);
		}
		for &handle in handles.iter().step_by(2) {
			instances.remove(handle).unwrap();
			instances.wait(handle, SECOND);
		}
		for mesh in [FIRST, SECOND] {
			let ready = mesh_ready(&mut instances, mesh);
			assert_eq!(ready.len(), handles.len() / 2);
			for handle in ready {
				assert!(instances.remove(handle).is_some());
			}
		}
		for handle in handles {
			assert!(instances.remove(handle).is_none());
		}
	}
}
