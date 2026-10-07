//! Per-sink GPU work: shadows, light clusters, visibility rasterization, material prepasses, the cascade fit, the
//! occlusion and linear depth pyramids, sun visibility, GTAO, SSGI, and material evaluation, which also traces
//! screen-space reflections. GTAO and SSGI each run only while their settings enable them.
//!
//! One [`VisibilityRenderPass`] exists per sink. It owns the sink's images, buffers, and descriptor sets, and
//! [`VisibilityRenderPass::prepare`] turns the frame's [`RenderInfo`] into one ordered recording, timing each stage
//! with the sink's [`StageCounters`] so the inspector reports where the pass's GPU time goes.

mod depth_pyramid;
mod gtao;
mod light_clusters;
mod materials;
mod occlusion;
mod reflections;
mod shadows;
mod ssgi;
mod sun_visibility;
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

/// The `Pipelines` struct holds the fixed pipelines one visibility pass draws or dispatches with, so every pass
/// requests them at creation and resolves them each frame the same way.
pub(super) struct Pipelines<const N: usize>([crate::rendering::PipelineRef; N]);

impl<const N: usize> Pipelines<N> {
	/// Requests the pipeline asset `byte-engine/rendering/visibility/{name}.pipeline` for each name. Next, call
	/// [`Self::resolve`] each frame.
	pub(super) fn request(pipeline_manager: &PipelineManagerClient, names: [&str; N]) -> Self {
		Self(names.map(|name| pipeline_manager.request_pipeline(&format!("byte-engine/rendering/visibility/{name}.pipeline"))))
	}

	/// Returns the compiled pipelines in request order, or `None` while any is still compiling.
	pub(super) fn resolve(&self, pipeline_manager: &PipelineManagerClient) -> Option<[ghi::PipelineHandle; N]> {
		let pipelines = self.0.map(|pipeline| pipeline_manager.pipeline(pipeline));
		pipelines.iter().all(Option::is_some).then(|| pipelines.map(Option::unwrap))
	}

	/// Returns whether any pipeline failed to load or compile, so [`Self::resolve`] cannot succeed until it is rebuilt.
	pub(super) fn failed(&self, pipeline_manager: &PipelineManagerClient) -> bool {
		self.0
			.iter()
			.any(|pipeline| matches!(pipeline_manager.get(*pipeline), crate::rendering::PipelineState::Failed))
	}
}

impl Pipelines<4> {
	/// Requests one raster pipeline per opaque-layer work range: the solid, masked, double-sided, and double-sided masked
	/// variants of `stem`, such as `double-sided-masked-{stem}`. The visibility and shadow passes pair them with
	/// [`PhaseDispatches::opaque_layer`] the same way.
	pub(super) fn phases(pipeline_manager: &PipelineManagerClient, stem: &str) -> Self {
		Self(["", "masked-", "double-sided-", "double-sided-masked-"].map(|variant| {
			pipeline_manager.request_pipeline(&format!("byte-engine/rendering/visibility/{variant}{stem}.pipeline"))
		}))
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
					dispatch.work_item_base,
					(view_base + first_layer) as u32,
					first_layer as u32,
					batch_views as u32,
					occlusion as u32,
				],
			);
			c.dispatch_meshes(dispatch.workgroup_count, 1, 1);
		}
	}
}

