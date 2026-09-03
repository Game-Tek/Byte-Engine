//! Per-sink GPU work: shadows, visibility rasterization, material prepasses, GTAO, and material evaluation.
//!
//! One [`VisibilityRenderPass`] exists per sink. It owns the sink's images, buffers, and descriptor sets, and
//! [`VisibilityRenderPass::prepare`] turns the frame's [`RenderInfo`] into one ordered recording.

mod gtao;
mod materials;
mod shadows;
mod visibility;

use std::num::NonZeroU32;

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

pub use self::gtao::GTAO_CONFIGURATION_PREFIX;
use self::gtao::GtaoPass;
pub(crate) use self::gtao::GtaoSettings;
use self::materials::{MaterialBuffers, MaterialEvaluationPass, MaterialPrepasses};
use self::shadows::ShadowPass;
pub(crate) use self::shadows::{DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT, ShadowWork};
use self::visibility::{VisibilityPass, VisibilityPhase};
use super::layout::{
	AO_MAP_BINDING, CONE_SHADOW_MAP_BINDING, CONE_SHADOW_MAP_FORMAT, DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING,
	DIRECTIONAL_SHADOW_MAP_FORMAT, INSTANCE_ID_BINDING, LIGHTING_DATA_BINDING, LIT_BINDING, MATERIAL_COUNT_BINDING,
	MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING,
	POINT_SHADOW_MAP_BINDING, POINT_SHADOW_MAP_FORMAT, SHADOW_CASCADE_COUNT, SHADOW_MAP_BINDING, SHADOW_MAP_RESOLUTION,
	TRIANGLE_INDEX_BINDING,
};
use super::mesh_dispatch::PhaseDispatches;
use super::scene::RenderInfo;
use super::shader_data::LightingData;
use super::skinning::SkinningPass;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink};

/// The `SinkTargets` struct names the render-graph images a sink gives the visibility pass.
#[derive(Clone, Copy)]
pub(crate) struct SinkTargets {
	pub(crate) lit: ghi::BaseImageHandle,
	pub(crate) depth: ghi::BaseImageHandle,
	pub(crate) primitive_index: ghi::BaseImageHandle,
	pub(crate) instance_id: ghi::BaseImageHandle,
}

/// The `VisibilityRenderPass` struct sequences visibility-buffer work for one sink and scene frame.
pub(crate) struct VisibilityRenderPass {
	pipeline_manager: PipelineManagerClient,
	shadows: ShadowPass,
	visibility: VisibilityPass,
	material_prepasses: MaterialPrepasses,
	gtao: GtaoPass,
	material_evaluation: MaterialEvaluationPass,
}

