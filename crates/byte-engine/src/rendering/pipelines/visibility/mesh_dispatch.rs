//! The work items that let one mesh dispatch cover every instance of a phase, which the GPU builds each frame.
//!
//! A task workgroup culls 32 meshlets, one on each lane. An instance that draws more than
//! [`SHARED_INSTANCE_MESHLETS`] meshlets gets workgroups of its own: one [`MeshDispatchWorkItem`] for each chunk of
//! 32 of its meshlets. The meshlets of smaller instances share workgroups instead, one item to a lane, so a scene of
//! many small meshes runs a workgroup for every 32 meshlets where it would run one for every instance. The task stage
//! then draws the small meshlets it keeps together, several to a mesh grid.
//!
//! How many meshlets an instance draws follows the camera, so the CPU doesn't write the items. [`MeshWork`] keeps
//! only the list of instances each raster phase draws, which changes when the scene does, and records three compute
//! passes that turn it into work items, ranges, and dispatch counts from the frame's instance records. The meshlet
//! passes then dispatch as many workgroups as the GPU counted.
//!
//! The same work ranges are reused by every view that renders that instance set (camera, cascades, cone layers, and
//! point cube faces), so only the view index changes between draws.

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;

use super::layout::{MAX_ADDRESSABLE_INSTANCES, MAX_INSTANCE_MESHLETS, MESHLET_CULLING_TASK_GROUP_SIZE, ShaderMeshLevel};
use super::scene::{Instance, RenderInfo};
use super::shader_data::ShaderMesh;
use crate::rendering::{PipelineManagerClient, PipelineRef};

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

	/// Names chunk `chunk_index` of a large instance's meshlets, or meshlet `chunk_index` of a small one. The GPU
	/// packs the frame's items, so only the shader tests pack one here.
	#[cfg(test)]
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

/// The work ranges a frame has: for each of the five raster phases, the items of its large instances, then the items
/// its small instances share. `scene_records.besl` spells it `WORK_RANGES`.
pub(crate) const WORK_RANGES: usize = 10;

/// The `MeshDispatch` struct identifies the work of one raster phase: the instances one pipeline draws.
///
/// Get the frame's from [`MeshWork::dispatches`] and draw each with
/// [`super::render_pass::record_meshlet_dispatches`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MeshDispatch {
	/// The phase, which names its two work ranges: the large instances' at twice it, and the shared one after.
	pub(crate) phase: u32,
	/// How many instances the phase lists. Some may draw nothing from where the camera is.
	pub(crate) listed: u32,
	/// The workgroup counts of every work range, which the GPU writes each frame.
	pub(crate) records: ghi::BufferHandle<[[u32; 3]; WORK_RANGES]>,
}

impl MeshDispatch {
	pub(crate) fn is_empty(self) -> bool {
		self.listed == 0
	}
}

/// The `PhaseDispatches` struct groups the frame's work by the raster phase that consumes it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PhaseDispatches {
	/// The opaque layer's work in the order [`super::render_pass::Pipelines::phases`] requests their pipelines:
	/// solid, masked, double-sided, and double-sided masked.
	pub(crate) opaque_layer: [MeshDispatch; 4],
	pub(crate) transparent: MeshDispatch,
}

/// The words at the start of the work buffer that hold each range's first item and workgroup count, which the task
/// stage finds its items through. `scene_records.besl` spells it `WORK_HEADER`.
const WORK_HEADER: usize = WORK_RANGES * 2;
/// The entries of the instance list each workgroup of the count and scatter passes takes. Every phase's entries start
/// at a multiple of it. `work_build.besl` spells it `WORK_GROUP_ENTRIES`.
const GROUP_ENTRIES: usize = 32;
/// The raster phases, each a run of the instance list.
const PHASES: usize = WORK_RANGES / 2;

/// The `MeshWork` struct owns the instance list and the passes that build the frame's work items from it.
///
/// Create it with [`Self::new`]. Call [`Self::write_lists`] on a frame whose copy of the list is out of date, hand
/// [`Self::dispatches`] to the passes that draw meshlets, and [`Self::record`] the build before any of them.
pub(crate) struct MeshWork {
	descriptor_set: ghi::DescriptorSetHandle,
	pipelines: [PipelineRef; 3],
	/// Each listed instance's slot, phase by phase, each phase padded to whole groups with empty entries.
	instance_list: ghi::DynamicBufferHandle<[u32]>,
	records: ghi::BufferHandle<[[u32; 3]; WORK_RANGES]>,
	/// The words the work buffer holds: the ranges, then the work items.
	capacity: u32,
	/// One past the last group of each phase in the instance list.
	phase_ends: [u32; PHASES],
	/// How many instances each phase lists.
	listed: [u32; PHASES],
	/// Whether a frame already reported that its work doesn't fit, so the warning appears once.
	reported_overflow: bool,
}

