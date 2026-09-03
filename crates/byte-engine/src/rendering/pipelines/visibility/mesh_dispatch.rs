//! Compact per-frame work items that let one mesh dispatch cover every instance of a phase.
//!
//! Each task workgroup reads one [`MeshDispatchWorkItem`] naming an instance and a chunk of its meshlets.
//! The same work range is reused by every view that renders that instance set (camera, cascades, cone
//! layers, and point cube faces), so only the view index changes between draws.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;

use super::layout::{MAX_INSTANCES, MAX_MESHLETS, MESH_DISPATCH_WORK_BINDING, MESHLET_CULLING_TASK_GROUP_SIZE};
use super::scene::{Instance, RenderInfo};

const MAX_WORK_ITEMS_PER_INSTANCE: usize = MAX_MESHLETS.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE as usize);
const MAX_MESH_DISPATCH_WORK_ITEMS: usize = MAX_INSTANCES * MAX_WORK_ITEMS_PER_INSTANCE;
const INSTANCE_BITS: u32 = MAX_INSTANCES.ilog2();
const CHUNK_BITS: u32 = MAX_WORK_ITEMS_PER_INSTANCE.ilog2();
const INSTANCE_MASK: u32 = (1 << INSTANCE_BITS) - 1;
const CHUNK_MASK: u32 = (1 << CHUNK_BITS) - 1;
const _: () = assert!(
	MAX_MESH_DISPATCH_WORK_ITEMS == 131_072 && INSTANCE_BITS == 10 && CHUNK_BITS == 7,
	"Update the compact work and meshlet-instance payload declarations in the task shaders when visibility limits change."
);

/// The `MeshDispatchWorkItem` struct identifies one independently culled meshlet range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub(crate) struct MeshDispatchWorkItem {
	packed: u32,
}

impl MeshDispatchWorkItem {
	pub(super) fn new(instance_index: u32, chunk_index: u32) -> Self {
		assert!(
			instance_index <= INSTANCE_MASK && chunk_index <= CHUNK_MASK,
			"Visibility mesh dispatch coordinate exceeds its packed range. The most likely cause is a pipeline limit changing without updating the shared work format."
		);
		Self {
			packed: instance_index | (chunk_index << INSTANCE_BITS),
		}
	}

	#[cfg(test)]
	pub(super) fn packed(self) -> u32 {
		self.packed
	}
}

/// The `MeshDispatch` struct identifies one contiguous work range that a single `dispatch_meshes` call consumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshDispatch {
	work_item_base: u32,
	workgroup_count: u32,
}

impl MeshDispatch {
	pub(crate) fn is_empty(self) -> bool {
		self.workgroup_count == 0
	}

	pub(crate) fn workgroup_count(self) -> u32 {
		self.workgroup_count
	}

	pub(crate) fn work_item_base(self) -> u32 {
		self.work_item_base
	}

	#[cfg(test)]
	pub(crate) fn with_workgroup_count(workgroup_count: u32) -> Self {
		Self {
			work_item_base: 0,
			workgroup_count,
		}
	}
}

/// The `PhaseDispatches` struct groups the frame's work ranges by the raster phase that consumes them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PhaseDispatches {
	pub(crate) opaque: MeshDispatch,
	pub(crate) masked: MeshDispatch,
	pub(crate) transparent: MeshDispatch,
}

/// The `MeshDispatchWorkBuffer` struct owns the GPU-visible work storage shared by every view of a frame.
pub(crate) struct MeshDispatchWorkBuffer {
	handle: ghi::DynamicBufferHandle<[MeshDispatchWorkItem; MAX_MESH_DISPATCH_WORK_ITEMS]>,
}

impl MeshDispatchWorkBuffer {
	/// Creates the work buffer and binds it to the base descriptor set.
	pub(crate) fn new(context: &mut ghi::implementation::Context, descriptor_set: ghi::DescriptorSetHandle) -> Self {
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

	/// Packs the frame's opaque, masked, and transparent instance lists into adjacent work ranges.
	pub(crate) fn write_phases(&self, frame: &mut ghi::implementation::Frame, render_info: &RenderInfo) -> PhaseDispatches {
		let work_items = frame.get_mut_dynamic_buffer_slice(self.handle);
		let mut base = 0usize;
		let mut phase = |instances: &[Instance]| {
			let count = build_work_items(&mut work_items[base..], instances);
			let dispatch = MeshDispatch {
				work_item_base: base as u32,
				workgroup_count: count as u32,
			};
			base += count;
			dispatch
		};
		let dispatches = PhaseDispatches {
			opaque: phase(&render_info.opaque_instances),
			masked: phase(&render_info.masked_instances),
			transparent: phase(&render_info.transparent_instances),
		};
		frame.sync_buffer(self.handle);
		dispatches
	}
}

/// Flattens instance and meshlet-chunk dimensions into one native dispatch dimension.
fn build_work_items(destination: &mut [MeshDispatchWorkItem], instances: &[Instance]) -> usize {
	let mut count = 0;
	for instance in instances {
		for chunk_index in 0..instance.meshlet_count.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE) {
			destination[count] = MeshDispatchWorkItem::new(instance.shader_mesh_index, chunk_index);
			count += 1;
		}
	}
	count
}

#[cfg(test)]
mod tests {
	use super::*;

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
			.map(|work| (work.packed & INSTANCE_MASK, work.packed >> INSTANCE_BITS))
			.collect::<Vec<_>>();
		assert_eq!(unpacked, [(7, 0), (7, 1), (19, 0)]);
	}
}
