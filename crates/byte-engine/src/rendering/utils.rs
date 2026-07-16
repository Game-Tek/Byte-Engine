use std::{alloc::Allocator, hash::Hash, marker::PhantomData, usize};

use utils::{
	hash::{HashMap, HashMapExt as _},
	StableVec, StableVecHandle,
};

use crate::core::factory::Handle;

pub struct MeshStats {
	vertex_count: usize,
	index_count: usize,
}

impl MeshStats {
	pub fn new(vertex_count: usize, index_count: usize) -> Self {
		Self {
			vertex_count,
			index_count,
		}
	}
}

pub struct AddMeshResponse {
	id: usize,
	base_vertex: usize,
	base_index: usize,
}

impl AddMeshResponse {
	pub fn id(&self) -> usize {
		self.id
	}

	pub fn vertex_offset(&self) -> usize {
		self.base_vertex
	}

	pub fn index_offset(&self) -> usize {
		self.base_index
	}
}

struct Mesh {
	vertex_count: usize,
	index_count: usize,
	base_index: usize,
	base_vertex: usize,
}

pub struct MeshBuffersStats<I> {
	vertex_count: usize,
	index_count: usize,

	meshes: HashMap<usize, Mesh>,

	instances: StableVec<(usize, I)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstanceBatch {
	base_index: usize,
	base_vertex: usize,
	instance_count: usize,
	index_count: usize,
	base_instance: usize,
}

impl InstanceBatch {
	pub fn base_index(&self) -> usize {
		self.base_index
	}

	pub fn base_vertex(&self) -> usize {
		self.base_vertex
	}

	pub fn instance_count(&self) -> usize {
		self.instance_count
	}

	pub fn index_count(&self) -> usize {
		self.index_count
	}

	pub fn base_instance(&self) -> usize {
		self.base_instance
	}
}

impl<I> MeshBuffersStats<I> {
	pub fn does_mesh_exist(&self, hash: u64) -> Option<usize> {
		if self.meshes.contains_key(&(hash as usize)) {
			Some(hash as usize)
		} else {
			None
		}
	}

	pub fn add_mesh(&mut self, mesh: MeshStats, hash: u64) -> AddMeshResponse {
		if let Some(existing_mesh) = self.meshes.get(&(hash as usize)) {
			assert_eq!(
				existing_mesh.vertex_count, mesh.vertex_count,
				"Tried to add a mesh with a hash which already exists but their vertex counts don't match."
			);
			assert_eq!(
				existing_mesh.index_count, mesh.index_count,
				"Tried to add a mesh with a hash which already exists but their index counts don't match."
			);

			return AddMeshResponse {
				id: hash as _,
				base_vertex: existing_mesh.base_vertex,
				base_index: existing_mesh.base_index,
			};
		}

		let vertex_offset = self.vertex_offset();
		let index_offset = self.index_offset();

		self.vertex_count += mesh.vertex_count;
		self.index_count += mesh.index_count;

		let mesh_id = hash as usize;

		self.meshes.insert(
			hash as usize,
			Mesh {
				base_vertex: vertex_offset,
				base_index: index_offset,
				vertex_count: mesh.vertex_count,
				index_count: mesh.index_count,
			},
		);

		AddMeshResponse {
			id: mesh_id,
			base_vertex: vertex_offset,
			base_index: index_offset,
		}
	}

	pub fn add_instance(&mut self, mesh_id: usize, instance_data: I) -> StableVecHandle {
		assert!(
			self.meshes.contains_key(&mesh_id),
			"Provided mesh_id for instance does not exist!"
		);
		self.instances.push((mesh_id, instance_data))
	}

	/// Removes an instance without shifting the remaining instance indices.
	pub fn remove_instance(&mut self, instance_id: StableVecHandle) -> Option<I> {
		self.instances.remove(instance_id).map(|(_, instance)| instance)
	}

	pub fn get_instance_batches(&self) -> InstanceBatches<'_, I> {
		InstanceBatches {
			batches: self.collect_instance_batches_in(std::alloc::Global),
			_marker: PhantomData,
		}
	}

