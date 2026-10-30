//! The hits of a ray-traced technique's rays, which surfel GI and probe GI share: the trace keeps each ray's hit, a
//! counting sort orders the hits by the material they hit, and the shade pass runs each material's closest-hit shader
//! for a run of hits side by side.
//!
//! The visibility pipeline manager creates one [`RayHits`] when the project builds either technique in, since only one
//! of them lights a frame. Each frame a technique lights, its `prepare` pairs its trace with [`RayHits::sort`] and the
//! shade pass that [`RayTracing`] names into [`RayStages`], which the first sink records.

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::{ComputeStage, Pipelines};
use crate::rendering::PipelineManagerClient;
use crate::rendering::pipelines::visibility::set_layouts;

/// The hit sort's compute pipelines, named as [`Pipelines::request`] takes them, in recording order.
pub(in crate::rendering::pipelines::visibility) const RAY_HIT_PIPELINES: [&str; 2] =
	["ray-hit-sort-scan", "ray-hit-sort-scatter"];
/// The material slots hits sort by, which `RayHitSort` in `layouts/ray-hits.besl` sizes its tables for.
const HIT_KEYS: usize = super::super::layout::MAX_MATERIALS;
/// The threads of the sort's one scan workgroup, and of each scatter workgroup. Matches `HIT_SORT_WORKGROUP_SIZE` in
/// `layouts/ray-hits.besl`.
const HIT_SORT_WORKGROUP_SIZE: u32 = 256;

/// The `RayHit` struct is one ray's hit, kept for the shade pass. It matches `RayHit` in `layouts/ray-hits.besl`; only
/// the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct RayHit {
	origin: ghi::pod::Vec3f,
	distance: f32,
	direction: ghi::pod::Vec3f,
	record: u32,
	barycentrics: [f32; 2],
	custom_index: u32,
	geometry_index: u32,
	primitive_index: u32,
	_padding: [u32; 3],
}

/// The `RayHitSort` struct is the counting sort of a frame's hits. It matches `RayHitSort` in `layouts/ray-hits.besl`;
/// only the GPU reads and writes it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct RayHitSort {
	counts: [u32; HIT_KEYS],
	offsets: [u32; HIT_KEYS],
	hit_count: u32,
	rays: u32,
}

/// The `RayTracing` struct names what a frame's rays trace and shade with: the scene sets they read, and the ray tracing
/// pipeline and shader binding table that run each hit's closest-hit shader.
///
/// The visibility pipeline manager assembles it from the scene, [`super::super::acceleration_structures`], and the
/// technique's [`super::super::ray_tracing::HitShading::prepare`], and hands it to the technique's `prepare`.
#[derive(Clone, Copy)]
pub(crate) struct RayTracing {
	pub(crate) scene: ghi::DescriptorSetHandle,
	pub(crate) ray_traced_scene: ghi::DescriptorSetHandle,
	pub(crate) pipeline: ghi::PipelineHandle,
	pub(crate) table: ghi::ShaderBindingTableHandle,
}

/// The `RayHits` struct holds the hits of the frame's rays and the sort that orders them, for the technique that lights
/// the frame.
///
/// The visibility pipeline manager creates one when the project builds surfel GI or probe GI in. Hand it to the
/// technique's `prepare`, which binds [`Self::descriptor_set`] after its own sets and sorts with [`Self::sort`].
pub(crate) struct RayHits {
	descriptor_set: ghi::DescriptorSetHandle,
	/// The sort's pipelines, in the order of [`RAY_HIT_PIPELINES`].
	pipelines: Pipelines<2>,
}

