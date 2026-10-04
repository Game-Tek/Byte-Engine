//! Per-sink GPU work: shadows, light clusters, visibility rasterization, material prepasses, the cascade fit, the
//! occlusion and linear depth pyramids, contact shadows, GTAO, SSGI, and material evaluation, which also traces
//! screen-space reflections. GTAO and SSGI each run only while their settings enable them.
//!
//! One [`VisibilityRenderPass`] exists per sink. It owns the sink's images, buffers, and descriptor sets, and
//! [`VisibilityRenderPass::prepare`] turns the frame's [`RenderInfo`] into one ordered recording.

mod contact_shadows;
mod depth_pyramid;
mod gtao;
mod light_clusters;
mod materials;
mod occlusion;
mod reflections;
mod shadows;
mod ssgi;
mod visibility;

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

/// The `ComputeStage` struct is one dispatch of a visibility compute subpass, so every subpass records the same way
/// through [`record_compute_stages`].
///
/// `N` is how many descriptor sets the stage binds, in binding order.
#[derive(Clone, Copy)]
pub(super) struct ComputeStage<const N: usize = 1> {
	pub(super) label: &'static str,
	pub(super) pipeline: ghi::PipelineHandle,
	pub(super) descriptor_sets: [ghi::DescriptorSetHandle; N],
	pub(super) extent: Extent,
	pub(super) workgroup: Extent,
}

/// Records each stage in its own debug region, nested inside `region` when one is given.
///
/// Nothing is recorded for an empty stage list, not even `region`.
pub(super) fn record_compute_stages<const N: usize>(
	c: &mut ghi::implementation::CommandBufferRecording,
	region: Option<&'static str>,
	stages: &[ComputeStage<N>],
) {
	use ghi::command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _};

	if stages.is_empty() {
		return;
	}
	if let Some(region) = region {
		c.start_region(|label| label.write_str(region));
	}
	for stage in stages {
		c.start_region(|label| label.write_str(stage.label));
		let c = c.bind_compute_pipeline(stage.pipeline);
		c.bind_descriptor_sets(&stage.descriptor_sets);
		c.dispatch(ghi::DispatchExtent::new(stage.extent, stage.workgroup));
		c.end_region();
	}
	if region.is_some() {
		c.end_region();
	}
}

/// The `PhasePipelines` struct holds one raster pipeline per opaque-layer work range, so the visibility and shadow
/// passes pair them with [`PhaseDispatches::opaque_layer`] the same way.
pub(super) struct PhasePipelines([crate::rendering::PipelineRef; 4]);

impl PhasePipelines {
	/// Requests the pipeline assets named by `names`: solid, masked, double-sided, and double-sided masked.
	pub(super) fn request(pipeline_manager: &PipelineManagerClient, names: [&str; 4]) -> Self {
		Self(names.map(|name| pipeline_manager.request_pipeline(name)))
	}

	/// Returns the compiled pipelines in request order, or `None` while any is still compiling.
	pub(super) fn resolve(&self, pipeline_manager: &PipelineManagerClient) -> Option<[ghi::PipelineHandle; 4]> {
		let [solid, masked, double_sided, double_sided_masked] = self.0;
		Some([
			pipeline_manager.pipeline(solid)?,
			pipeline_manager.pipeline(masked)?,
			pipeline_manager.pipeline(double_sided)?,
			pipeline_manager.pipeline(double_sided_masked)?,
		])
	}
}