	pub fn get_instance_batches_in<'a>(&self, allocator: &'a bumpalo::Bump) -> Vec<InstanceBatch, &'a bumpalo::Bump> {
		self.collect_instance_batches_in(allocator)
	}

	/// Collects contiguous mesh batches with the caller-selected allocation strategy.
	fn collect_instance_batches_in<A: Allocator>(&self, allocator: A) -> Vec<InstanceBatch, A> {
		let mut batches = Vec::with_capacity_in(self.instances.len(), allocator);
		let mut current_batch: Option<(usize, InstanceBatch)> = None;

		for instance_id in 0..self.instances.slots_len() {
			let Some((mesh_id, _)) = self.instances.get_slot(instance_id) else {
				if let Some((_, batch)) = current_batch.take() {
					batches.push(batch);
				}
				continue;
			};

			let mesh = &self.meshes.get(mesh_id).unwrap();
			match &mut current_batch {
				Some((current_mesh_id, batch)) if current_mesh_id == mesh_id => {
					batch.instance_count += 1;
				}
				Some(_) => {
					let (_, batch) = current_batch
						.replace((
							*mesh_id,
							InstanceBatch {
								index_count: mesh.index_count,
								instance_count: 1,
								base_vertex: mesh.base_vertex,
								base_index: mesh.base_index,
								base_instance: instance_id,
							},
						))
						.unwrap();
					batches.push(batch);
				}
				None => {
					current_batch = Some((
						*mesh_id,
						InstanceBatch {
							index_count: mesh.index_count,
							instance_count: 1,
							base_vertex: mesh.base_vertex,
							base_index: mesh.base_index,
							base_instance: instance_id,
						},
					));
				}
			}
		}

		if let Some((_, batch)) = current_batch {
			batches.push(batch);
		}

		batches
	}

	pub fn vertex_offset(&self) -> usize {
		self.vertex_count
	}

	pub fn index_offset(&self) -> usize {
		self.index_count
	}

	pub fn get_instance_id(&self, handle: I) -> Option<StableVecHandle>
	where
		I: Eq,
	{
		self.instances
			.handled_iter()
			.find_map(|(instance_handle, (_, h))| (*h == handle).then_some(instance_handle))
	}
}

impl<I> Default for MeshBuffersStats<I> {
	fn default() -> Self {
		Self {
			vertex_count: 0,
			index_count: 0,
			meshes: HashMap::with_capacity(4096),
			instances: StableVec::new(),
		}
	}
}

pub struct InstanceBatches<'a, I> {
	batches: Vec<InstanceBatch>,
	_marker: PhantomData<&'a I>,
}

impl<'a, I> InstanceBatches<'a, I> {
	pub fn iter(&self) -> InstanceBatchesIterator<'_, I> {
		InstanceBatchesIterator {
			batches: self.batches.iter(),
			_marker: PhantomData,
		}
	}
}

#[derive(Clone)]
pub struct InstanceBatchesIterator<'a, I> {
	batches: std::slice::Iter<'a, InstanceBatch>,
	_marker: PhantomData<I>,
}

impl<'a, I> InstanceBatchesIterator<'a, I> {
	pub fn into_vec(self) -> Vec<InstanceBatch> {
		self.batches.copied().collect()
	}
}

impl<'a, I: 'a> Iterator for InstanceBatchesIterator<'a, I> {
	type Item = BatchInstancesIterator<'a, I>;

	fn next(&mut self) -> Option<Self::Item> {
		self.batches.next().map(|b| BatchInstancesIterator {
			batch: *b,
			index: 0,
			_marker: PhantomData,
		})
	}
}

pub struct BatchInstancesIterator<'a, I> {
	batch: InstanceBatch,
	index: usize,
	_marker: PhantomData<&'a I>,
}

impl<'a, I> BatchInstancesIterator<'a, I> {
	pub fn index_count(&self) -> usize {
		self.batch.index_count()
	}

	pub fn instance_count(&self) -> usize {
		self.batch.instance_count()
	}

	pub fn base_vertex(&self) -> usize {
		self.batch.base_vertex()
	}

	pub fn base_index(&self) -> usize {
		self.batch.base_index()
	}

	pub fn base_instance(&self) -> usize {
		self.batch.base_instance()
	}
}

impl<'a, I> Iterator for BatchInstancesIterator<'a, I> {
	type Item = usize;

	fn next(&mut self) -> Option<Self::Item> {
		if self.index < self.batch.instance_count {
			let i = self.batch.base_instance + self.index;
			self.index += 1;
			Some(i)
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use crate::rendering::utils::{MeshBuffersStats, MeshStats};

	#[test]
	fn test_one_mesh_and_instance() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96), 1);

		assert_eq!(mesh.vertex_offset(), 0);
		assert_eq!(mesh.index_offset(), 0);

		let mesh_instance = mesh_buffer_stats.add_instance(mesh.id(), ());

		let batches = mesh_buffer_stats.get_instance_batches();
		let mut batches = batches.iter();

