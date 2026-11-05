//! Compact per-frame work items that let one mesh dispatch cover every instance of a phase.
//!
//! Each task workgroup reads one [`MeshDispatchWorkItem`] naming an instance and a chunk of its meshlets.
//! The same work range is reused by every view that renders that instance set (camera, cascades, cone
//! layers, and point cube faces), so only the view index changes between draws.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;

use super::layout::{MAX_ADDRESSABLE_INSTANCES, MAX_INSTANCE_MESHLETS, MESHLET_CULLING_TASK_GROUP_SIZE};
use super::scene::{Instance, RenderInfo};

const MAX_WORK_ITEMS_PER_INSTANCE: usize = MAX_INSTANCE_MESHLETS.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE as usize);
const CHUNK_BITS: u32 = MAX_WORK_ITEMS_PER_INSTANCE.ilog2();
const CHUNK_MASK: u32 = (1 << CHUNK_BITS) - 1;
const _: () = assert!(
	CHUNK_BITS == 7 && MAX_ADDRESSABLE_INSTANCES.ilog2() + CHUNK_BITS == u32::BITS,
	"Update the compact work decoding in the task shader when visibility limits change."
);

/// The `MeshDispatchWorkItem` struct identifies one independently culled meshlet range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub(crate) struct MeshDispatchWorkItem {
	pub(super) packed: u32,
}

impl MeshDispatchWorkItem {
	pub(super) fn new(instance_index: u32, chunk_index: u32) -> Self {
		debug_assert!(
			(instance_index as usize) < MAX_ADDRESSABLE_INSTANCES && chunk_index <= CHUNK_MASK,
			"Visibility mesh dispatch coordinate exceeds its packed range. The most likely cause is a pipeline limit changing without updating the shared work format."
		);
		Self {
			packed: chunk_index | (instance_index << CHUNK_BITS),
		}
	}
}

/// The `MeshDispatch` struct identifies one contiguous work range that a single `dispatch_meshes` call consumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshDispatch {
	pub(crate) work_item_base: u32,
	pub(crate) workgroup_count: u32,
}

impl MeshDispatch {
	pub(crate) fn is_empty(self) -> bool {
		self.workgroup_count == 0
	}
}

/// The `PhaseDispatches` struct groups the frame's work ranges by the raster phase that consumes them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct PhaseDispatches {
	/// The opaque layer's work ranges in the order [`super::render_pass::Pipelines::phases`] requests their pipelines:
	/// solid, masked, double-sided, and double-sided masked.
	pub(crate) opaque_layer: [MeshDispatch; 4],
	pub(crate) transparent: MeshDispatch,
}

/// The `MeshDispatchWorkBuffer` struct owns the GPU-visible work storage shared by every view of a frame.
pub(crate) struct MeshDispatchWorkBuffer {
	handle: ghi::DynamicBufferHandle<[MeshDispatchWorkItem]>,
	/// Whether a frame already reported dropping work items, so the warning appears once.
	reported_overflow: bool,
}

impl MeshDispatchWorkBuffer {
	/// Creates the work buffer with room for `meshlet_groups` work items and binds it in `scene_set`, the pipeline's
	/// scene descriptor set.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		scene_set: ghi::DescriptorSetHandle,
		meshlet_groups: u32,
	) -> Self {
		let handle = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Mesh Dispatch Work")
				.length(meshlet_groups as usize)
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		context.write(&[super::set_layouts::SCENE
			.binder(scene_set)
			.buffer("mesh_dispatch_work", handle)]);
		Self {
			handle,
			reported_overflow: false,
		}
	}

	/// Packs the frame's opaque, masked, both double-sided, and transparent instance lists into adjacent work ranges.
	///
	/// Instances whose work items don't fit in the buffer are left out of the frame, and the first frame that leaves
	/// any out logs a warning.
	pub(crate) fn write_phases(&mut self, frame: &mut ghi::implementation::Frame, render_info: &RenderInfo) -> PhaseDispatches {
		let work_items = frame.get_mut_dynamic_buffer_slice(self.handle);
		let capacity = work_items.len();
		let mut base = 0usize;
		let mut dropped = 0usize;
		let mut phase = |instances: &[Instance]| {
			let (count, left_out) = build_work_items(&mut work_items[base..], instances);
			dropped += left_out;
			let dispatch = MeshDispatch {
				work_item_base: base as u32,
				workgroup_count: count as u32,
			};
			base += count;
			dispatch
		};
		let dispatches = PhaseDispatches {
			opaque_layer: [
				phase(&render_info.opaque_instances),
				phase(&render_info.masked_instances),
				phase(&render_info.double_sided_instances),
				phase(&render_info.double_sided_masked_instances),
			],
			transparent: phase(&render_info.transparent_instances),
		};
		frame.sync_buffer(self.handle);
		crate::rendering::warn_once(&mut self.reported_overflow, dropped > 0, || {
			format!(
				"Visibility left {dropped} instances out of a frame. The most likely cause is that the frame's instances need more than the {capacity} meshlet groups the pipeline holds; raise `render.instances.meshlet-group-capacity`."
			)
		});
		dispatches
	}
}

/// Flattens instance and meshlet-chunk dimensions into one native dispatch dimension.
///
/// Returns how many work items it wrote and how many instances did not fit in `destination`.
fn build_work_items(destination: &mut [MeshDispatchWorkItem], instances: &[Instance]) -> (usize, usize) {
	let mut count = 0;
	for (written, instance) in instances.iter().enumerate() {
		let chunks = instance.meshlet_count.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE);
		// One bounds check per instance instead of one per work item.
		let Some(slots) = destination.get_mut(count..count + chunks as usize) else {
			return (count, instances.len() - written);
		};
		for (slot, chunk_index) in slots.iter_mut().zip(0..chunks) {
			*slot = MeshDispatchWorkItem::new(instance.shader_mesh_index, chunk_index);
		}
		count += chunks as usize;
	}
	(count, 0)
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

		let (count, left_out) = build_work_items(&mut destination, &instances);

		assert_eq!((count, left_out), (3, 0));
		let unpacked = destination[..count]
			.iter()
			.map(|work| (work.packed >> CHUNK_BITS, work.packed & CHUNK_MASK))
			.collect::<Vec<_>>();
		assert_eq!(unpacked, [(7, 0), (7, 1), (19, 0)]);
	}

	#[test]
	fn work_items_address_instances_past_ten_bits() {
		let instance = (MAX_ADDRESSABLE_INSTANCES - 1) as u32;

		let work = MeshDispatchWorkItem::new(instance, CHUNK_MASK);

		assert_eq!((work.packed >> CHUNK_BITS, work.packed & CHUNK_MASK), (instance, CHUNK_MASK));
	}

	#[test]
	fn instances_that_overflow_the_work_buffer_are_left_out_whole() {
		let instances = [
			Instance {
				shader_mesh_index: 0,
				meshlet_count: 64,
			},
			Instance {
				shader_mesh_index: 1,
				meshlet_count: 64,
			},
			Instance {
				shader_mesh_index: 2,
				meshlet_count: 1,
			},
		];
		let mut destination = [MeshDispatchWorkItem::default(); 3];

		let (count, left_out) = build_work_items(&mut destination, &instances);

		// The second instance needs two items and only one is left, so it and every later instance stay out.
		assert_eq!((count, left_out), (2, 2));
	}
}