/// Records one mesh dispatch per non-empty work range and batch of up to [`MAX_TASK_VIEWS`] views. The batches draw
/// `view_count` packed views from `view_base` into consecutive layers from zero.
///
/// `descriptor_sets` are the base set and a sink's [`OcclusionCulling::descriptor_set`]. The shared task shader declares
/// the occlusion resources, so every pass binds them, but only `occlusion` other than [`OcclusionPhase::Disabled`] uses
/// them. Call it inside a render pass whose attachments hold `view_count` layers, or one unlayered target for one view.
pub(super) fn record_meshlet_dispatches(
	c: &mut impl ghi::command_buffer::RasterizationRenderPassMode,
	descriptor_sets: [ghi::DescriptorSetHandle; 2],
	occlusion: OcclusionPhase,
	ranges: impl IntoIterator<Item = (MeshDispatch, ghi::PipelineHandle)>,
	view_base: usize,
	view_count: usize,
) {
	use ghi::command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _};

	for (dispatch, pipeline) in ranges {
		if dispatch.is_empty() {
			continue;
		}
		let c = c.bind_raster_pipeline(pipeline);
		c.bind_descriptor_sets(&descriptor_sets);
		for first_layer in (0..view_count).step_by(MAX_TASK_VIEWS) {
			let batch_views = (view_count - first_layer).min(MAX_TASK_VIEWS);
			c.write_push_constant(
				0,
				[
					dispatch.work_item_base(),
					(view_base + first_layer) as u32,
					first_layer as u32,
					batch_views as u32,
					occlusion as u32,
				],
			);
			c.dispatch_meshes(dispatch.workgroup_count(), 1, 1);
		}
	}
}

pub use self::contact_shadows::CONTACT_SHADOWS_CONFIGURATION_PREFIX;
use self::contact_shadows::ContactShadowPass;
pub(crate) use self::contact_shadows::{ContactShadowSettings, ContactShadowTargets, create_contact_shadow_targets};
use self::depth_pyramid::DepthPyramidPass;
pub use self::gtao::GTAO_CONFIGURATION_PREFIX;
use self::gtao::GtaoPass;
pub(crate) use self::gtao::GtaoSettings;
use self::light_clusters::LightClusterPass;
use self::materials::{MaterialBuffers, MaterialEvaluationPass, MaterialPrepasses, ScreenSpaceLighting};
use self::occlusion::OcclusionCulling;
pub(crate) use self::occlusion::OcclusionPhase;
use self::reflections::ScreenSpaceReflections;
pub(crate) use self::reflections::create_radiance_history_target;
use self::shadows::CascadeFitPass;
pub(crate) use self::shadows::{DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT, ShadowMaps, ShadowWork};
pub use self::ssgi::SSGI_CONFIGURATION_PREFIX;
use self::ssgi::SsgiPass;
pub(crate) use self::ssgi::{SsgiSettings, SsgiTargets, create_ssgi_targets};
use self::visibility::{VisibilityPass, VisibilityPhase};
#[cfg(test)]
pub(crate) use self::{
	depth_pyramid::screen_view_data,
	shadows::{ReceiverFitShaderData, receiver_fit_shader_data},
};
use super::layout::{
	AO_MAP_BINDING, CONE_SHADOW_MAP_BINDING, CONTACT_SHADOW_MAP_BINDING, DIFFUSE_RADIANCE_HISTORY_BINDING,
	DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING, INDIRECT_DIFFUSE_MAP_BINDING, INSTANCE_ID_BINDING, LIGHTING_DATA_BINDING,
	LIT_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
	MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_TASK_VIEWS, POINT_SHADOW_MAP_BINDING, SHADOW_MAP_BINDING,
	TRIANGLE_INDEX_BINDING,
};
use super::mesh_dispatch::{MeshDispatch, PhaseDispatches};
use super::scene::RenderInfo;
use super::shader_data::LightingData;
use super::skinning::SkinningPass;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The `SinkTargets` struct names the render-graph images a sink gives the visibility pass.
#[derive(Clone, Copy)]
pub(crate) struct SinkTargets {
	pub(crate) lit: ghi::BaseImageHandle,
	pub(crate) depth: ghi::BaseImageHandle,
	pub(crate) primitive_index: ghi::BaseImageHandle,
	pub(crate) instance_id: ghi::BaseImageHandle,
	/// The SSGI images, including the diffuse light that opaque material evaluation writes for next frame's rays.
	pub(crate) ssgi: SsgiTargets,
	/// The sun's contact shadows, which opaque material evaluation multiplies into the sun's shadow.
	pub(crate) contact_shadows: ContactShadowTargets,
	/// The light opaque material evaluation writes for next frame's reflection rays.
	pub(crate) radiance_history: ghi::DynamicImageHandle,
}