		let batch = batches.next().unwrap();
		assert_eq!(batch.index_count(), 96);
		assert_eq!(batch.instance_count(), 1);
		assert_eq!(batch.base_vertex(), 0);
		assert_eq!(batch.base_index(), 0);
		assert_eq!(batch.base_instance(), 0);
	}

	#[test]
	fn test_one_mesh_and_two_instances() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96), 1);

		assert_eq!(mesh.vertex_offset(), 0);
		assert_eq!(mesh.index_offset(), 0);

		let mesh_instance1 = mesh_buffer_stats.add_instance(mesh.id(), ());
		let mesh_instance2 = mesh_buffer_stats.add_instance(mesh.id(), ());

		let batches = mesh_buffer_stats.get_instance_batches();
		let mut batches = batches.iter();

		let batch = batches.next().unwrap();
		assert_eq!(batch.index_count(), 96);
		assert_eq!(batch.instance_count(), 2);
		assert_eq!(batch.base_vertex(), 0);
		assert_eq!(batch.base_index(), 0);
		assert_eq!(batch.base_instance(), 0);
	}

	#[test]
	fn test_two_meshes_and_two_instances() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh1 = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96), 1);
		let mesh2 = mesh_buffer_stats.add_mesh(MeshStats::new(64, 192), 2);

		assert_eq!(mesh1.vertex_offset(), 0);
		assert_eq!(mesh1.index_offset(), 0);
		assert_eq!(mesh2.vertex_offset(), 32);
		assert_eq!(mesh2.index_offset(), 96);

		let mesh1_instance1 = mesh_buffer_stats.add_instance(mesh1.id(), ());
		let mesh2_instance2 = mesh_buffer_stats.add_instance(mesh2.id(), ());

		let batches = mesh_buffer_stats.get_instance_batches();
		let mut batches = batches.iter().collect::<Vec<_>>();
		batches.sort_by_key(|e| e.batch.base_instance);
		let mut batches = batches.iter();

		let batch = batches.next().unwrap();
		assert_eq!(batch.index_count(), 96);
		assert_eq!(batch.instance_count(), 1);
		assert_eq!(batch.base_vertex(), 0);
		assert_eq!(batch.base_index(), 0);
		assert_eq!(batch.base_instance(), 0);

		let batch = batches.next().unwrap();
		assert_eq!(batch.index_count(), 192);
		assert_eq!(batch.instance_count(), 1);
		assert_eq!(batch.base_vertex(), 32);
		assert_eq!(batch.base_index(), 96);
		assert_eq!(batch.base_instance(), 1);
	}

	#[test]
	fn test_removed_instance_does_not_shift_or_batch_through_hole() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96), 1);
		let first = mesh_buffer_stats.add_instance(mesh.id(), "first");
		let second = mesh_buffer_stats.add_instance(mesh.id(), "second");
		let third = mesh_buffer_stats.add_instance(mesh.id(), "third");

		assert_eq!(mesh_buffer_stats.remove_instance(second), Some("second"));
		assert_eq!(mesh_buffer_stats.get_instance_id("first"), Some(first));
		assert_eq!(mesh_buffer_stats.get_instance_id("third"), Some(third));

		let batches = mesh_buffer_stats.get_instance_batches();
		let batches = batches.iter().into_vec();

		assert_eq!(batches.len(), 2);
		assert_eq!(batches[0].base_instance(), first.index());
		assert_eq!(batches[0].instance_count(), 1);
		assert_eq!(batches[1].base_instance(), third.index());
		assert_eq!(batches[1].instance_count(), 1);
	}

	#[test]
	fn heap_and_frame_allocators_preserve_mesh_switch_and_hole_batches() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();
		let first_mesh = mesh_buffer_stats.add_mesh(MeshStats::new(10, 30), 1);
		let second_mesh = mesh_buffer_stats.add_mesh(MeshStats::new(20, 60), 2);

		mesh_buffer_stats.add_instance(first_mesh.id(), "first");
		let removed = mesh_buffer_stats.add_instance(first_mesh.id(), "removed");
		mesh_buffer_stats.add_instance(second_mesh.id(), "second-a");
		mesh_buffer_stats.add_instance(second_mesh.id(), "second-b");
		mesh_buffer_stats.add_instance(first_mesh.id(), "last");
		assert_eq!(mesh_buffer_stats.remove_instance(removed), Some("removed"));

		let heap_batches = mesh_buffer_stats.get_instance_batches();
		let frame_allocator = bumpalo::Bump::new();
		let frame_batches = mesh_buffer_stats.get_instance_batches_in(&frame_allocator);

		assert_eq!(heap_batches.batches.as_slice(), frame_batches.as_slice());
		assert_eq!(frame_batches.len(), 3);
		assert_eq!((frame_batches[0].index_count(), frame_batches[0].base_instance()), (30, 0));
		assert_eq!((frame_batches[1].index_count(), frame_batches[1].instance_count()), (60, 2));
		assert_eq!((frame_batches[2].index_count(), frame_batches[2].base_instance()), (30, 4));
	}
}
