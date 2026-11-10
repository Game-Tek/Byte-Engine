//! Compact per-frame work items that let one mesh dispatch cover every instance of a phase.
//!
//! A task workgroup culls 32 meshlets, one on each lane. An instance that draws more than
//! [`SHARED_INSTANCE_MESHLETS`] meshlets gets workgroups of its own: one [`MeshDispatchWorkItem`] for each chunk of
//! 32 of its meshlets. The meshlets of smaller instances share workgroups instead, one item to a lane, so a scene of
//! many small meshes runs a workgroup for every 32 meshlets where it would run one for every instance. The task stage
//! then draws the small meshlets it keeps together, several to a mesh grid.
//!
//! The same work ranges are reused by every view that renders that instance set (camera, cascades, cone layers, and
//! point cube faces), so only the view index changes between draws.

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

/// The most meshlets an instance may draw for them to share task workgroups with other instances' meshlets.
///
/// Such an instance takes one work item for each meshlet where a larger one takes one for every 32, so the limit
/// trades work items for fuller workgroups.
pub(crate) const SHARED_INSTANCE_MESHLETS: u32 = 8;
const _: () = assert!(
	SHARED_INSTANCE_MESHLETS <= CHUNK_MASK,
	"A shared work item names its meshlet in the bits a chunk index takes."
);

/// The `MeshDispatchWorkItem` struct identifies the meshlets one task workgroup or one of its lanes culls: a chunk of
/// 32 meshlets of a large instance, or one meshlet of a small one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(transparent)]
pub(crate) struct MeshDispatchWorkItem {
	pub(super) packed: u32,
}

impl MeshDispatchWorkItem {
	/// The item of a lane that has no meshlet, which pads the last workgroup that small instances share.
	pub(super) const EMPTY: Self = Self { packed: u32::MAX };

	/// Names chunk `chunk_index` of a large instance's meshlets, or meshlet `chunk_index` of a small one.
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

/// The `WorkRange` struct identifies one contiguous work range that a single `dispatch_meshes` call consumes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct WorkRange {
	pub(crate) work_item_base: u32,
	pub(crate) workgroup_count: u32,
}

/// The `MeshDispatch` struct identifies the work of one raster phase: the instances one pipeline draws.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct MeshDispatch {
	/// The large instances' items, one to a workgroup.
	pub(crate) chunks: WorkRange,
	/// The small instances' items, 32 to a workgroup. It starts where `chunks` ends.
	pub(crate) shared: WorkRange,
}

impl MeshDispatch {
	pub(crate) fn is_empty(self) -> bool {
		self.chunks.workgroup_count == 0 && self.shared.workgroup_count == 0
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
			let (chunks, shared, left_out) = build_work_items(&mut work_items[base..], instances);
			dropped += left_out;
			let dispatch = MeshDispatch {
				chunks: WorkRange {
					work_item_base: base as u32,
					workgroup_count: chunks as u32,
				},
				shared: WorkRange {
					work_item_base: (base + chunks) as u32,
					workgroup_count: (shared / LANES) as u32,
				},
			};
			base += chunks + shared;
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
				"Visibility left {dropped} instances out of a frame. The most likely cause is that the frame's instances need more than the {capacity} meshlet groups the pipeline holds. Raise `render.instances.meshlet-group-capacity`."
			)
		});
		dispatches
	}
}

/// The lanes of a task workgroup, as a count of work items.
const LANES: usize = MESHLET_CULLING_TASK_GROUP_SIZE as usize;