/// The `FrameWork` struct names the work one sink records for the whole frame: deforming skinned meshes and drawing
/// the shadow maps every sink samples.
///
/// The visibility pipeline manager passes it to the first sink's [`VisibilityRenderPass::prepare`], whose camera the
/// shadow views were made for, so that sink also fits the cascades to its surfaces.
#[derive(Clone, Copy)]
pub(crate) struct FrameWork<'a> {
	pub(crate) skinning: &'a SkinningPass,
	pub(crate) shadow_maps: &'a ShadowMaps,
}

/// The `SinkHistory` struct describes what the previous frame's history images hold for one sink.
///
/// Temporal passes use it to reproject into those images. The visibility pipeline manager builds it from the sink
/// it recorded last frame.
#[derive(Clone, Copy)]
pub(crate) struct SinkHistory {
	/// The view the sink was recorded with.
	pub(crate) view: View,
	/// The exposure the recorded light was multiplied by.
	pub(crate) exposure: f32,
	/// Whether SSGI ran, so its history and the diffuse radiance history hold this sink's data.
	pub(crate) ssgi: bool,
}

/// The `VisibilityRenderPass` struct sequences visibility-buffer work for one sink and scene frame.
pub(crate) struct VisibilityRenderPass {
	pipeline_manager: PipelineManagerClient,
	cascade_fit: CascadeFitPass,
	light_clusters: LightClusterPass,
	visibility: VisibilityPass,
	occlusion: OcclusionCulling,
	material_prepasses: MaterialPrepasses,
	depth_pyramid: DepthPyramidPass,
	contact_shadows: ContactShadowPass,
	gtao: GtaoPass,
	ssgi: SsgiPass,
	reflections: ScreenSpaceReflections,
	material_evaluation: MaterialEvaluationPass,
}

