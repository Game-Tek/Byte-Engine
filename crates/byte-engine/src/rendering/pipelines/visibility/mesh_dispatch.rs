use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;

use super::pipeline_manager::Instance;
use super::{MAX_INSTANCES, MAX_MESHLETS, MESHLET_CULLING_TASK_GROUP_SIZE, MESH_DISPATCH_WORK_BINDING};

const MAX_WORK_ITEMS_PER_INSTANCE: usize = MAX_MESHLETS.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE as usize);
const MAX_MESH_DISPATCH_WORK_ITEMS: usize = MAX_INSTANCES * MAX_WORK_ITEMS_PER_INSTANCE;
const INSTANCE_BITS: u32 = MAX_INSTANCES.ilog2();
const CHUNK_BITS: u32 = MAX_WORK_ITEMS_PER_INSTANCE.ilog2();
const INSTANCE_MASK: u32 = (1 << INSTANCE_BITS) - 1;
const CHUNK_MASK: u32 = (1 << CHUNK_BITS) - 1;
const _: () = assert!(
	MAX_MESH_DISPATCH_WORK_ITEMS == 131_072
		&& MAX_MESHLETS == 4096
		&& MAX_INSTANCES == 1024
		&& INSTANCE_BITS == 10
		&& CHUNK_BITS == 7,
	"Update the compact work and meshlet-instance payload declarations in the shadow shaders when visibility limits change."
);

/// The `MeshDispatchWorkItem` struct identifies one independently culled meshlet range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(transparent)]
pub(crate) struct MeshDispatchWorkItem {
	packed: u32,
}

impl MeshDispatchWorkItem {
	/// Packs bounded mesh dispatch coordinates into the shader's single-word work format.
	pub(super) fn new(instance_index: u32, chunk_index: u32) -> Self {
		assert!(
			instance_index <= INSTANCE_MASK && chunk_index <= CHUNK_MASK,
			"Visibility mesh dispatch coordinate exceeds its packed range. The most likely cause is a pipeline limit changing without updating the shared work format."
		);
		Self {
			packed: instance_index | (chunk_index << INSTANCE_BITS),
		}
	}

	fn instance_index(self) -> u32 {
		self.packed & INSTANCE_MASK
	}

	fn chunk_index(self) -> u32 {
		(self.packed >> INSTANCE_BITS) & CHUNK_MASK
	}

	#[cfg(test)]
	pub(super) fn packed(self) -> u32 {
		self.packed
	}
}

/// The `MeshDispatch` struct carries the native task-workgroup count for one single-view mesh dispatch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshDispatch {
	workgroup_count: u32,
}

impl MeshDispatch {
	/// Returns whether this dispatch contains any mesh work.
	pub fn is_empty(self) -> bool {
		self.workgroup_count == 0
	}

	/// Returns the number of task workgroups represented by this dispatch.
	pub fn workgroup_count(self) -> u32 {
		self.workgroup_count
	}

	#[cfg(test)]
	pub(super) fn with_workgroup_count(workgroup_count: u32) -> Self {
		Self { workgroup_count }
	}
}

/// The `MeshDispatchWorkBuffer` struct owns reusable GPU-visible work storage for batched mesh dispatches.
pub(crate) struct MeshDispatchWorkBuffer {
	handle: ghi::DynamicBufferHandle<[MeshDispatchWorkItem; MAX_MESH_DISPATCH_WORK_ITEMS]>,
}

impl MeshDispatchWorkBuffer {
	/// Creates and binds work storage for the worst-case single-view visibility workload.
	///
	/// Call [`Self::write`] during frame preparation, then reuse the returned [`MeshDispatch`]
	/// for each view that renders the same instance set.
	pub fn new(context: &mut ghi::implementation::Context, descriptor_set: ghi::DescriptorSetHandle) -> Self {
		let handle = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Mesh Dispatch Work")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		context.write(&[ghi::DescriptorWrite::buffer(
			descriptor_set,
			MESH_DISPATCH_WORK_BINDING.slot(),
			handle.into(),
		)]);
		Self { handle }
	}

	/// Rebuilds one compact single-view dispatch without transient allocations.
	pub fn write(&self, frame: &mut ghi::implementation::Frame, instances: &[Instance]) -> MeshDispatch {
		let work_items = frame.get_mut_dynamic_buffer_slice(self.handle);
		let workgroup_count = build_work_items(work_items, instances);
		frame.sync_buffer(self.handle);
		MeshDispatch {
			workgroup_count: u32::try_from(workgroup_count).expect(
				"Visibility mesh dispatch count exceeds u32. The most likely cause is a work-buffer capacity larger than the native dispatch interface.",
			),
		}
	}
}

/// Flattens instance and meshlet-chunk dimensions into one native dispatch dimension.
fn build_work_items(destination: &mut [MeshDispatchWorkItem], instances: &[Instance]) -> usize {
	let mut count = 0;

	for instance in instances {
		let chunk_count = instance.meshlet_count.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE);
		for chunk_index in 0..chunk_count {
			let work_item = destination.get_mut(count).unwrap_or_else(|| {
				panic!(
					"Visibility mesh dispatch work capacity exceeded. The most likely cause is an instance count or meshlet count beyond the visibility pipeline limits."
				)
			});
			*work_item = MeshDispatchWorkItem::new(instance.shader_mesh_index, chunk_index);
			count += 1;
		}
	}

	count
}

#[cfg(test)]
mod tests {
	use super::{build_work_items, MeshDispatchWorkItem};
	use crate::rendering::pipelines::visibility::pipeline_manager::Instance;

	#[test]
	fn compact_work_items_flatten_instances_and_partial_meshlet_groups() {
		let instances = [
			Instance {
				shader_mesh_index: 7,
				meshlet_count: 33,
			},
			Instance {
				shader_mesh_index: 11,
				meshlet_count: 0,
			},
			Instance {
				shader_mesh_index: 19,
				meshlet_count: 2,
			},
		];
		let mut destination = [MeshDispatchWorkItem::default(); 4];

		let count = build_work_items(&mut destination, &instances);

		assert_eq!(count, 3);
		let unpacked = destination[..count]
			.iter()
			.map(|work| (work.instance_index(), work.chunk_index()))
			.collect::<Vec<_>>();
		assert_eq!(unpacked, [(7, 0), (7, 1), (19, 0)]);
	}
}