/// Writes the work items of `instances`: a chunk item for every 32 meshlets of each large instance, then an item for
/// every meshlet of each small one, padded with empty items to whole workgroups.
///
/// Returns how many chunk items it wrote, how many shared items with their padding, and how many instances did not
/// fit in `destination`. Once an instance of a kind doesn't fit, it and the later ones of its kind stay out whole.
fn build_work_items(destination: &mut [MeshDispatchWorkItem], instances: &[Instance]) -> (usize, usize, usize) {
	let is_small = |instance: &&Instance| instance.meshlet_count <= SHARED_INSTANCE_MESHLETS;
	let mut left_out = 0;

	let mut chunks = 0;
	let mut large = instances.iter().filter(|instance| !is_small(instance));
	while let Some(instance) = large.next() {
		let items = instance.meshlet_count.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE);
		// One bounds check per instance instead of one per work item.
		let Some(slots) = destination.get_mut(chunks..chunks + items as usize) else {
			left_out += 1 + large.count();
			break;
		};
		for (slot, chunk_index) in slots.iter_mut().zip(0..items) {
			*slot = MeshDispatchWorkItem::new(instance.shader_mesh_index, chunk_index);
		}
		chunks += items as usize;
	}

	// The shared items end on a whole workgroup, so only whole workgroups of room count.
	let destination = &mut destination[chunks..];
	let room = destination.len() / LANES * LANES;
	let mut shared = 0;
	let mut small = instances.iter().filter(is_small);
	while let Some(instance) = small.next() {
		let Some(slots) = destination[..room].get_mut(shared..shared + instance.meshlet_count as usize) else {
			left_out += 1 + small.count();
			break;
		};
		for (slot, meshlet_index) in slots.iter_mut().zip(0..instance.meshlet_count) {
			*slot = MeshDispatchWorkItem::new(instance.shader_mesh_index, meshlet_index);
		}
		shared += instance.meshlet_count as usize;
	}
	let padded = shared.next_multiple_of(LANES);
	destination[shared..padded].fill(MeshDispatchWorkItem::EMPTY);
	(chunks, padded, left_out)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Reads each written item as its instance and its chunk or meshlet, with `None` for padding.
	fn items(destination: &[MeshDispatchWorkItem]) -> Vec<Option<(u32, u32)>> {
		destination
			.iter()
			.map(|work| (*work != MeshDispatchWorkItem::EMPTY).then_some((work.packed >> CHUNK_BITS, work.packed & CHUNK_MASK)))
			.collect()
	}

	fn instance(shader_mesh_index: u32, meshlet_count: u32) -> Instance {
		Instance {
			shader_mesh_index,
			meshlet_count,
		}
	}

	#[test]
	fn large_instances_take_an_item_per_chunk_and_small_ones_share_workgroups() {
		let instances = [instance(7, 33), instance(11, 0), instance(19, 2), instance(23, 1)];
		let mut destination = [MeshDispatchWorkItem::default(); 2 + LANES];

		let (chunks, shared, left_out) = build_work_items(&mut destination, &instances);

		assert_eq!((chunks, shared, left_out), (2, LANES, 0));
		assert_eq!(items(&destination[..chunks]), [Some((7, 0)), Some((7, 1))]);
		// The small instances' meshlets follow one another on the lanes of one workgroup, which padding fills.
		let lanes = items(&destination[chunks..]);
		assert_eq!(lanes[..3], [Some((19, 0)), Some((19, 1)), Some((23, 0))]);
		assert!(lanes[3..].iter().all(Option::is_none));
	}

	#[test]
	fn work_items_address_instances_past_ten_bits() {
		let instance = (MAX_ADDRESSABLE_INSTANCES - 1) as u32;

		let work = MeshDispatchWorkItem::new(instance, CHUNK_MASK);

		assert_eq!((work.packed >> CHUNK_BITS, work.packed & CHUNK_MASK), (instance, CHUNK_MASK));
	}

	#[test]
	fn instances_that_overflow_the_work_buffer_are_left_out_whole() {
		let instances = [instance(0, 64), instance(1, 64), instance(2, 1), instance(3, 64)];
		let mut destination = [MeshDispatchWorkItem::default(); 3];

		let (chunks, shared, left_out) = build_work_items(&mut destination, &instances);

		// The second instance needs two items and only one is left, so it and the later large one stay out. The small
		// one needs a whole workgroup of room, which isn't left either.
		assert_eq!((chunks, shared, left_out), (2, 0, 3));
	}
}