impl MeshWork {
	/// Creates the work buffers for `instances` listed instances and `meshlet_groups` work items, binds the work items
	/// and ranges in `scene_set`, the pipeline's scene descriptor set, for the meshlet passes to read, and requests the
	/// build pipelines. `meshes` and `mesh_levels` are the instance and detail level records the build reads.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		scene_set: ghi::DescriptorSetHandle,
		meshes: ghi::DynamicBufferHandle<[ShaderMesh]>,
		mesh_levels: ghi::BufferHandle<[ShaderMeshLevel]>,
		instances: u32,
		meshlet_groups: u32,
	) -> Self {
		let fixed = |name, uses| {
			ghi::buffer::Builder::new(uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let device = |name, uses, length: usize| fixed(name, uses).length(length);
		// Room for every instance, with each phase padded to whole groups.
		let entries = (instances as usize).next_multiple_of(GROUP_ENTRIES) + PHASES * GROUP_ENTRIES;
		let instance_list = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Visibility Mesh Work Instances")
				.length(entries)
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let entry_items =
			context.build_buffer::<[u32]>(device("Visibility Mesh Work Entry Items", ghi::Uses::Storage, entries));
		let group_items = context.build_buffer::<[[u32; 2]]>(device(
			"Visibility Mesh Work Group Items",
			ghi::Uses::Storage,
			entries / GROUP_ENTRIES,
		));
		let work_items = context.build_buffer::<[MeshDispatchWorkItem]>(device(
			"Visibility Mesh Dispatch Work",
			ghi::Uses::Storage,
			WORK_HEADER + meshlet_groups as usize,
		));
		let records = context.build_buffer::<[[u32; 3]; WORK_RANGES]>(fixed(
			"Visibility Mesh Work Dispatches",
			ghi::Uses::Storage | ghi::Uses::Indirect,
		));
		let descriptor_set = super::set_layouts::WORK_BUILD.create_set(context, "Visibility Mesh Work Set");
		let build = super::set_layouts::WORK_BUILD.binder(descriptor_set);
		let scene = super::set_layouts::SCENE.binder(scene_set);
		context.write(&[
			build.buffer("meshes", meshes),
			build.buffer("mesh_levels", mesh_levels),
			build.buffer("instance_list", instance_list),
			build.buffer("entry_items", entry_items),
			build.buffer("group_items", group_items),
			build.buffer("work_items", work_items),
			build.buffer("work_dispatches", records),
			scene.buffer("mesh_dispatch_work", work_items),
		]);
		Self {
			descriptor_set,
			pipelines: ["work-count", "work-ranges", "work-scatter"]
				.map(|name| pipeline_manager.request_pipeline(&format!("byte_engine/rendering/visibility/{name}.pipeline"))),
			instance_list,
			records,
			capacity: WORK_HEADER as u32 + meshlet_groups,
			phase_ends: [0; PHASES],
			listed: [0; PHASES],
			reported_overflow: false,
		}
	}

	/// Writes the frame's copy of the instance list from the opaque, masked, both double-sided, and transparent
	/// instance lists, each phase padded to whole groups.
	///
	/// Call it on the first frame of each frame sequence after the lists changed. A list whose work doesn't fit the
	/// work buffer, at the detail its instances draw when it's written, logs one warning. The GPU draws the
	/// instances whose work fits and leaves out the ones after them, whenever that happens.
	pub(crate) fn write_lists(&mut self, frame: &mut ghi::implementation::Frame, render_info: &RenderInfo) {
		let entries = frame.get_mut_dynamic_buffer_slice(self.instance_list);
		let lists = [
			&render_info.opaque_instances,
			&render_info.masked_instances,
			&render_info.double_sided_instances,
			&render_info.double_sided_masked_instances,
			&render_info.transparent_instances,
		];
		let mut written = 0;
		let mut items = 0;
		for (phase, list) in lists.into_iter().enumerate() {
			let end = (written + list.len()).next_multiple_of(GROUP_ENTRIES);
			for (entry, instance) in entries[written..written + list.len()].iter_mut().zip(list) {
				*entry = instance.shader_mesh_index;
			}
			entries[written + list.len()..end].fill(MeshDispatchWorkItem::EMPTY.packed);
			items += work_items(list);
			written = end;
			self.phase_ends[phase] = (end / GROUP_ENTRIES) as u32;
			self.listed[phase] = list.len() as u32;
		}
		frame.sync_buffer(self.instance_list);
		let capacity = self.capacity as usize - WORK_HEADER;
		crate::rendering::warn_once(&mut self.reported_overflow, items > capacity, || {
			format!(
				"Visibility left instances out of a frame. The most likely cause is that the frame's instances need {items} meshlet groups, more than the {capacity} the pipeline holds. Raise `render.instances.meshlet-group-capacity`."
			)
		});
	}

	/// Returns the frame's work, by raster phase.
	pub(crate) fn dispatches(&self) -> PhaseDispatches {
		let dispatch = |phase: usize| MeshDispatch {
			phase: phase as u32,
			listed: self.listed[phase],
			records: self.records,
		};
		PhaseDispatches {
			opaque_layer: [dispatch(0), dispatch(1), dispatch(2), dispatch(3)],
			transparent: dispatch(4),
		}
	}

	/// Returns the build pipelines once they have compiled. Until then no frame can draw meshlets.
	pub(crate) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<[ghi::PipelineHandle; 3]> {
		let [count, ranges, scatter] = self.pipelines;
		Some([
			pipeline_manager.pipeline(count)?,
			pipeline_manager.pipeline(ranges)?,
			pipeline_manager.pipeline(scatter)?,
		])
	}

	/// Records the passes that build the frame's work from its instance records: count each listed instance's items,
	/// lay the ranges out, and write the items. Record it before any pass that draws meshlets, and after anything
	/// that writes the frame's instance records.
	pub(crate) fn record(&self, c: &mut ghi::implementation::CommandBufferRecording, pipelines: [ghi::PipelineHandle; 3]) {
		use ghi::command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _};

		let [count, ranges, scatter] = pipelines;
		let entries = self.phase_ends[PHASES - 1] * GROUP_ENTRIES as u32;
		let groups =
			|| ghi::DispatchExtent::new(utils::Extent::line(entries.max(1)), utils::Extent::line(GROUP_ENTRIES as u32));
		c.start_region(|label| label.write_str("Mesh Work"));
		let bound = c.bind_compute_pipeline(count);
		bound.bind_descriptor_sets(&[self.descriptor_set]);
		bound.write_push_constant(0, [entries]);
		bound.dispatch(groups());
		let bound = c.bind_compute_pipeline(ranges);
		bound.bind_descriptor_sets(&[self.descriptor_set]);
		let [opaque, masked, double_sided, double_sided_masked, transparent] = self.phase_ends;
		bound.write_push_constant(
			0,
			[opaque, masked, double_sided, double_sided_masked, transparent, self.capacity],
		);
		bound.dispatch(ghi::DispatchExtent::new(utils::Extent::line(1), utils::Extent::line(1)));
		let bound = c.bind_compute_pipeline(scatter);
		bound.bind_descriptor_sets(&[self.descriptor_set]);
		bound.write_push_constant(0, [entries, self.capacity]);
		bound.dispatch(groups());
		c.end_region();
	}
}

