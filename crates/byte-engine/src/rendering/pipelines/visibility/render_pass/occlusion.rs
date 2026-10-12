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

use super::super::mesh_dispatch::MAX_MESH_DISPATCH_WORK_ITEMS;
use super::{ComputeStage, Pipelines, record_compute_stages};
use crate::rendering::PipelineManagerClient;
use crate::rendering::pipelines::visibility::set_layouts;
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

/// The pyramid's levels, as `layouts/hiz-base.besl` and `layouts/hiz-tail.besl` name the image each one is written
/// through. The base stage writes the first [`BASE_LEVELS`] and the tail stage the rest.
const LEVEL_NAMES: [&str; OCCLUSION_PYRAMID_MIP_COUNT as usize] = [
	"level_0", "level_1", "level_2", "level_3", "level_4", "level_5", "level_6", "level_7", "level_8",
];

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
		let descriptor_set = set_layouts::OCCLUSION.create_set(context, "Occlusion Culling Descriptor Set");
		let build_descriptor_sets = [
			set_layouts::HIZ_BASE.create_set(context, "Occlusion Pyramid Base Descriptor Set"),
			set_layouts::HIZ_TAIL.create_set(context, "Occlusion Pyramid Tail Descriptor Set"),
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
		let [base_set, tail_set] = build_descriptor_sets;
		let culling = set_layouts::OCCLUSION.binder(descriptor_set);
		let base = set_layouts::HIZ_BASE.binder(base_set);
		let tail = set_layouts::HIZ_TAIL.binder(tail_set);
		let mut writes = smallvec::SmallVec::<[ghi::DescriptorWrite; 16]>::from_iter([
			culling.sampled("occlusion_pyramid", pyramid, min_sampler),
			culling.buffer("occlusion_visibility", visibility),
			base.sampled("source_depth", depth, point_sampler),
			// The tail stage reduces the last level the base stage wrote.
			tail.image_mip("source_level", pyramid, BASE_LEVELS as u32 - 1),
		]);
		for (mip, name) in LEVEL_NAMES.into_iter().enumerate() {
			let stage = if mip < BASE_LEVELS { base } else { tail };
			writes.push(stage.image_mip(name, pyramid, mip as u32));
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