use self::depth_pyramid::DepthPyramidPass;
pub(crate) use self::depth_pyramid::ScreenViewData;
pub use self::gtao::GTAO_CONFIGURATION_PREFIX;
pub(super) use self::gtao::GTAO_PIPELINES;
use self::gtao::GtaoPass;
pub(crate) use self::gtao::GtaoSettings;
use self::light_clusters::LightClusterPass;
use self::materials::{MaterialEvaluationPass, MaterialPrepasses, ScreenSpaceLighting};
use self::occlusion::OcclusionCulling;
pub(crate) use self::occlusion::OcclusionPhase;
use self::reflections::ScreenSpaceReflections;
pub(crate) use self::reflections::create_radiance_history_target;
use self::shadows::CascadeFitPass;
pub(crate) use self::shadows::{DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT, ShadowMaps, ShadowWork};
pub use self::ssgi::SSGI_CONFIGURATION_PREFIX;
use self::ssgi::SsgiPass;
pub(crate) use self::ssgi::{SsgiSettings, SsgiTargets, create_ssgi_targets};
pub use self::sun_visibility::CONTACT_SHADOWS_CONFIGURATION_PREFIX;
use self::sun_visibility::SunVisibilityPass;
pub(crate) use self::sun_visibility::{ContactShadowSettings, SunVisibilityTargets, create_sun_visibility_targets};
use self::visibility::{VisibilityPass, VisibilityPhase};
#[cfg(test)]
pub(crate) use self::{
	depth_pyramid::screen_view_data,
	occlusion::{OCCLUSION_PYRAMID_HEIGHT, OCCLUSION_PYRAMID_MIP_COUNT, OCCLUSION_PYRAMID_WIDTH},
	shadows::{ReceiverFitShaderData, receiver_fit_shader_data},
};
use super::features::VisibilityFeatures;
use super::layout::{
	AO_MAP_BINDING, CONE_SHADOW_MAP_BINDING, DIFFUSE_RADIANCE_HISTORY_BINDING, DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING,
	INSTANCE_ID_BINDING, LIGHTING_DATA_BINDING, LIT_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING,
	MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_MATERIALS, MAX_PIXEL_MAPPING_ENTRIES,
	MAX_TASK_VIEWS, POINT_SHADOW_MAP_BINDING, SHADOW_MAP_BINDING, SSGI_HISTORY_BINDING, SSGI_NORMALS_BINDING,
	SSGI_VIEW_BINDING, SUN_VISIBILITY_BINDING, TRIANGLE_INDEX_BINDING,
};
use super::mesh_dispatch::{MeshDispatch, PhaseDispatches};
use super::scene::RenderInfo;
use super::shader_data::LightingData;
use super::skinning::SkinningPass;
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassFunction};
use crate::rendering::{PipelineManagerClient, Sink, View};

/// The `StageCounters` struct holds the GPU timing counters one sink's visibility pass records around each of its
/// stages, so the inspector reports `stage.<name>` next to the pass's `scene.VisibilityPipelineManager` time.
///
/// Create it with [`StageCounters::new`] while building the sink and hand it to [`VisibilityRenderPass::new`]. A
/// stage records its counter only on frames it records commands, so a stage that runs nothing reports no time.
#[derive(Clone, Copy)]
pub(crate) struct StageCounters {
	skinning: ghi::CounterHandle,
	/// The directional cascades and every cone layer and point cube face drawn this frame.
	shadow_maps: ghi::CounterHandle,
	/// The max-depth pyramid built from the directional cascades.
	shadow_depth_pyramid: ghi::CounterHandle,
	light_clusters: ghi::CounterHandle,
	visibility_early: ghi::CounterHandle,
	occlusion_pyramid: ghi::CounterHandle,
	visibility_late: ghi::CounterHandle,
	/// The opaque layer's material count, offset, and pixel mapping.
	material_prepasses: ghi::CounterHandle,
	cascade_fit: ghi::CounterHandle,
	depth_pyramid: ghi::CounterHandle,
	/// The sun visibility, GTAO, and SSGI dispatches, recorded interleaved so the GPU can overlap them. One counter
	/// covers them all, because a precise timestamp between stages would serialize them again.
	screen_space: ghi::CounterHandle,
	/// Opaque material evaluation.
	material_evaluation: ghi::CounterHandle,
	background: ghi::CounterHandle,
	/// The transparent layer's visibility draw, material prepasses, and material evaluation.
	transparent: ghi::CounterHandle,
}

impl StageCounters {
	/// Creates one counter per stage, named as the inspector reports it.
	pub(crate) fn new(builder: &mut RenderPassBuilder<'_>) -> Self {
		Self {
			skinning: builder.create_gpu_counter("skinning"),
			shadow_maps: builder.create_gpu_counter("shadow-maps"),
			shadow_depth_pyramid: builder.create_gpu_counter("shadow-depth-pyramid"),
			light_clusters: builder.create_gpu_counter("light-clusters"),
			visibility_early: builder.create_gpu_counter("visibility-early"),
			occlusion_pyramid: builder.create_gpu_counter("occlusion-pyramid"),
			visibility_late: builder.create_gpu_counter("visibility-late"),
			material_prepasses: builder.create_gpu_counter("material-prepasses"),
			cascade_fit: builder.create_gpu_counter("cascade-fit"),
			depth_pyramid: builder.create_gpu_counter("depth-pyramid"),
			screen_space: builder.create_gpu_counter("screen-space"),
			material_evaluation: builder.create_gpu_counter("material-evaluation"),
			background: builder.create_gpu_counter("background"),
			transparent: builder.create_gpu_counter("transparent"),
		}
	}
}

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
	pub(crate) sun_visibility: SunVisibilityTargets,
	/// The light opaque material evaluation writes for next frame's reflection rays.
	pub(crate) radiance_history: ghi::DynamicImageHandle,
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
	sun_visibility: SunVisibilityPass,
	/// Absent when the project left GTAO out of its shaders.
	gtao: Option<GtaoPass>,
	ssgi: SsgiPass,
	reflections: ScreenSpaceReflections,
	material_evaluation: MaterialEvaluationPass,
	stage_counters: StageCounters,
}

