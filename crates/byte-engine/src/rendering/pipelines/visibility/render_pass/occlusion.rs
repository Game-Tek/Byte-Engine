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

use super::super::layout::{OCCLUSION_PYRAMID_BINDING, OCCLUSION_VISIBILITY_BINDING};
use super::super::mesh_dispatch::MAX_MESH_DISPATCH_WORK_ITEMS;
use super::{ComputeStage, Pipelines, record_compute_stages};
use crate::rendering::PipelineManagerClient;
use crate::rendering::render_pass::RenderPassFunction;

/// The pyramid has a fixed extent, so its mip count does not depend on the sink's. Culling maps it over the whole
/// screen, whatever the sink's aspect ratio.
pub(crate) const OCCLUSION_PYRAMID_WIDTH: u32 = 512;
pub(crate) const OCCLUSION_PYRAMID_HEIGHT: u32 = 256;
/// Mip zero through the 2x1 level. `meshlet-task.besl` spells the last level as `OCCLUSION_PYRAMID_LAST_LEVEL`.
pub(crate) const OCCLUSION_PYRAMID_MIP_COUNT: u32 = 9;
const _: () = assert!(OCCLUSION_PYRAMID_WIDTH >> (OCCLUSION_PYRAMID_MIP_COUNT - 1) == 2);
/// The build's first stage seeds mip zero in 16x16 blocks and reduces each block through workgroup memory to one
/// texel of mip four, so mip zero must be whole blocks; the second stage takes mip four, 32x16, to the top in one
/// 16x8 workgroup. `hiz-base.besl` and `hiz-tail.besl` spell these shapes.
const BASE_LEVELS: usize = 5;
const BASE_BLOCK: u32 = 16;
const _: () =
	assert!(OCCLUSION_PYRAMID_WIDTH.is_multiple_of(BASE_BLOCK) && OCCLUSION_PYRAMID_HEIGHT.is_multiple_of(BASE_BLOCK));
const _: () =
	assert!(OCCLUSION_PYRAMID_WIDTH >> (BASE_LEVELS - 1) == 32 && OCCLUSION_PYRAMID_HEIGHT >> (BASE_LEVELS - 1) == 16);

// Both build stages read their source at 1033 and write their levels from 1034 up.
const SOURCE_BINDING: ghi::ResourceSlot = ghi::ResourceSlot::new(1033);
const FIRST_LEVEL_BINDING: u32 = 1034;

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

/// The `OcclusionCulling` struct owns one sink's occlusion pyramid, its record of unoccluded meshlets, and the
/// descriptor set the camera's visibility passes bind to cull against them.
///
/// Call [`Self::prepare`] each frame, and record the build between the early and the late opaque pass. Every meshlet
/// pass binds [`Self::descriptor_set`] next to the base set, because the shared task shader declares its resources.
pub(super) struct OcclusionCulling {
	/// Gives the camera's task shader the pyramid and the record of unoccluded meshlets.
	pub(super) descriptor_set: ghi::DescriptorSetHandle,
	/// The base stage's set, with the sink's depth and mips zero through four, and the tail stage's, with mip four
	/// and the levels above it.
	build_descriptor_sets: [ghi::DescriptorSetHandle; 2],
	/// The base and tail pipelines.
	pipelines: Pipelines<2>,
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
		let build_descriptor_sets = [
			context.create_descriptor_set(Some("Occlusion Pyramid Base Descriptor Set")),
			context.create_descriptor_set(Some("Occlusion Pyramid Tail Descriptor Set")),
		];
		// Each frame builds and reads the pyramid within its own commands, and the queue orders frames, so one copy serves
		// every frame in flight. The record of unoccluded meshlets carries over to the next frame the same way.
		// The seed rounds each depth toward the far plane before storing it in 16 bits, so culling stays conservative.
		let pyramid = context.build_image(
			ghi::image::Builder::new(ghi::Formats::R16F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Occlusion Pyramid")
				.extent(Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(OCCLUSION_PYRAMID_MIP_COUNT),
		);
		// Metal applies min/max reduction only when every sampler filter is linear, as the default ones are. Culling
		// samples whole levels, so the linear mip filter never blends two of them.
		let min_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.reduction_mode(ghi::SamplingReductionModes::Min)
				.max_lod((OCCLUSION_PYRAMID_MIP_COUNT - 1) as f32),
		);
		// The seed fetches exact texels, so the sampler only completes the descriptor.
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		let visibility = context.build_buffer::<[u32; MAX_MESH_DISPATCH_WORK_ITEMS]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Occlusion Visibility")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let level = |set, slot, level| ghi::DescriptorWrite::image_mip(set, slot, pyramid, ghi::Layouts::General, level);
		let [base_set, tail_set] = build_descriptor_sets;
		let mut writes = vec![
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				OCCLUSION_PYRAMID_BINDING.slot(),
				pyramid,
				min_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::buffer(descriptor_set, OCCLUSION_VISIBILITY_BINDING.slot(), visibility.into()),
			ghi::DescriptorWrite::combined_image_sampler(base_set, SOURCE_BINDING, depth, point_sampler, ghi::Layouts::Read),
			level(tail_set, SOURCE_BINDING, BASE_LEVELS as u32 - 1),
		];
		for mip in 0..OCCLUSION_PYRAMID_MIP_COUNT {
			let (set, first) = if (mip as usize) < BASE_LEVELS {
				(base_set, 0)
			} else {
				(tail_set, BASE_LEVELS as u32)
			};
			writes.push(level(set, ghi::ResourceSlot::new(FIRST_LEVEL_BINDING + mip - first), mip));
		}
		context.write(&writes);

		Self {
			descriptor_set,
			build_descriptor_sets,
			pipelines: Pipelines::request(pipeline_manager, ["hiz-base", "hiz-tail"]),
		}
	}

	/// Returns the recording that builds the pyramid, or `None` while a pipeline is still compiling. Record it after the
	/// [`OcclusionPhase::Early`] pass and before the [`OcclusionPhase::Late`] pass.
	pub(super) fn prepare(&self, pipeline_manager: &PipelineManagerClient) -> Option<impl RenderPassFunction + use<>> {
		let [base, tail] = self.pipelines.resolve(pipeline_manager)?;
		let [base_set, tail_set] = self.build_descriptor_sets;
		let stages = [
			ComputeStage {
				label: "Occlusion Pyramid Base",
				pipeline: base,
				descriptor_sets: [base_set],
				extent: Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT),
				workgroup: Extent::new(BASE_BLOCK, BASE_BLOCK, 1),
			},
			ComputeStage {
				label: "Occlusion Pyramid Tail",
				pipeline: tail,
				descriptor_sets: [tail_set],
				extent: Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT).mip(BASE_LEVELS as u32),
				workgroup: Extent::rectangle(OCCLUSION_PYRAMID_WIDTH, OCCLUSION_PYRAMID_HEIGHT).mip(BASE_LEVELS as u32),
			},
		];
		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			record_compute_stages(c, Some("Occlusion Pyramid"), &stages)
		})
	}
}