impl VisibilityRenderPass {
	/// Creates every per-sink GPU resource and requests the fixed visibility pipelines.
	///
	/// The material-evaluation descriptor set still needs the environment written by the pipeline manager;
	/// see [`Self::material_evaluation_descriptor_set`].
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		targets: SinkTargets,
		cone_shadow_pool_capacity: usize,
		point_shadow_pool_capacity: usize,
		gtao_settings: GtaoSettings,
	) -> Self {
		let visibility_descriptor_set = context.create_descriptor_set(Some("Visibility Descriptor Set"));
		let material_evaluation_descriptor_set = context.create_descriptor_set(Some("Material Evaluation Descriptor Set"));
		let material_buffers = MaterialBuffers::new(context);
		fn depth_map(format: ghi::Formats, name: &str) -> ghi::image::Builder<'_> {
			ghi::image::Builder::new(format, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.optimized_clear_value(ghi::ClearValue::Depth(0.0))
		}
		let ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let directional_shadow_map = context.build_dynamic_image(
			depth_map(DIRECTIONAL_SHADOW_MAP_FORMAT, "Directional Shadow Map")
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32)),
		);
		let directional_shadow_depth_pyramid = context.build_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("Directional Shadow Depth Pyramid")
				.extent(Extent::rectangle(
					SHADOW_MAP_RESOLUTION / 4,
					SHADOW_MAP_RESOLUTION / 4 * SHADOW_CASCADE_COUNT as u32,
				))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT),
		);
		// Dynamic images start at zero extent, so these pools have no backing maps until a visible light uses them.
		// Metal requires two layers to create the array texture that material evaluation always binds.
		let cone_shadow_map = context.build_dynamic_image(
			depth_map(CONE_SHADOW_MAP_FORMAT, "Cone Shadow Map")
				.array_layers(NonZeroU32::new(cone_shadow_pool_capacity.max(2) as u32)),
		);
		let point_shadow_map = context.build_dynamic_image(
			depth_map(POINT_SHADOW_MAP_FORMAT, "Point Shadow Map").cube_array_compatible(
				NonZeroU32::new(point_shadow_pool_capacity.max(1) as u32)
					.expect("Point shadow map pool has a nonzero fallback cube."),
			),
		);
		let linear_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let depth_pyramid_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod((DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let sampled = |binding: ghi::ShaderResourceDescriptor, image: ghi::BaseImageHandle, sampler| {
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				binding.slot(),
				image,
				sampler,
				ghi::Layouts::Read,
			)
		};
		let visibility_buffer = |binding: ghi::ShaderResourceDescriptor, buffer: ghi::BaseBufferHandle| {
			ghi::DescriptorWrite::buffer(visibility_descriptor_set, binding.slot(), buffer)
		};
		context.write(&[
			ghi::DescriptorWrite::image(
				material_evaluation_descriptor_set,
				LIT_BINDING.slot(),
				targets.lit,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHTING_DATA_BINDING.slot(),
				lighting_buffer.into(),
			),
			sampled(AO_MAP_BINDING, ao_map.into(), linear_sampler),
			sampled(SHADOW_MAP_BINDING, directional_shadow_map.into(), depth_sampler),
			sampled(
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING,
				directional_shadow_depth_pyramid.into(),
				depth_pyramid_sampler,
			),
			sampled(CONE_SHADOW_MAP_BINDING, cone_shadow_map.into(), depth_sampler),
			sampled(POINT_SHADOW_MAP_BINDING, point_shadow_map.into(), depth_sampler),
			visibility_buffer(MATERIAL_COUNT_BINDING, material_buffers.count.into()),
			visibility_buffer(MATERIAL_OFFSET_BINDING, material_buffers.offset.into()),
			visibility_buffer(MATERIAL_OFFSET_SCRATCH_BINDING, material_buffers.offset_scratch.into()),
			visibility_buffer(
				MATERIAL_EVALUATION_DISPATCHES_BINDING,
				material_buffers.evaluation_dispatches.into(),
			),
			visibility_buffer(MATERIAL_XY_BINDING, material_buffers.pixel_mapping.into()),
			ghi::DescriptorWrite::image(
				visibility_descriptor_set,
				TRIANGLE_INDEX_BINDING.slot(),
				targets.primitive_index,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				visibility_descriptor_set,
				INSTANCE_ID_BINDING.slot(),
				targets.instance_id,
				ghi::Layouts::General,
			),
		]);

		Self {
			shadows: ShadowPass::new(
				context,
				&pipeline_manager,
				base_descriptor_set,
				directional_shadow_map.into(),
				directional_shadow_depth_pyramid.into(),
				cone_shadow_map.into(),
				point_shadow_map.into(),
			),
			visibility: VisibilityPass::new(
				&pipeline_manager,
				base_descriptor_set,
				targets.primitive_index,
				targets.instance_id,
				targets.depth,
			),
			material_prepasses: MaterialPrepasses::new(
				&pipeline_manager,
				base_descriptor_set,
				visibility_descriptor_set,
				material_buffers.count,
			),
			gtao: GtaoPass::new(context, &pipeline_manager, targets.depth, ao_map.into(), gtao_settings),
			material_evaluation: MaterialEvaluationPass::new(
				targets.lit,
				base_descriptor_set,
				visibility_descriptor_set,
				material_evaluation_descriptor_set,
				material_buffers.evaluation_dispatches,
			),
			pipeline_manager,
		}
	}

	pub(crate) fn set_gtao_settings(&mut self, settings: GtaoSettings) {
		self.gtao.set_settings(settings);
	}

	/// Returns the descriptor set that carries material-evaluation-only resources, including the environment.
	pub(crate) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation.descriptor_set
	}

	/// Prepares one opaque visibility layer and one nearest-surface transparent layer.
	///
	/// Returns `None` while any fixed pipeline is still compiling. `skinning` is passed only by the first sink
	/// so deformation runs once per frame.
	pub(crate) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		skinning: Option<&'a SkinningPass>,
		dispatches: PhaseDispatches,
		render_info: &'a RenderInfo,
		shadow_work: ShadowWork,
	) -> Option<impl RenderPassFunction + use<'a>> {
		let pipeline_manager = &self.pipeline_manager;
		let skinning = match skinning {
			Some(pass) => Some((pass, pipeline_manager.pipeline(pass.pipeline())?)),
			None => None,
		};
		let visibility_pipelines = self.visibility.pipelines(pipeline_manager)?;
		let prepass_pipelines = self.material_prepasses.pipelines(pipeline_manager)?;
		let shadows = self.shadows.prepare(frame, pipeline_manager, dispatches, shadow_work)?;
		let gtao = self.gtao.prepare(frame, sink, self.gtao.pipelines(pipeline_manager)?);
		let opaque_materials = self.material_evaluation.prepare(
			&render_info.opaque_materials,
			&render_info.opaque_material_mask,
			VisibilityPhase::Opaque,
		);
		let transparent_materials = self.material_evaluation.prepare(
			&render_info.transparent_materials,
			&render_info.transparent_material_mask,
			VisibilityPhase::Transparent,
		);
		let extent = sink.extent();
		let visibility = &self.visibility;
		let material_prepasses = &self.material_prepasses;

		Some(
			move |c: &mut ghi::implementation::CommandBufferRecording, t: &[ghi::AttachmentInformation]| {
				use ghi::command_buffer::CommonCommandBufferMode as _;

				c.start_region(|label| label.write_str("Visibility Render Model"));
				if let Some((pass, pipeline)) = skinning {
					pass.record(c, &render_info.skinning_dispatches, pipeline);
				}
				shadows(c, t);

				// The opaque layer establishes the depth and color retained by every later transparent primitive.
				visibility.record(
					c,
					extent,
					VisibilityPhase::Opaque,
					dispatches.opaque,
					dispatches.masked,
					visibility_pipelines,
				);
				material_prepasses.record(c, extent, prepass_pipelines);
				gtao(c, t);
				opaque_materials(c, t);

				// The visibility buffer holds one transparent layer. Resolving every blend primitive together lets
				// normal depth testing select the nearest surface before source-over evaluation.
				if !dispatches.transparent.is_empty() {
					visibility.record(
						c,
						extent,
						VisibilityPhase::Transparent,
						dispatches.transparent,
						Default::default(),
						visibility_pipelines,
					);
					material_prepasses.record(c, extent, prepass_pipelines);
					transparent_materials(c, t);
				}
				c.end_region();
			},
		)
	}
}
