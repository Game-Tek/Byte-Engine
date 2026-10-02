//! Two-phase hierarchical-depth (HiZ) occlusion culling for the camera's visibility passes.
//!
//! The camera draws its opaque layer in two passes, each selected by an [`OcclusionPhase`]:
//!
//! 1. The early pass draws the meshlets the previous frame's late pass found unoccluded. [`OcclusionCulling`] then
//!    reduces that depth into a farthest-depth pyramid.
//! 2. The late pass projects every instance's and meshlet's bounding sphere into the pyramid, draws the unoccluded
//!    meshlets the early pass skipped, and records every unoccluded meshlet for the next frame's early pass.
//!
//! The pyramid only ever holds surfaces drawn this frame, so geometry an occluder uncovers, such as behind an opening
//! door or after a fast camera turn, shows on the frame it becomes visible. Stale records only cost time: they move
//! meshlets between the passes, and both passes read the same records.

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::{ComputeStage, record_compute_stages};
use super::super::layout::{OCCLUSION_PYRAMID_BINDING, OCCLUSION_VISIBILITY_BINDING};
use super::super::mesh_dispatch::MAX_MESH_DISPATCH_WORK_ITEMS;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::PipelineManagerClient;

/// The pyramid has a fixed extent, so its mip count does not depend on the sink's. Culling maps it over the whole
/// screen, whatever the sink's aspect ratio.
const OCCLUSION_PYRAMID_WIDTH: u32 = 512;
const OCCLUSION_PYRAMID_HEIGHT: u32 = 256;
/// Mip zero through the 2x1 level. `meshlet-task.besl` spells the last level as `OCCLUSION_PYRAMID_LAST_LEVEL`.
const OCCLUSION_PYRAMID_MIP_COUNT: u32 = 9;
const _: () = assert!(OCCLUSION_PYRAMID_WIDTH >> (OCCLUSION_PYRAMID_MIP_COUNT - 1) == 2);

// Both build stages read their source at 1033 and write their level at 1034.
const SOURCE_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1033);
const DESTINATION_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1034);

/// The `OcclusionPhase` enum selects how one meshlet pass culls by occlusion. `meshlet-task.besl` spells each phase as an
/// `OCCLUSION_*` constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub(crate) enum OcclusionPhase {
	/// Culls by frustum and normal cone only, as the shadow views do.
	Disabled = 0,
	/// Draws the meshlets the previous late pass found unoccluded. Build the pyramid from its depth.
	Early = 1,
	/// Draws the unoccluded meshlets the early pass skipped, and records every unoccluded meshlet for the next frame.
	Late = 2,
	/// Draws the unoccluded meshlets without recording them, as transparent geometry does.
	Test = 3,
}

impl OcclusionPhase {
	pub(super) fn label(self) -> &'static str {
		match self {
			Self::Disabled | Self::Test => "",
			Self::Early => " (Early)",
			Self::Late => " (Late)",
		}
	}
}

/// The `OcclusionCulling` struct owns one sink's occlusion pyramid, its record of unoccluded meshlets, and the
/// descriptor set the camera's visibility passes bind to cull against them.
///
/// Call [`Self::prepare`] each frame, and record the build between the early and the late opaque pass. Every meshlet
/// pass binds [`Self::descriptor_set`] next to the base set, because the shared task shader declares its resources.
pub(super) struct OcclusionCulling {
	descriptor_set: ghi::DescriptorSetHandle,
	/// Seeds mip zero from the sink's depth, then reduces each later level from the one above it.
	build_descriptor_sets: [ghi::DescriptorSetHandle; OCCLUSION_PYRAMID_MIP_COUNT as usize],
	seed_pipeline: crate::rendering::PipelineRef,
	reduce_pipeline: crate::rendering::PipelineRef,
}

pub(super) struct OcclusionPipelines {
	seed: ghi::PipelineHandle,
	reduce: ghi::PipelineHandle,
}

impl OcclusionCulling {
	/// Creates the pyramid and the unoccluded-meshlet record, and requests the build pipelines.
	///
	/// `depth` is the sink's visibility depth. Next, call [`Self::prepare`] each frame.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		depth: ghi::BaseImageHandle,
	) -> Self {
		let descriptor_set = context.create_descriptor_set(Some("Occlusion Culling Descriptor Set"));
		let build_descriptor_sets = std::array::from_fn(|_| context.create_descriptor_set(Some("Occlusion Pyramid Descriptor Set")));
		// Each frame builds and reads the pyramid within its own commands, and the queue orders frames, so one copy serves
		// every frame in flight. The record of unoccluded meshlets carries over to the next frame the same way.
		let pyramid = context.build_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Occlusion Pyramid")
				.extent(Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(OCCLUSION_PYRAMID_MIP_COUNT),
		);
		// Metal applies min/max reduction only when every sampler filter is linear. Culling samples whole levels, so
		// the linear mip filter never blends two of them.
		let min_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Min)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod((OCCLUSION_PYRAMID_MIP_COUNT - 1) as f32),
		);
		// The seed fetches exact texels, so the sampler only completes the descriptor.
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod(0.0),
		);
		let visibility = context.build_buffer::<[u32; MAX_MESH_DISPATCH_WORK_ITEMS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Occlusion Visibility")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let level = |set, slot, level| ghi::DescriptorWrite::image_mip(set, slot, pyramid, ghi::Layouts::General, level);
		let mut writes = vec![
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				OCCLUSION_PYRAMID_BINDING.slot(),
				pyramid,
				min_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::buffer(descriptor_set, OCCLUSION_VISIBILITY_BINDING.slot(), visibility.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				build_descriptor_sets[0],
				SOURCE_BINDING,
				depth,
				point_sampler,
				ghi::Layouts::Read,
			),
			level(build_descriptor_sets[0], DESTINATION_BINDING, 0),
		];
		for (index, &set) in build_descriptor_sets.iter().enumerate().skip(1) {
			writes.push(level(set, SOURCE_BINDING, index as u32 - 1));
			writes.push(level(set, DESTINATION_BINDING, index as u32));
		}
		context.write(&writes);

		Self {
			descriptor_set,
			build_descriptor_sets,
			seed_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/hiz-seed.pipeline"),
			reduce_pipeline: pipeline_manager.request_pipeline("byte-engine/rendering/visibility/hiz-reduce.pipeline"),
		}
	}

	/// Returns the set that gives the camera's task shader the pyramid and the record of unoccluded meshlets.
	pub(super) fn descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.descriptor_set
	}

	pub(super) fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<OcclusionPipelines> {
		Some(OcclusionPipelines {
			seed: pipeline_manager.pipeline(self.seed_pipeline)?,
			reduce: pipeline_manager.pipeline(self.reduce_pipeline)?,
		})
	}

	/// Returns the recording that builds the pyramid. Record it after the [`OcclusionPhase::Early`] pass and before the
	/// [`OcclusionPhase::Late`] pass.
	pub(super) fn prepare(&self, pipelines: OcclusionPipelines) -> impl RenderPassFunction + use<> {

		let stages: [ComputeStage; OCCLUSION_PYRAMID_MIP_COUNT as usize] = std::array::from_fn(|level| ComputeStage {
			label: if level == 0 { "Occlusion Pyramid Seed" } else { "Occlusion Pyramid Reduce" },
			pipeline: if level == 0 { pipelines.seed } else { pipelines.reduce },
			descriptor_sets: [self.build_descriptor_sets[level]],
			extent: Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT).mip(level as u32),
			workgroup: Extent::new(8, 8, 1),
		});
		move |c| record_compute_stages(c, Some("Occlusion Pyramid"), &stages)
	}
}
