use std::collections::hash_map::Entry;

use utils::hash::{HashMap, HashMapExt as _};

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

pub struct MeshBuffersStats {
	vertex_count: usize,
	index_count: usize,

	meshes: Vec<Mesh>,

	instances: Vec<usize>,
}

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

impl MeshBuffersStats {
	pub fn add_mesh(&mut self, mesh: MeshStats) -> AddMeshResponse {
		let vertex_offset = self.vertex_offset();
		let index_offset = self.index_offset();

		self.vertex_count += mesh.vertex_count;
		self.index_count += mesh.index_count;

		let mesh_id = self.meshes.len();

		self.meshes.push(Mesh {
			base_vertex: vertex_offset,
			base_index: index_offset,
			vertex_count: mesh.vertex_count,
			index_count: mesh.index_count,
		});

		AddMeshResponse {
			id: mesh_id,
			base_vertex: vertex_offset,
			base_index: index_offset,
		}
	}

	pub fn add_instance(&mut self, mesh_id: usize) {
		self.instances.push(mesh_id);
	}

	pub fn get_instance_batches(&self) -> Vec<InstanceBatch> {
		let mut batches = HashMap::with_capacity(self.meshes.len());

		for (instance_id, &mesh_id) in self.instances.iter().enumerate() {
			let mesh = &self.meshes[mesh_id];

			match batches.entry(mesh_id) {
				Entry::Vacant(e) => {
					e.insert(InstanceBatch {
						index_count: mesh.index_count,
						instance_count: 1,
						base_vertex: mesh.base_vertex,
						base_index: mesh.base_index,
						base_instance: instance_id,
					});
				}
				Entry::Occupied(mut e) => {
					e.get_mut().instance_count += 1;
				}
			}
		}

		batches.into_values().collect::<Vec<_>>()
	}

	pub fn vertex_offset(&self) -> usize {
		self.vertex_count
	}

	pub fn index_offset(&self) -> usize {
		self.index_count
	}
}

impl Default for MeshBuffersStats {
	fn default() -> Self {
		Self {
			vertex_count: 0,
			index_count: 0,
			meshes: Vec::new(),
			instances: Vec::new(),
		}
	}
}

#[cfg(test)]
mod tests {
    use crate::rendering::utils::{MeshBuffersStats, MeshStats};

	#[test]
	fn test_one_mesh_and_instance() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96));

		assert_eq!(mesh.vertex_offset(), 0);
		assert_eq!(mesh.index_offset(), 0);

		let mesh_instance = mesh_buffer_stats.add_instance(mesh.id());

		let batches = mesh_buffer_stats.get_instance_batches();

		let batch = &batches[0];
		assert_eq!(batch.index_count, 96);
		assert_eq!(batch.instance_count, 1);
		assert_eq!(batch.base_vertex, 0);
		assert_eq!(batch.base_index, 0);
		assert_eq!(batch.base_instance, 0);
	}

	#[test]
	fn test_one_mesh_and_two_instances() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96));

		assert_eq!(mesh.vertex_offset(), 0);
		assert_eq!(mesh.index_offset(), 0);

		let mesh_instance1 = mesh_buffer_stats.add_instance(mesh.id());
		let mesh_instance2 = mesh_buffer_stats.add_instance(mesh.id());

		let batches = mesh_buffer_stats.get_instance_batches();

		let batch = &batches[0];
		assert_eq!(batch.index_count, 96);
		assert_eq!(batch.instance_count, 2);
		assert_eq!(batch.base_vertex, 0);
		assert_eq!(batch.base_index, 0);
		assert_eq!(batch.base_instance, 0);
	}

	#[test]
	fn test_two_meshes_and_two_instances() {
		let mut mesh_buffer_stats = MeshBuffersStats::default();

		let mesh1 = mesh_buffer_stats.add_mesh(MeshStats::new(32, 96));
		let mesh2 = mesh_buffer_stats.add_mesh(MeshStats::new(64, 192));

		assert_eq!(mesh1.vertex_offset(), 0);
		assert_eq!(mesh1.index_offset(), 0);
		assert_eq!(mesh2.vertex_offset(), 32);
		assert_eq!(mesh2.index_offset(), 96);

		let mesh1_instance1 = mesh_buffer_stats.add_instance(mesh1.id());
		let mesh2_instance2 = mesh_buffer_stats.add_instance(mesh2.id());

		let batches = mesh_buffer_stats.get_instance_batches();

		let batch = &batches[0];
		assert_eq!(batch.index_count, 96);
		assert_eq!(batch.instance_count, 1);
		assert_eq!(batch.base_vertex, 0);
		assert_eq!(batch.base_index, 0);
		assert_eq!(batch.base_instance, 0);

		let batch = &batches[1];
		assert_eq!(batch.index_count, 192);
		assert_eq!(batch.instance_count, 1);
		assert_eq!(batch.base_vertex, 32);
		assert_eq!(batch.base_index, 96);
		assert_eq!(batch.base_instance, 1);
	}
}