impl RayHits {
	/// Creates room for the hits of `ray_budget` rays a frame, the largest budget of the techniques the project built
	/// in, and requests the sort's pipelines.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		ray_budget: u32,
	) -> Self {
		let descriptor_set = set_layouts::RAY_HITS.create_set(context, "Ray Hits Descriptor Set");
		// Every buffer starts zeroed, which is zero hit counts.
		let device_buffer = |name, length: u32| {
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.length(length as usize)
		};
		let hits: ghi::BufferHandle<[RayHit]> = context.build_buffer(device_buffer("Ray Hits", ray_budget));
		let keys: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Ray Hit Keys", ray_budget));
		let sort: ghi::BufferHandle<RayHitSort> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Ray Hit Sort")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let sorted: ghi::BufferHandle<[u32]> = context.build_buffer(device_buffer("Ray Sorted Hits", ray_budget));
		let set = set_layouts::RAY_HITS.binder(descriptor_set);
		context.write(&[
			set.buffer("ray_hits", hits),
			set.buffer("ray_hit_keys", keys),
			set.buffer("ray_hit_sort", sort),
			set.buffer("ray_sorted_hits", sorted),
		]);
		Self {
			descriptor_set,
			pipelines: Pipelines::request(pipeline_manager, RAY_HIT_PIPELINES),
		}
	}

	/// Returns the set a technique's trace and shade passes bind after their own sets.
	pub(super) fn descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.descriptor_set
	}

	/// Returns the sort of the hits of a technique whose trace runs once for each of `rays` rays, or `None` while a sort
	/// pipeline is still compiling.
	pub(super) fn sort(&self, pipeline_manager: &PipelineManagerClient, rays: u32) -> Option<[ComputeStage; 2]> {
		let [scan, scatter] = self.pipelines.resolve(pipeline_manager)?;
		let stage = |label, pipeline, threads| ComputeStage {
			label,
			pipeline,
			descriptor_sets: [self.descriptor_set],
			extent: Extent::line(threads),
			workgroup: Extent::line(HIT_SORT_WORKGROUP_SIZE),
		};
		Some([
			stage("Ray Hit Sort Scan", scan, HIT_SORT_WORKGROUP_SIZE),
			stage("Ray Hit Sort Scatter", scatter, rays),
		])
	}
}

/// The `RayStages` struct holds one frame's ray work: the trace, the hit sort, and the shade pass, which the first sink
/// records in that order.
///
/// `N` is how many descriptor sets the trace and shade passes bind: the scene's, the technique's, then
/// [`RayHits::descriptor_set`].
#[derive(Clone, Copy)]
pub(crate) struct RayStages<const N: usize> {
	/// Names the debug region the stages record in.
	pub(super) label: &'static str,
	/// Traces the frame's rays, writing misses and keeping hits for the sort.
	pub(super) trace: ComputeStage<N>,
	/// Orders the hits by the material they hit.
	pub(super) sort: [ComputeStage; 2],
	/// Runs each sorted hit's closest-hit shader and lights the surface it reports.
	pub(super) shade: ShadeStage<N>,
}

impl<const N: usize> RayStages<N> {
	/// Records the trace, the hit sort, and the shade pass, in that order.
	pub(super) fn record(&self, c: &mut ghi::implementation::CommandBufferRecording) {
		use ghi::command_buffer::CommonCommandBufferMode as _;

		c.start_region(|label| label.write_str(self.label));
		super::record_compute_stages(c, None, &[self.trace]);
		super::record_compute_stages(c, None, &self.sort);
		self.shade.record(c);
		c.end_region();
	}
}

/// The `ShadeStage` struct is a frame's shade pass: one ray generation invocation per ray of the technique's budget, of
/// which those past the frame's hit count return at once.
#[derive(Clone, Copy)]
pub(super) struct ShadeStage<const N: usize> {
	pub(super) label: &'static str,
	pub(super) tracing: RayTracing,
	pub(super) descriptor_sets: [ghi::DescriptorSetHandle; N],
	/// The technique's ray budget, which bounds its hits.
	pub(super) rays: u32,
}

impl<const N: usize> ShadeStage<N> {
	fn record(&self, c: &mut ghi::implementation::CommandBufferRecording) {
		use ghi::command_buffer::{
			BoundPipelineLayoutMode as _, BoundRayTracingPipelineMode as _, CommonCommandBufferMode as _,
		};

		c.start_region(|label| label.write_str(self.label));
		let c = c.bind_ray_tracing_pipeline(self.tracing.pipeline);
		c.bind_descriptor_sets(&self.descriptor_sets);
		c.trace_rays(self.tracing.table, Extent::line(self.rays));
		c.end_region();
	}
}

#[cfg(test)]
mod tests {
	use std::mem::size_of;

	use super::*;
	use crate::rendering::SetLayout;

	#[test]
	fn host_records_match_the_strides_their_layout_layers_reflect() {
		let stride = |layout: &SetLayout, name: &str| {
			layout
				.interface()
				.binding(name)
				.and_then(|binding| binding.buffer_stride)
				.unwrap_or_else(|| panic!("Expected the layout to declare the buffer `{name}`")) as usize
		};

		assert_eq!(stride(&set_layouts::RAY_HITS, "ray_hits"), size_of::<RayHit>());
		assert_eq!(stride(&set_layouts::RAY_HITS, "ray_hit_sort"), size_of::<RayHitSort>());
	}
}