impl VisibilityRenderPass {
	/// Creates every per-sink GPU resource and requests the fixed visibility pipelines.
	///
	/// `shadow_maps` are the maps every sink shares; this sink's material evaluation samples them. The
	/// material-evaluation descriptor set still needs the environment written by the pipeline manager; see
	/// [`Self::material_evaluation_descriptor_set`].
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		targets: SinkTargets,
		shadow_maps: &ShadowMaps,
		gtao_settings: GtaoSettings,
		ssgi_settings: SsgiSettings,
		contact_shadow_settings: ContactShadowSettings,
	) -> Self {
		let visibility_descriptor_set = context.create_descriptor_set(Some("Visibility Descriptor Set"));
		let material_evaluation_descriptor_set = context.create_descriptor_set(Some("Material Evaluation Descriptor Set"));
		let material_buffers = MaterialBuffers::new(context);
		let shadow_map_images = shadow_maps.images;
		let ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
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
		let depth_pyramid = DepthPyramidPass::new(context, &pipeline_manager, targets.depth);
		let occlusion = OcclusionCulling::new(context, &pipeline_manager, targets.depth);
		let ssgi = SsgiPass::new(
			context,
			&pipeline_manager,
			targets.depth,
			depth_pyramid.depth_pyramid(),
			depth_pyramid.view_data(),
			targets.ssgi,
			ssgi_settings,
		);
		let light_clusters = LightClusterPass::new(
			context,
			&pipeline_manager,
			lighting_buffer,
			material_evaluation_descriptor_set,
		);
		let reflections = ScreenSpaceReflections::new(
			context,
			material_evaluation_descriptor_set,
			depth_pyramid.depth_pyramid(),
			targets.radiance_history,
		);
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
			ghi::DescriptorWrite::image(
				material_evaluation_descriptor_set,
				DIFFUSE_RADIANCE_HISTORY_BINDING.slot(),
				targets.ssgi.diffuse_radiance_history,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHTING_DATA_BINDING.slot(),
				lighting_buffer.into(),
			),
			sampled(AO_MAP_BINDING, ao_map.into(), linear_sampler),
			sampled(INDIRECT_DIFFUSE_MAP_BINDING, targets.ssgi.indirect_diffuse, linear_sampler),
			// Point sampling keeps a shadow edge from bleeding one pixel onto the lit surface beside it.
			sampled(CONTACT_SHADOW_MAP_BINDING, targets.contact_shadows.filtered, depth_sampler),
			sampled(SHADOW_MAP_BINDING, shadow_map_images.directional, depth_sampler),
			sampled(
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING,
				shadow_map_images.directional_depth_pyramid,
				depth_pyramid_sampler,
			),
			sampled(CONE_SHADOW_MAP_BINDING, shadow_map_images.cone, depth_sampler),
			sampled(POINT_SHADOW_MAP_BINDING, shadow_map_images.point, depth_sampler),
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
			cascade_fit: CascadeFitPass::new(context, &pipeline_manager, base_descriptor_set, targets.depth),
			light_clusters,
			visibility: VisibilityPass::new(
				&pipeline_manager,
				[base_descriptor_set, occlusion.descriptor_set()],
				targets.primitive_index,
				targets.instance_id,
				targets.depth,
			),
			occlusion,
			material_prepasses: MaterialPrepasses::new(
				&pipeline_manager,
				base_descriptor_set,
				visibility_descriptor_set,
				material_buffers.count,
			),
			gtao: GtaoPass::new(
				context,
				&pipeline_manager,
				targets.depth,
				depth_pyramid.depth_pyramid(),
				depth_pyramid.view_data(),
				ao_map.into(),
				gtao_settings,
			),
			contact_shadows: ContactShadowPass::new(
				context,
				&pipeline_manager,
				targets.depth,
				targets.contact_shadows,
				contact_shadow_settings,
			),
			depth_pyramid,
			ssgi,
			reflections,
			material_evaluation: MaterialEvaluationPass::new(
				targets.lit,
				targets.ssgi.diffuse_radiance_history,
				targets.radiance_history,
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

	pub(crate) fn set_ssgi_settings(&mut self, settings: SsgiSettings) {
		self.ssgi.set_settings(settings);
	}

	pub(crate) fn set_contact_shadow_settings(&mut self, settings: ContactShadowSettings) {
		self.contact_shadows.set_settings(settings);
	}

	/// Returns the descriptor set that carries material-evaluation-only resources, including the environment.
	pub(crate) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation.descriptor_set
	}

	/// Prepares one opaque visibility layer, the scene `background`, and one nearest-surface transparent layer.
	///
	/// Returns `None` while any fixed pipeline is still compiling. Only the first sink passes `frame_work`, so it runs
	/// once per frame. `history` describes how this pass recorded the sink in the previous
	/// frame at the same extent, or is `None` when the previous frame's images do not hold this sink's data.
	pub(crate) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_work: Option<FrameWork<'a>>,
		dispatches: PhaseDispatches,
		render_info: &'a RenderInfo,
		shadow_work: ShadowWork,
		history: Option<SinkHistory>,
		exposure: f32,
		background: Option<&crate::rendering::render_pass::SceneBackground>,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<impl RenderPassFunction + use<'a>> {
		let pipeline_manager = &self.pipeline_manager;
		// The cascades were made for the camera of the sink that records the frame-wide work, so only its surfaces
		// can fit them.
		let shadow_work = match frame_work {
			Some(_) => shadow_work,
			None => ShadowWork {
				receiver_fit: None,
				..shadow_work
			},
		};
		let (skinning, shadows) = match frame_work {
			Some(work) => (
				Some((work.skinning, pipeline_manager.pipeline(work.skinning.pipeline())?)),
				Some(work.shadow_maps.prepare(
					frame,
					pipeline_manager,
					dispatches,
					shadow_work,
					self.occlusion.descriptor_set(),
				)?),
			),
			None => (None, None),
		};
		let visibility_pipelines = self.visibility.pipelines(pipeline_manager)?;
		let prepass_pipelines = self.material_prepasses.pipelines(pipeline_manager)?;
		let cascade_fit = self.cascade_fit.prepare(frame, pipeline_manager, shadow_work, sink)?;
		let fits_receivers = shadow_work.receiver_fit.is_some();
		let light_cluster_pipeline = self.light_clusters.pipeline(pipeline_manager)?;
		let depth_pyramid_pipeline = self.depth_pyramid.pipeline(pipeline_manager)?;
		let occlusion_pipelines = self.occlusion.pipelines(pipeline_manager)?;
		let contact_shadow_pipelines = self.contact_shadows.pipelines(pipeline_manager)?;
		// A disabled pass neither records nor holds the frame back while its pipelines compile.
		let gtao_pipelines = match self.gtao.enabled() {
			true => Some(self.gtao.pipelines(pipeline_manager)?),
			false => None,
		};
		let ssgi_pipelines = match self.ssgi.enabled() {
			true => Some(self.ssgi.pipelines(pipeline_manager)?),
			false => None,
		};
		let light_clusters = self.light_clusters.prepare(frame, sink, light_cluster_pipeline);
		let occlusion_pyramid = self.occlusion.prepare(occlusion_pipelines);
		let depth_pyramid = self.depth_pyramid.prepare(frame, sink, depth_pyramid_pipeline);
		let contact_shadows = self
			.contact_shadows
			.prepare(frame, sink, shadow_work.directional, contact_shadow_pipelines);
		let gtao = gtao_pipelines.map(|pipelines| self.gtao.prepare(frame, sink, pipelines));
		// SSGI history exists only if the pass also ran last frame.
		let ssgi_history = history.filter(|history| history.ssgi);
		let ssgi = ssgi_pipelines.map(|pipelines| self.ssgi.prepare(frame, sink, ssgi_history, exposure, pipelines));
		self.reflections.prepare(frame, history);
		let screen_space_lighting = ScreenSpaceLighting {
			gtao: gtao.is_some(),
			ssgi: ssgi.is_some(),
		};
		let opaque_materials = self.material_evaluation.prepare(
			&render_info.opaque_materials,
			&render_info.opaque_material_mask,
			VisibilityPhase::Opaque,
			screen_space_lighting,
		);
		let transparent_materials = self.material_evaluation.prepare(
			&render_info.transparent_materials,
			&render_info.transparent_material_mask,
			VisibilityPhase::Transparent,
			screen_space_lighting,
		);
		// Prepare the background last: it may record one-time work, such as building lookup tables, that would be
		// lost if this pass gave up on the frame after it. A background still compiling leaves the sky black for this
		// frame instead of holding the scene back.
		let background = background.and_then(|background| background.prepare(frame, sink, frame_allocator));
		let extent = sink.extent();
		let visibility = &self.visibility;
		let material_prepasses = &self.material_prepasses;

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::CommonCommandBufferMode as _;

			c.start_region(|label| label.write_str("Visibility Render Model"));
			if let Some((pass, pipeline)) = skinning {
				pass.record(c, &render_info.skinning_dispatches, pipeline);
			}
			// Cascades fitted to the camera's surfaces are drawn once the opaque layer's depth exists.
			if !fits_receivers && let Some(shadows) = &shadows {
				shadows(c);
			}
			// Both material evaluation layers read the clusters, and nothing before them does.
			light_clusters(c);

			// The opaque layer establishes the depth and color retained by every later transparent primitive. Its early
			// pass draws what was unoccluded last frame, and the late pass draws what that depth does not hide.
			let opaque = |c: &mut ghi::implementation::CommandBufferRecording, occlusion| {
				visibility.record(c, extent, VisibilityPhase::Opaque, dispatches, visibility_pipelines, occlusion);
			};
			opaque(c, OcclusionPhase::Early);
			occlusion_pyramid(c);
			opaque(c, OcclusionPhase::Late);
			material_prepasses.record(c, extent, prepass_pipelines);
			cascade_fit(c);
			if fits_receivers && let Some(shadows) = &shadows {
				shadows(c);
			}
			// The screen-space passes don't read shadows, so the GPU can run them alongside the shadow maps.
			depth_pyramid(c);
			contact_shadows(c);
			if let Some(gtao) = &gtao {
				gtao(c);
			}
			if let Some(ssgi) = &ssgi {
				ssgi(c);
			}
			opaque_materials(c);
			// The background fills pixels no opaque surface covered, so transparent surfaces composite over it.
			if let Some(background) = background {
				background(c);
			}

			// The visibility buffer holds one transparent layer. Resolving every blend primitive together lets
			// normal depth testing select the nearest surface before source-over evaluation.
			if !dispatches.transparent.is_empty() {
				visibility.record(
					c,
					extent,
					VisibilityPhase::Transparent,
					dispatches,
					visibility_pipelines,
					OcclusionPhase::Test,
				);
				material_prepasses.record(c, extent, prepass_pipelines);
				transparent_materials(c);
			}
			c.end_region();
		})
	}
}