/// Returns how many work items `instances` take: one for every 32 meshlets of each large instance, and the small
/// ones' meshlets rounded up to whole workgroups.
fn work_items(instances: &[Instance]) -> usize {
	let lanes = MESHLET_CULLING_TASK_GROUP_SIZE as usize;
	let (mut chunks, mut shared) = (0, 0);
	for instance in instances {
		if instance.meshlet_count > SHARED_INSTANCE_MESHLETS {
			chunks += instance.meshlet_count.div_ceil(MESHLET_CULLING_TASK_GROUP_SIZE) as usize;
		} else {
			shared += instance.meshlet_count as usize;
		}
	}
	chunks + shared.next_multiple_of(lanes)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn instance(shader_mesh_index: u32, meshlet_count: u32) -> Instance {
		Instance {
			shader_mesh_index,
			meshlet_count,
		}
	}

	#[test]
	fn large_instances_take_an_item_per_chunk_and_small_ones_share_workgroups() {
		let lanes = MESHLET_CULLING_TASK_GROUP_SIZE as usize;
		// Two chunks, then three meshlets that share one workgroup.
		let instances = [instance(7, 33), instance(11, 0), instance(19, 2), instance(23, 1)];

		assert_eq!(work_items(&instances), 2 + lanes);
		assert_eq!(work_items(&instances[..2]), 2);
		assert_eq!(work_items(&[]), 0);
	}

	#[test]
	fn work_items_address_instances_past_ten_bits() {
		let instance = (MAX_ADDRESSABLE_INSTANCES - 1) as u32;

		let work = MeshDispatchWorkItem::new(instance, CHUNK_MASK);

		assert_eq!((work.packed >> CHUNK_BITS, work.packed & CHUNK_MASK), (instance, CHUNK_MASK));
	}
}