impl VisibilityRenderPass {
	/// Creates every per-sink GPU resource and requests the fixed visibility pipelines.
	///
	/// `shadow_maps` are the maps every sink shares; this sink's material evaluation samples them. `stage_counters`
	/// are this sink's, from [`StageCounters::new`]. The material-evaluation descriptor set still needs the
	/// environment written by the pipeline manager; see [`Self::material_evaluation_descriptor_set`]. `features` are
	/// the ones the project's material shaders were baked with; a left-out feature creates no pass and requests no
	/// pipelines.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		targets: SinkTargets,
		shadow_maps: &ShadowMaps,
		stage_counters: StageCounters,
		features: VisibilityFeatures,
	) -> Self {
		let visibility_descriptor_set = context.create_descriptor_set(Some("Visibility Descriptor Set"));
		let material_evaluation_descriptor_set = context.create_descriptor_set(Some("Material Evaluation Descriptor Set"));
		// The material prepasses write these buffers, and material evaluation reads them.
		let material_buffer = |name, extra_uses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | extra_uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let material_count = context.build_buffer(material_buffer("Material Count", ghi::Uses::empty()));
		let material_offset: ghi::BufferHandle<[u32; MAX_MATERIALS]> =
			context.build_buffer(material_buffer("Material Offset", ghi::Uses::empty()));
		let material_offset_scratch: ghi::BufferHandle<[u32; MAX_MATERIALS]> =
			context.build_buffer(material_buffer("Material Offset Scratch", ghi::Uses::empty()));
		let evaluation_dispatches =
			context.build_buffer(material_buffer("Material Evaluation Dispatches", ghi::Uses::Indirect));
		let pixel_mapping: ghi::BufferHandle<[[u16; 2]; MAX_PIXEL_MAPPING_ENTRIES]> =
			context.build_buffer(material_buffer("Material XY", ghi::Uses::empty()));
		let ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let linear_sampler = context.build_sampler(ghi::sampler::Builder::new());
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {}),
		);
		let depth_pyramid_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.reduction_mode(ghi::SamplingReductionModes::Max)
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
		let cascade_fit = CascadeFitPass::new(context, &pipeline_manager, base_descriptor_set);
		let depth_pyramid = DepthPyramidPass::new(
			context,
			&pipeline_manager,
			targets.depth,
			cascade_fit.receiver_bounds(),
			cascade_fit.receiver_fit_parameters(),
		);
		let occlusion = OcclusionCulling::new(context, &pipeline_manager, targets.depth);
		let ssgi = SsgiPass::new(
			context,
			&pipeline_manager,
			depth_pyramid.depth_pyramid,
			depth_pyramid.view_data,
			targets.ssgi,
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
			depth_pyramid.depth_pyramid,
			targets.radiance_history,
		);
		let visibility_buffer = |binding: ghi::ShaderResourceDescriptor, buffer: ghi::BaseBufferHandle| {
			ghi::DescriptorWrite::buffer(visibility_descriptor_set, binding.slot(), buffer)
		};
		let storage = |set, binding: ghi::ShaderResourceDescriptor, image: ghi::BaseImageHandle| {
			ghi::DescriptorWrite::image(set, binding.slot(), image, ghi::Layouts::General)
		};
		context.write(&[
			storage(material_evaluation_descriptor_set, LIT_BINDING, targets.lit),
			storage(
				material_evaluation_descriptor_set,
				DIFFUSE_RADIANCE_HISTORY_BINDING,
				targets.ssgi.diffuse_radiance_history.into(),
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHTING_DATA_BINDING.slot(),
				lighting_buffer.into(),
			),
			sampled(AO_MAP_BINDING, ao_map.into(), linear_sampler),
			// Material evaluation upsamples the half-resolution SSGI result itself, so it point-samples its images.
			sampled(SSGI_HISTORY_BINDING, targets.ssgi.history.into(), depth_sampler),
			sampled(SSGI_NORMALS_BINDING, targets.ssgi.normals.into(), depth_sampler),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				SSGI_VIEW_BINDING.slot(),
				depth_pyramid.view_data.into(),
			),
			sampled(SUN_VISIBILITY_BINDING, targets.sun_visibility.visibility, depth_sampler),
			sampled(SHADOW_MAP_BINDING, shadow_maps.directional, depth_sampler),
			sampled(
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING,
				shadow_maps.directional_depth_pyramid,
				depth_pyramid_sampler,
			),
			sampled(CONE_SHADOW_MAP_BINDING, shadow_maps.cone, depth_sampler),
			sampled(POINT_SHADOW_MAP_BINDING, shadow_maps.point, depth_sampler),
			visibility_buffer(MATERIAL_COUNT_BINDING, material_count.into()),
			visibility_buffer(MATERIAL_OFFSET_BINDING, material_offset.into()),
			visibility_buffer(MATERIAL_OFFSET_SCRATCH_BINDING, material_offset_scratch.into()),
			visibility_buffer(MATERIAL_EVALUATION_DISPATCHES_BINDING, evaluation_dispatches.into()),
			visibility_buffer(MATERIAL_XY_BINDING, pixel_mapping.into()),
			storage(visibility_descriptor_set, TRIANGLE_INDEX_BINDING, targets.primitive_index),
			storage(visibility_descriptor_set, INSTANCE_ID_BINDING, targets.instance_id),
		]);

		Self {
			cascade_fit,
			light_clusters,
			visibility: VisibilityPass {
				descriptor_sets: [base_descriptor_set, occlusion.descriptor_set],
				pipelines: Pipelines::phases(&pipeline_manager, "visibility"),
				primitive_index: targets.primitive_index,
				instance_id: targets.instance_id,
				depth: targets.depth,
			},
			occlusion,
			material_prepasses: MaterialPrepasses {
				descriptor_sets: [
					base_descriptor_set,
					visibility_descriptor_set,
					material_evaluation_descriptor_set,
				],
				count_buffer: material_count,
				pipelines: Pipelines::request(&pipeline_manager, ["material-count", "material-offset", "pixel-mapping"]),
			},
			gtao: features.gtao.then(|| {
				GtaoPass::new(
					context,
					&pipeline_manager,
					targets.depth,
					depth_pyramid.depth_pyramid,
					depth_pyramid.view_data,
					ao_map.into(),
				)
			}),
			sun_visibility: SunVisibilityPass::new(
				context,
				&pipeline_manager,
				base_descriptor_set,
				targets.depth,
				depth_pyramid.depth_pyramid,
				depth_pyramid.view_data,
				shadow_maps,
				targets.sun_visibility,
			),
			depth_pyramid,
			ssgi,
			reflections,
			material_evaluation: MaterialEvaluationPass {
				base_descriptor_set,
				visibility_descriptor_set,
				descriptor_set: material_evaluation_descriptor_set,
				evaluation_dispatches,
			},
			stage_counters,
			pipeline_manager,
		}
	}

	/// Returns the descriptor set that carries material-evaluation-only resources, including the environment.
	pub(crate) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation.descriptor_set
	}

	/// Prepares one opaque visibility layer, the scene `background`, and one nearest-surface transparent layer.
	///
	/// Returns `None` while any fixed pipeline is still compiling. `frame_work` is the work one sink records for the
	/// whole frame: deforming skinned meshes and drawing the shadow maps every sink samples. The visibility pipeline
	/// manager passes it only to the first sink, whose camera the shadow views were made for, so that sink also fits the
	/// cascades to its surfaces. `history` describes how this pass recorded the sink in the previous frame at the same
	/// extent, or is `None` when the previous frame's images do not hold this sink's data. `exposure` is the shared
	/// lighting exposure uploaded for every sink in this frame. `gtao_settings`, `ssgi_settings`, and
	/// `contact_shadow_settings` are the runtime controls the visibility pipeline manager holds.
	pub(crate) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_work: Option<(&'a SkinningPass, &'a ShadowMaps)>,
		dispatches: PhaseDispatches,
		render_info: &'a RenderInfo,
		shadow_work: ShadowWork,
		history: Option<SinkHistory>,
		exposure: f32,
		gtao_settings: GtaoSettings,
		ssgi_settings: SsgiSettings,
		contact_shadow_settings: ContactShadowSettings,
		background: Option<&crate::rendering::render_pass::SceneBackground>,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<impl RenderPassFunction + use<'a>> {
		let pipeline_manager = &self.pipeline_manager;
		// The cascades were made for the camera of the sink that records the frame-wide work, so only its surfaces
		// can fit them.
		let shadow_work = ShadowWork {
			receiver_fit: shadow_work.receiver_fit.filter(|_| frame_work.is_some()),
			..shadow_work
		};
		let (skinning, shadows) = match frame_work {
			Some((skinning, shadow_maps)) => (
				Some((skinning, pipeline_manager.pipeline(skinning.pipeline)?)),
				Some(shadow_maps.prepare(
					frame,
					pipeline_manager,
					dispatches,
					shadow_work,
					self.visibility.descriptor_sets,
					self.stage_counters,
				)?),
			),
			None => (None, None),
		};
		let visibility_pipelines = self.visibility.pipelines.resolve(pipeline_manager)?;
		let prepass_pipelines = self.material_prepasses.pipelines.resolve(pipeline_manager)?;
		let cascade_fit = self.cascade_fit.prepare(frame, pipeline_manager, shadow_work, sink)?;
		let fits_receivers = shadow_work.receiver_fit.is_some();
		let [light_cluster_pipeline] = self.light_clusters.pipelines.resolve(pipeline_manager)?;
		let [depth_pyramid_pipeline] = self.depth_pyramid.pipelines.resolve(pipeline_manager)?;
		let occlusion_pyramid = self.occlusion.prepare(pipeline_manager)?;
		let sun_visibility_pipelines = self.sun_visibility.pipelines.resolve(pipeline_manager)?;
		// A disabled pass neither records nor holds the frame back while its pipelines compile. GTAO whose pipelines
		// failed, such as a release whose `config.json` did not match the bake, shades without it instead of stalling.
		let gtao = self
			.gtao
			.as_ref()
			.filter(|gtao| gtao_settings.enabled && !gtao.pipelines.failed(pipeline_manager));
		let gtao_pipelines = match gtao {
			Some(gtao) => Some((gtao, gtao.pipelines.resolve(pipeline_manager)?)),
			None => None,
		};
		let ssgi_pipelines = match ssgi_settings.enabled {
			true => Some(self.ssgi.pipelines.resolve(pipeline_manager)?),
			false => None,
		};
		let light_clusters = self.light_clusters.prepare(frame, sink, light_cluster_pipeline);
		let depth_pyramid = self
			.depth_pyramid
			.prepare(frame, sink, depth_pyramid_pipeline, fits_receivers);
		let sun_visibility = self.sun_visibility.prepare(
			frame,
			sink,
			shadow_work.directional,
			shadow_work.sun_angular_radius_tangent,
			contact_shadow_settings,
			sun_visibility_pipelines,
		);
		let gtao = gtao_pipelines.map(|(gtao, pipelines)| gtao.prepare(frame, sink, gtao_settings, pipelines));
		// SSGI history exists only if the pass also ran last frame.
		let ssgi_history = history.filter(|history| history.ssgi);
		let ssgi = ssgi_pipelines.map(|pipelines| self.ssgi.prepare(frame, sink, ssgi_history, exposure, pipelines));
		self.reflections.prepare(frame, history);
		let screen_space_lighting = ScreenSpaceLighting {
			gtao: gtao.is_some(),
			ssgi: ssgi.is_some(),
		};
		let opaque_materials = self.material_evaluation.prepare(
			&render_info.opaque_evaluations,
			&render_info.opaque_evaluation_mask,
			VisibilityPhase::Opaque,
			screen_space_lighting,
		);
		let transparent_materials = self.material_evaluation.prepare(
			&render_info.transparent_evaluations,
			&render_info.transparent_evaluation_mask,
			VisibilityPhase::Transparent,
			screen_space_lighting,
		);
		// Prepare the background last: it may record one-time work, such as building lookup tables, that would be
		// lost if this pass gave up on the frame after it. A background still compiling leaves the sky black for this
		// frame instead of holding the scene back.
		let background = background.and_then(|background| background.prepare(frame, sink, frame_allocator));
		let extent = sink.extent();
		// Each stage records its counter only when it records commands, so an idle stage reports no time.
		let counters = self.stage_counters;
		let skinning = skinning.filter(|_| !render_info.skinning_dispatches.is_empty());

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::CommonCommandBufferMode as _;

			c.start_region(|label| label.write_str("Visibility Render Model"));
			if let Some((pass, pipeline)) = skinning {
				c.counter(counters.skinning, |c| {
					pass.record(c, &render_info.skinning_dispatches, pipeline)
				});
			}
			// Cascades fitted to the camera's surfaces are drawn once the opaque layer's depth exists. The shadow
			// recording times its maps and pyramid with this sink's counters itself.
			if !fits_receivers && let Some(shadows) = &shadows {
				shadows(c);
			}
			// Both material evaluation layers read the clusters, and nothing before them does.
			c.counter(counters.light_clusters, &light_clusters);

			// The opaque layer establishes the depth and color retained by every later transparent primitive. Its early
			// pass draws what was unoccluded last frame, and the late pass draws what that depth does not hide.
			let draw = |c: &mut ghi::implementation::CommandBufferRecording, phase, occlusion| {
				self.visibility
					.record(c, extent, phase, dispatches, visibility_pipelines, occlusion);
			};
			c.counter(counters.visibility_early, |c| {
				draw(c, VisibilityPhase::Opaque, OcclusionPhase::Early)
			});
			c.counter(counters.occlusion_pyramid, &occlusion_pyramid);
			c.counter(counters.visibility_late, |c| {
				draw(c, VisibilityPhase::Opaque, OcclusionPhase::Late)
			});
			c.counter(counters.material_prepasses, |c| {
				self.material_prepasses
					.record(c, extent, prepass_pipelines, VisibilityPhase::Opaque)
			});
			// The depth pyramid also finds the directional shadow receiver bounds the cascade fit shrinks to.
			c.counter(counters.depth_pyramid, &depth_pyramid);
			if fits_receivers {
				c.counter(counters.cascade_fit, &cascade_fit);
				if let Some(shadows) = &shadows {
					shadows(c);
				}
			}
			// The screen-space passes read the depth pyramid and write their own images, so their first stages are
			// independent of each other and so are their second stages, which read only the first. Recording the
			// stages by rank lets the GPU overlap them: the resource tracker barriers the first second stage against
			// every first stage, and the others need nothing more. The sun visibility resolve also reads the shadow
			// maps and their pyramid, which both orders above record before it.
			c.counter(counters.screen_space, |c| {
				if let Some(sun_visibility) = &sun_visibility {
					record_compute_stages(c, None, &[sun_visibility.trace]);
				}
				if let Some(gtao) = &gtao {
					record_compute_stages(c, None, &[gtao.evaluate]);
				}
				if let Some(ssgi) = &ssgi {
					record_compute_stages(c, None, &[ssgi.trace]);
				}
				if let Some(sun_visibility) = &sun_visibility {
					record_compute_stages(c, None, &[sun_visibility.resolve]);
				}
				if let Some(gtao) = &gtao {
					record_compute_stages(c, None, &[gtao.blur]);
				}
				if let Some(ssgi) = &ssgi {
					record_compute_stages(c, None, &[ssgi.temporal]);
				}
				if let Some(gtao) = &gtao {
					record_compute_stages(c, None, &[gtao.upscale]);
				}
			});
			c.counter(counters.material_evaluation, &opaque_materials);
			// The background fills pixels no opaque surface covered, so transparent surfaces composite over it.
			if let Some(background) = background {
				c.counter(counters.background, background);
			}

			// The visibility buffer holds one transparent layer. Resolving every blend primitive together lets
			// normal depth testing select the nearest surface before source-over evaluation.
			if !dispatches.transparent.is_empty() {
				c.counter(counters.transparent, |c| {
					draw(c, VisibilityPhase::Transparent, OcclusionPhase::Test);
					self.material_prepasses
						.record(c, extent, prepass_pipelines, VisibilityPhase::Transparent);
					transparent_materials(c);
				});
			}
			c.end_region();
		})
	}
}
