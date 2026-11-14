//! Per-sink GPU work: shadows, light clusters, visibility rasterization, material prepasses, the cascade fit, the
//! occlusion and linear depth pyramids, sun visibility, GTAO, SSGI or the surfel or probe resolve, material
//! evaluation, which also traces screen-space reflections, and the light of the height fog. GTAO runs only while its settings enable it, and SSGI, surfel
//! GI, and probe GI only in frames they light; see [`IndirectDiffuseSource`].
//!
//! One [`VisibilityRenderPass`] exists per sink. It owns the sink's images, buffers, and descriptor sets, and
//! [`VisibilityRenderPass::prepare`] turns the frame's [`RenderInfo`] into one ordered recording, timing each stage
//! with the sink's [`StageCounters`] so the inspector reports where the pass's GPU time goes.

mod backdrop;
mod depth_pyramid;
mod fog_light;
mod gtao;
mod light_clusters;
mod materials;
mod occlusion;
mod probe_gi;
mod ray_hits;
mod reflections;
mod shadows;
mod ssgi;
mod subsurface;
mod sun_visibility;
mod surfel_gi;
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
	/// Requests the pipeline asset `byte_engine/rendering/visibility/{name}.pipeline` for each name. Next, call
	/// [`Self::resolve`] each frame.
	pub(super) fn request(pipeline_manager: &PipelineManagerClient, names: [&str; N]) -> Self {
		Self(names.map(|name| pipeline_manager.request_pipeline(&format!("byte_engine/rendering/visibility/{name}.pipeline"))))
	}

	/// Returns the compiled pipelines in request order, or `None` while any is still compiling.
	pub(super) fn resolve(&self, pipeline_manager: &PipelineManagerClient) -> Option<[ghi::PipelineHandle; N]> {
		let pipelines = self.0.map(|pipeline| pipeline_manager.pipeline(pipeline));
		pipelines.iter().all(Option::is_some).then(|| pipelines.map(Option::unwrap))
	}
}

impl Pipelines<2> {
	/// Requests the solid raster pipelines of `stem`: the one-sided one, then the double-sided one,
	/// `double-sided-{stem}`. Pair them with the masked ones of the coverage shaders through [`opaque_layer_pipelines`].
	pub(super) fn sides(pipeline_manager: &PipelineManagerClient, stem: &str) -> Self {
		Self(["", "double-sided-"].map(|variant| {
			pipeline_manager.request_pipeline(&format!("byte_engine/rendering/visibility/{variant}{stem}.pipeline"))
		}))
	}
}

/// Returns the pipeline of each opaque-layer work range, in the order of [`PhaseDispatches::opaque_layer`]: solid,
/// masked, double-sided, and double-sided masked. `solid` and `masked` each hold a pass's one-sided pipeline, then its
/// double-sided one. The masked ones are `None` until the coverage shaders compile, and no masked instance is admitted
/// before.
pub(super) fn opaque_layer_pipelines(
	[solid, double_sided]: [ghi::PipelineHandle; 2],
	masked: Option<[ghi::PipelineHandle; 2]>,
) -> [Option<ghi::PipelineHandle>; 4] {
	[
		Some(solid),
		masked.map(|[masked, _]| masked),
		Some(double_sided),
		masked.map(|[_, masked]| masked),
	]
}

/// Records one mesh dispatch per non-empty work range and batch of up to [`MAX_TASK_VIEWS`] views. The batches draw
/// `view_count` packed views from `view_base` into consecutive layers from zero.
///
/// `descriptor_sets` are the base set and a sink's [`OcclusionCulling::descriptor_set`]. The shared task shader declares
/// the occlusion resources, so every pass binds them, but only `occlusion` other than [`OcclusionPhase::Disabled`] uses
/// them. `minimum_caster_extent` is the narrowest instance, in normalized device units across x, that orthographic views
/// draw; pass zero for perspective views and to draw every size. Call it inside a render pass whose attachments hold
/// `view_count` layers, or one unlayered target for one view.
pub(super) fn record_meshlet_dispatches(
	c: &mut impl ghi::command_buffer::RasterizationRenderPassMode,
	descriptor_sets: [ghi::DescriptorSetHandle; 2],
	occlusion: OcclusionPhase,
	ranges: impl IntoIterator<Item = (MeshDispatch, Option<ghi::PipelineHandle>)>,
	view_base: usize,
	view_count: usize,
	minimum_caster_extent: f32,
) {
	use ghi::command_buffer::{BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _};

	for (dispatch, pipeline) in ranges {
		if dispatch.is_empty() {
			continue;
		}
		let Some(pipeline) = pipeline else {
			unreachable!("A work range that holds instances has a pipeline: masked instances wait for the coverage shaders.");
		};
		let c = c.bind_raster_pipeline(pipeline);
		c.bind_descriptor_sets(&descriptor_sets);
		// The large instances' workgroups, then the ones small instances' meshlets share. The GPU counted both, so a
		// range without work dispatches no workgroups.
		for shared_lanes in [0, 1] {
			let range = dispatch.phase * 2 + shared_lanes;
			for first_layer in (0..view_count).step_by(MAX_TASK_VIEWS) {
				let batch_views = (view_count - first_layer).min(MAX_TASK_VIEWS);
				c.write_push_constant(
					0,
					[
						range,
						(view_base + first_layer) as u32,
						first_layer as u32,
						batch_views as u32,
						occlusion as u32,
						minimum_caster_extent.to_bits(),
						shared_lanes,
					],
				);
				c.dispatch_meshes_indirect(dispatch.records, range as usize);
			}
		}
	}
}

use self::backdrop::BackdropPass;
pub(crate) use self::backdrop::{BackdropTargets, create_backdrop_targets};
use self::depth_pyramid::DepthPyramidPass;
// Only the layout tests compare the record with the stride its layer reflects.
#[cfg(test)]
pub(crate) use self::depth_pyramid::ScreenViewData;
use self::fog_light::FogLightPass;
pub(crate) use self::fog_light::create_fog_light_target;
#[cfg(test)]
pub(crate) use self::fog_light::{FogLightShaderParameters, fog_light_shader_parameters};
pub(super) use self::gtao::GTAO_PIPELINES;
use self::gtao::{GtaoPass, GtaoTargets};
pub(crate) use self::gtao::{GtaoSettings, create_gtao_targets};
use self::light_clusters::LightClusterPass;
pub(super) use self::materials::{MATERIAL_EVALUATION_PUSH_CONSTANT_SIZE, MATERIAL_PREPASS_PIPELINES};
use self::materials::{MaterialEvaluationPass, MaterialPrepasses, ScreenSpaceLighting, evaluation_is_active};
use self::occlusion::OcclusionCulling;
pub(crate) use self::occlusion::OcclusionPhase;
pub(super) use self::probe_gi::PROBE_GI_PIPELINES;
use self::probe_gi::ProbeResolvePass;
pub(crate) use self::probe_gi::{
	DEFAULT_PROBE_RAY_BUDGET, DEFAULT_PROBE_SPACING, PROBE_RAY_BUDGET_RANGE, PROBE_RAYS, PROBE_SPACING_RANGE, ProbeFrame,
	ProbeGiSettings, ProbeVolume,
};
pub(super) use self::ray_hits::RAY_HIT_PIPELINES;
pub(crate) use self::ray_hits::{RayHits, RayTracing};
use self::reflections::ScreenSpaceReflections;
use self::shadows::CascadeFitPass;
pub(crate) use self::shadows::{ShadowMaps, ShadowWork, SunCascades};
pub(super) use self::ssgi::SSGI_PIPELINES;
use self::ssgi::SsgiPass;
pub(crate) use self::ssgi::{SsgiTargets, create_diffuse_radiance_history_target, create_ssgi_targets};
pub(super) use self::subsurface::SUBSURFACE_PIPELINES;
use self::subsurface::SubsurfacePass;
pub(crate) use self::subsurface::{SubsurfaceTargets, create_subsurface_targets};
pub(super) use self::sun_visibility::CONTACT_SHADOW_PIPELINES;
use self::sun_visibility::SunVisibilityPass;
pub(crate) use self::sun_visibility::{ContactShadowSettings, SunVisibilityTargets, create_sun_visibility_targets};
pub(super) use self::surfel_gi::SURFEL_GI_PIPELINES;
use self::surfel_gi::SurfelResolvePass;
pub(crate) use self::surfel_gi::{
	DEFAULT_SURFEL_CAPACITY, DEFAULT_SURFEL_RAY_BUDGET, MAX_SURFEL_RAY_BUDGET, SURFEL_CAPACITY_RANGE, SURFEL_RAY_BUDGET_STEP,
	SurfelFrame, SurfelGiSettings, SurfelLight, SurfelPool, SurfelTargets, create_surfel_targets,
};
use self::visibility::{VisibilityPass, VisibilityPhase};
#[cfg(test)]
pub(crate) use self::{
	depth_pyramid::screen_view_data,
	occlusion::{OCCLUSION_PYRAMID_HEIGHT, OCCLUSION_PYRAMID_MIP_COUNT, OCCLUSION_PYRAMID_WIDTH},
	shadows::{ReceiverFitShaderData, receiver_fit_shader_data},
};
use super::acceleration_structures::AccelerationStructureWork;
use super::features::{MaterialEvaluationMode, VisibilityFeatures};
use super::layout::MAX_TASK_VIEWS;
use super::mesh_dispatch::{MeshDispatch, MeshWork, PhaseDispatches};
use super::scene::RenderInfo;
use super::set_layouts;
use super::shader_data::LightingData;
use super::skinning::SkinningPass;
use crate::configuration::{Accepts, Setting};
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassFunction};
use crate::rendering::{PipelineManagerClient, ShownDebugViews, Sink, View};

/// The `FrameWork` struct is the work one sink records for the whole frame, before any sink samples its results.
///
/// The visibility pipeline manager hands it to the first sink it records; see [`VisibilityRenderPass::prepare`].
#[derive(Clone, Copy)]
pub(crate) struct FrameWork<'a> {
	/// Deforms the frame's skinned meshes.
	pub(crate) skinning: &'a SkinningPass,
	/// Builds the mesh work every pass that draws meshlets dispatches.
	pub(crate) mesh_work: &'a MeshWork,
	/// Draws the shadow maps every sink samples.
	pub(crate) shadows: &'a ShadowMaps,
	/// Builds the acceleration structures surfel GI or probe GI traces, in frames they light.
	pub(crate) acceleration_structures: Option<AccelerationStructureWork<'a>>,
	/// Brings the generated environment up to date before any ray reads it.
	pub(crate) environment: Option<&'a crate::rendering::render_pass::SceneEnvironment>,
}

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
	/// The bottom-level builds and the top-level build of the scene's acceleration structures.
	acceleration_structures: ghi::CounterHandle,
	/// The surfel economy, count, scan, and scatter that list the frame's surfels in the grid.
	surfel_grid: ghi::CounterHandle,
	/// The support rays that check a share of the surfels still lie on a surface.
	surfel_validate: ghi::CounterHandle,
	/// The surfel rays' trace, the hit sort, and the shade pass.
	surfel_trace: ghi::CounterHandle,
	/// The update of every live surfel's estimate and record, and the allocation of the rays' hits' spawns.
	surfel_integrate: ghi::CounterHandle,
	/// The probe schedule's ranking, cut, and list.
	probe_schedule: ghi::CounterHandle,
	/// The probe rays' trace, the hit sort, and the shade pass.
	probe_trace: ghi::CounterHandle,
	/// The blend of the listed probes' rays into their maps, and their relocation and classification.
	probe_update: ghi::CounterHandle,
	/// The sun visibility, GTAO, and SSGI, surfel, or probe resolve dispatches, recorded interleaved so the GPU can overlap
	/// them. This total is the screen passes' real cost; the per-dispatch counters below overlap one another.
	screen_space: ghi::CounterHandle,
	/// One counter per screen-space dispatch, in recording order. An end timestamp waits for every dispatch recorded
	/// before it, so a dispatch's time can include time other dispatches ran alongside it.
	screen_space_dispatches: ScreenSpaceCounters,
	/// The fog light tile pass and march.
	fog_light: ghi::CounterHandle,
	/// Opaque material evaluation.
	material_evaluation: ghi::CounterHandle,
	/// The spread of subsurface pixels' diffuse light.
	subsurface: ghi::CounterHandle,
	background: ghi::CounterHandle,
	/// The copy of the opaque scene that blended materials read behind their surfaces.
	backdrop: ghi::CounterHandle,
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
			acceleration_structures: builder.create_gpu_counter("acceleration-structures"),
			surfel_grid: builder.create_gpu_counter("surfel-grid"),
			surfel_validate: builder.create_gpu_counter("surfel-validate"),
			surfel_trace: builder.create_gpu_counter("surfel-trace"),
			surfel_integrate: builder.create_gpu_counter("surfel-integrate"),
			probe_schedule: builder.create_gpu_counter("probe-schedule"),
			probe_trace: builder.create_gpu_counter("probe-trace"),
			probe_update: builder.create_gpu_counter("probe-update"),
			screen_space: builder.create_gpu_counter("screen-space"),
			screen_space_dispatches: ScreenSpaceCounters {
				sun_visibility_trace: builder.create_gpu_counter("sun-visibility-trace"),
				gtao_evaluate: builder.create_gpu_counter("gtao-evaluate"),
				ssgi_trace: builder.create_gpu_counter("ssgi-trace"),
				surfel_resolve: builder.create_gpu_counter("surfel-resolve"),
				probe_resolve: builder.create_gpu_counter("probe-resolve"),
				sun_visibility_resolve: builder.create_gpu_counter("sun-visibility-resolve"),
				gtao_blur: builder.create_gpu_counter("gtao-blur"),
				ssgi_temporal: builder.create_gpu_counter("ssgi-temporal"),
				surfel_allocate: builder.create_gpu_counter("surfel-allocate"),
				surfel_spatial: builder.create_gpu_counter("surfel-spatial"),
				gtao_upscale: builder.create_gpu_counter("gtao-upscale"),
			},
			fog_light: builder.create_gpu_counter("fog-light"),
			material_evaluation: builder.create_gpu_counter("material-evaluation"),
			subsurface: builder.create_gpu_counter("subsurface"),
			background: builder.create_gpu_counter("background"),
			backdrop: builder.create_gpu_counter("backdrop"),
			transparent: builder.create_gpu_counter("transparent"),
		}
	}
}

/// The `ScreenSpaceCounters` struct holds one GPU timing counter per screen-space dispatch, so the inspector can
/// split [`StageCounters`]'s `screen-space` total by technique and step.
#[derive(Clone, Copy)]
struct ScreenSpaceCounters {
	sun_visibility_trace: ghi::CounterHandle,
	gtao_evaluate: ghi::CounterHandle,
	ssgi_trace: ghi::CounterHandle,
	surfel_resolve: ghi::CounterHandle,
	probe_resolve: ghi::CounterHandle,
	sun_visibility_resolve: ghi::CounterHandle,
	gtao_blur: ghi::CounterHandle,
	ssgi_temporal: ghi::CounterHandle,
	surfel_allocate: ghi::CounterHandle,
	surfel_spatial: ghi::CounterHandle,
	gtao_upscale: ghi::CounterHandle,
}

/// The `SinkTargets` struct names the render-graph images a sink gives the visibility pass.
#[derive(Clone, Copy)]
pub(crate) struct SinkTargets {
	/// The scene's light, one copy per frame in flight. Reflection rays read the previous frame's copy.
	pub(crate) lit: ghi::DynamicImageHandle,
	/// The scene's reverse-Z depth, one copy per frame in flight. Reflection rays read the previous frame's copy to find
	/// the pixel that saw their hit.
	pub(crate) depth: ghi::DynamicImageHandle,
	pub(crate) primitive_index: ghi::BaseImageHandle,
	pub(crate) instance_id: ghi::BaseImageHandle,
	/// The SSGI images. Surfel GI and probe GI write the same images in SSGI's place.
	pub(crate) ssgi: SsgiTargets,
	/// The diffuse light that opaque material evaluation writes for next frame's SSGI rays, present when the project
	/// builds SSGI in.
	pub(crate) diffuse_radiance_history: Option<ghi::DynamicImageHandle>,
	/// Surfel GI's own images, present when the frame has a [`SurfelPool`] that surfel GI can light this sink from.
	pub(crate) surfels: Option<SurfelTargets>,
	/// The sun's contact shadows, which opaque material evaluation multiplies into the sun's shadow.
	pub(crate) sun_visibility: SunVisibilityTargets,
	/// The ambient occlusion GTAO writes and opaque material evaluation reads.
	pub(crate) gtao: GtaoTargets,
	/// The half-resolution light of the height fog, which the height fog pass reads.
	pub(crate) fog_light: ghi::BaseImageHandle,
	/// The copy of the opaque scene that blended materials read behind their surfaces.
	pub(crate) backdrop: BackdropTargets,
	/// What opaque material evaluation leaves at subsurface pixels, present when the project builds subsurface
	/// scattering in.
	pub(crate) subsurface: Option<SubsurfaceTargets>,
}

/// The `IndirectDiffuseSource` enum names the pass that lights a frame's indirect diffuse light, so the frame runs only
/// that pass and the next frame knows which history it left.
///
/// Each writes the images opaque material evaluation reads indirect diffuse light from. The visibility pipeline manager
/// picks one each frame, or none, and the environment alone lights indirect diffuse light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IndirectDiffuseSource {
	/// SSGI traces the depth pyramid at half resolution. Material evaluation also writes the diffuse light its next
	/// frame's rays read.
	ScreenSpace,
	/// Surfel GI gathers ray-traced bounce light into surfels on the surfaces the camera sees.
	Surfels,
	/// Probe GI gathers ray-traced bounce light into cascades of irradiance probes around the camera.
	Probes,
	/// Lightmaps BELD baked from a level light the instances they cover; material evaluation reads them itself, so no
	/// pass runs for them.
	Baked,
}

impl IndirectDiffuseSource {
	/// Returns the value opaque material evaluation's `indirect_diffuse` push constant takes when this source lit the
	/// frame, which is never zero.
	///
	/// Material evaluation adds the environment's diffuse light itself under SSGI, whose light holds only what its rays
	/// hit, and not under surfel GI or probe GI, whose light holds the environment's light their rays reached. Under
	/// lightmaps, a pixel reads its instance's lightmap instead of the SSGI images.
	pub(crate) fn push_constant(self) -> u32 {
		match self {
			Self::ScreenSpace => 1,
			Self::Surfels | Self::Probes => 2,
			Self::Baked => 3,
		}
	}

	/// Every source, in the order of the `render.gi.technique` words in [`GiSettings::SETTINGS`].
	const ALL: [Self; 4] = [Self::ScreenSpace, Self::Surfels, Self::Probes, Self::Baked];

	/// Returns the `gi.technique` value that selects this source, and the startup parameter that builds it in.
	fn names(self) -> (&'static str, &'static str) {
		match self {
			Self::ScreenSpace => ("ssgi", super::features::SSGI_ENABLED_PARAMETER),
			Self::Surfels => ("surfels", super::features::SURFEL_GI_ENABLED_PARAMETER),
			Self::Probes => ("probes", super::features::PROBE_GI_ENABLED_PARAMETER),
			Self::Baked => ("baked", super::features::BAKED_GI_ENABLED_PARAMETER),
		}
	}

	/// Returns whether the source traces rays, so it lights frames only on a device that does.
	pub(crate) fn traces_rays(self) -> bool {
		matches!(self, Self::Surfels | Self::Probes)
	}

	/// Returns whether the project built this source into its shaders, so it can be selected.
	pub(crate) fn built_in(self, features: VisibilityFeatures) -> bool {
		match self {
			Self::ScreenSpace => features.ssgi,
			Self::Surfels => features.surfel_gi,
			Self::Probes => features.probe_gi,
			Self::Baked => features.baked_gi,
		}
	}
}

/// The `GiSettings` struct holds the runtime global illumination controls the visibility pipeline manager applies to
/// every frame: whether indirect diffuse light comes from a technique at all, and which one.
///
/// `ssgi.enabled`, `surfel-gi.enabled`, `probe-gi.enabled`, and `baked-gi.enabled` only decide, at startup, which techniques the shaders
/// carry; these settings pick among those. Next, pass them to the manager's indirect diffuse source choice each frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GiSettings {
	/// Whether a technique lights indirect diffuse light. Off, the environment lights it alone.
	pub(crate) enabled: bool,
	/// The technique that lights indirect diffuse light while `enabled`.
	pub(crate) technique: IndirectDiffuseSource,
	/// The techniques the project built into its shaders, the only ones `technique` can select.
	pub(crate) features: VisibilityFeatures,
}

impl GiSettings {
	/// Creates the controls of a project built with `features`: on, with SSGI when it is built in, then probe GI, then
	/// surfel GI, then lightmaps.
	pub(crate) fn new(features: VisibilityFeatures) -> Self {
		let technique = if features.ssgi {
			IndirectDiffuseSource::ScreenSpace
		} else if features.probe_gi {
			IndirectDiffuseSource::Probes
		} else if features.surfel_gi {
			IndirectDiffuseSource::Surfels
		} else if features.baked_gi {
			IndirectDiffuseSource::Baked
		} else {
			IndirectDiffuseSource::ScreenSpace
		};
		Self {
			enabled: true,
			technique,
			features,
		}
	}

	/// The runtime settings, named inside the `render.gi.` port. A technique the project left out of its shaders is
	/// not selected, and the log says so.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[
		Setting {
			name: "enabled",
			accepts: Accepts::Bool,
			apply: |settings, read| {
				let enabled = read.bool();
				Ok(Self { enabled, ..settings })
			},
		},
		Setting {
			name: "technique",
			// Listed in `IndirectDiffuseSource::ALL` order, which `apply` indexes.
			accepts: Accepts::Choice(&["ssgi", "surfels", "probes", "baked"]),
			apply: |settings, read| {
				let technique = IndirectDiffuseSource::ALL[read.choice()];
				let (name, feature) = technique.names();
				if !technique.built_in(settings.features) {
					let reason = format!(
						"GI technique `{name}` was not selected. The most likely cause is that `render.{feature}` was `false` at startup, so the technique was left out of the project's shaders. Set it to `true` in `config.json`, then rebake with `beld bake --force`. See {}.",
						crate::online_docs_url("use/resources/bake#leave-render-features-out-of-the-bake")
					);
					log::warn!("{reason}");
					return Err(reason);
				}
				Ok(Self { technique, ..settings })
			},
		},
	];
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
	/// The source that lit the recorded frame's indirect diffuse light, so only SSGI's own frames give SSGI history.
	pub(crate) indirect_diffuse: Option<IndirectDiffuseSource>,
}

impl SinkHistory {
	/// Returns what a temporal pass reprojects into this history with: the transform from `current`'s view space to the
	/// history camera's clip space, the one that rotates `current`'s view-space normals into the history camera's axes,
	/// and the ratio that brings the history's pre-exposed light to `exposure`.
	pub(crate) fn reprojection(&self, current: View, exposure: f32) -> (math::Matrix, math::Matrix, f32) {
		let previous_view = self.view.view() * math::inverse(current.view());
		(
			self.view.projection() * previous_view,
			previous_view,
			exposure / self.exposure,
		)
	}
}

/// The `VisibilityRenderPass` struct sequences visibility-buffer work for one sink and scene frame.
pub(crate) struct VisibilityRenderPass {
	pipeline_manager: PipelineManagerClient,
	cascade_fit: CascadeFitPass,
	light_clusters: LightClusterPass,
	visibility: VisibilityPass,
	occlusion: OcclusionCulling,
	/// Absent in the uber material evaluation mode, whose uber shaders find each pixel's material themselves.
	material_prepasses: Option<MaterialPrepasses>,
	depth_pyramid: DepthPyramidPass,
	sun_visibility: SunVisibilityPass,
	/// Absent when the project left GTAO out of its shaders.
	gtao: Option<GtaoPass>,
	/// Absent when the project left SSGI out of its shaders.
	ssgi: Option<SsgiPass>,
	/// Absent when the frame has no surfel pool to light this sink from.
	surfel_resolve: Option<SurfelResolvePass>,
	/// Absent when the frame has no probe volume to light this sink from.
	probe_resolve: Option<ProbeResolvePass>,
	/// Absent when the project left subsurface scattering out of its shaders.
	subsurface: Option<SubsurfacePass>,
	reflections: ScreenSpaceReflections,
	backdrop: BackdropPass,
	material_evaluation: MaterialEvaluationPass,
	fog_light: FogLightPass,
	stage_counters: StageCounters,
	/// Which material debug views to write this frame; see `MATERIAL_DEBUG_VIEWS`.
	debug_views: crate::rendering::DebugViews,
}

impl VisibilityRenderPass {
	/// Creates every per-sink GPU resource and requests the fixed visibility pipelines.
	///
	/// `shadow_maps` are the maps every sink shares; this sink's material evaluation samples them. `stage_counters`
	/// are this sink's, from [`StageCounters::new`]. The material-evaluation descriptor set still needs the
	/// environment written by the pipeline manager; see [`Self::material_evaluation_descriptor_set`]. `features` are
	/// the ones the project's material shaders were baked with; a left-out feature creates no pass and requests no
	/// pipelines. Surfel GI lights the sink when `targets` holds surfel images, and `probe_gi` tells whether the frame
	/// has a [`ProbeVolume`] that probe GI can light this sink from.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: PipelineManagerClient,
		base_descriptor_set: ghi::DescriptorSetHandle,
		views_buffer: ghi::BaseBufferHandle,
		lighting_buffer: ghi::DynamicBufferHandle<LightingData>,
		targets: SinkTargets,
		shadow_maps: &ShadowMaps,
		stage_counters: StageCounters,
		features: VisibilityFeatures,
		meshlet_groups: u32,
		probe_gi: bool,
		debug_views: crate::rendering::DebugViews,
	) -> Self {
		let visibility_buffer_descriptor_set =
			set_layouts::VISIBILITY_BUFFER.create_set(context, "Visibility Buffer Descriptor Set");
		let material_evaluation_descriptor_set =
			set_layouts::MATERIAL_EVALUATION.create_set(context, "Material Evaluation Descriptor Set");
		let material_prepasses = (features.material_evaluation == MaterialEvaluationMode::Pipelines).then(|| {
			MaterialPrepasses::new(
				context,
				&pipeline_manager,
				base_descriptor_set,
				visibility_buffer_descriptor_set,
				material_evaluation_descriptor_set,
				targets.instance_id,
			)
		});
		let linear_sampler = context.build_sampler(ghi::sampler::Builder::new());
		let depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {}),
		);
		// The passes of this frame bind its own copy of the depth; only reflections read the previous one.
		let depth: ghi::BaseImageHandle = targets.depth.into();
		let cascade_fit = CascadeFitPass::new(context, &pipeline_manager, views_buffer);
		let depth_pyramid = DepthPyramidPass::new(
			context,
			&pipeline_manager,
			depth,
			cascade_fit.receiver_bounds(),
			cascade_fit.receiver_fit_parameters(),
		);
		let occlusion = OcclusionCulling::new(context, &pipeline_manager, depth, meshlet_groups);
		// The diffuse radiance history exists exactly when the project builds SSGI in.
		let ssgi = targets.diffuse_radiance_history.map(|diffuse_radiance_history| {
			SsgiPass::new(
				context,
				&pipeline_manager,
				depth_pyramid.depth_pyramid,
				depth_pyramid.view_data,
				targets.ssgi,
				diffuse_radiance_history,
			)
		});
		let surfel_resolve = targets.surfels.map(|surfels| {
			SurfelResolvePass::new(
				context,
				depth_pyramid.depth_pyramid,
				depth_pyramid.view_data,
				targets.ssgi,
				surfels,
				&debug_views,
			)
		});
		let probe_resolve = probe_gi.then(|| {
			ProbeResolvePass::new(
				context,
				depth_pyramid.depth_pyramid,
				depth_pyramid.view_data,
				targets.ssgi,
				&debug_views,
			)
		});
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
			targets.lit,
			targets.depth,
		);
		let backdrop = BackdropPass::new(
			context,
			&pipeline_manager,
			depth_pyramid.view_descriptor_set,
			material_evaluation_descriptor_set,
			depth_sampler,
			targets.lit,
			depth,
			targets.backdrop,
		);
		let visibility = set_layouts::VISIBILITY_BUFFER.binder(visibility_buffer_descriptor_set);
		let evaluation = set_layouts::MATERIAL_EVALUATION.binder(material_evaluation_descriptor_set);
		// The debug target pool, which the shown views write; see `MATERIAL_DEBUG_VIEWS`.
		let mut writes = debug_views
			.targets()
			.iter()
			.enumerate()
			.map(|(slot, image)| {
				evaluation.image_element(crate::rendering::debug_views::DEBUG_VIEW_TARGETS_BINDING, *image, slot as u32)
			})
			.collect::<Vec<_>>();
		writes.extend([
			evaluation.image("lit_map", targets.lit),
			evaluation.buffer("lighting_data", lighting_buffer),
			evaluation.sampled("ao", targets.gtao.ambient_occlusion, linear_sampler),
			evaluation.sampled("gtao_bent_normal", targets.gtao.bent_normal, linear_sampler),
			// Material evaluation upsamples the half-resolution SSGI result itself. It fetches texels at edges, and filters the
			// history in one read where all four texels lie on the pixel's plane.
			evaluation.sampled("ssgi_history", targets.ssgi.history, linear_sampler),
			evaluation.sampled("ssgi_normals", targets.ssgi.normals, depth_sampler),
			evaluation.buffer("ssgi_view", depth_pyramid.view_data),
			evaluation.sampled("sun_visibility", targets.sun_visibility.visibility, depth_sampler),
			evaluation.sampled("depth_shadow_map", shadow_maps.directional, depth_sampler),
			// The layer fixes the pyramid's maximum sampler.
			evaluation.texture("directional_shadow_depth_pyramid", shadow_maps.directional_depth_pyramid),
			visibility.image("triangle_index", targets.primitive_index),
			visibility.image("instance_index_render_target", targets.instance_id),
		]);
		// Without SSGI the shaders leave out `diffuse_radiance_map`, so nothing is bound there.
		writes.extend(
			targets
				.diffuse_radiance_history
				.map(|image| evaluation.image("diffuse_radiance_map", image)),
		);
		// Without subsurface scattering the shaders leave out its images, so nothing is bound there.
		if let Some(subsurface) = targets.subsurface {
			writes.extend([
				evaluation.image("subsurface_diffuse", subsurface.diffuse),
				evaluation.image("subsurface_shape", subsurface.shape),
				evaluation.image("subsurface_albedo", subsurface.albedo),
			]);
		}
		// One image of each local shadow-map size class, which a light picks by its class.
		for (size_class, (cone, point)) in shadow_maps.cone.iter().zip(&shadow_maps.point).enumerate() {
			writes.extend([
				evaluation.sampled_element("cone_shadow_maps", *cone, depth_sampler, size_class as u32),
				evaluation.sampled_element("point_shadow_maps", *point, depth_sampler, size_class as u32),
			]);
		}
		context.write(&writes);

		Self {
			cascade_fit,
			light_clusters,
			visibility: VisibilityPass {
				descriptor_sets: [base_descriptor_set, occlusion.descriptor_set],
				pipelines: Pipelines::sides(&pipeline_manager, "visibility"),
				primitive_index: targets.primitive_index,
				instance_id: targets.instance_id,
				depth,
			},
			occlusion,
			material_evaluation: MaterialEvaluationPass {
				base_descriptor_set,
				visibility_descriptor_set: visibility_buffer_descriptor_set,
				descriptor_set: material_evaluation_descriptor_set,
			},
			material_prepasses,
			gtao: features.gtao.then(|| {
				GtaoPass::new(
					context,
					&pipeline_manager,
					depth,
					depth_pyramid.depth_pyramid,
					depth_pyramid.view_descriptor_set,
					targets.gtao,
				)
			}),
			sun_visibility: SunVisibilityPass::new(
				context,
				&pipeline_manager,
				base_descriptor_set,
				depth,
				targets.primitive_index,
				targets.instance_id,
				depth_pyramid.depth_pyramid,
				depth_pyramid.view_data,
				shadow_maps,
				targets.sun_visibility,
				features.contact_shadows,
			),
			fog_light: FogLightPass::new(
				context,
				&pipeline_manager,
				base_descriptor_set,
				lighting_buffer,
				depth,
				shadow_maps,
				targets.fog_light,
			),
			depth_pyramid,
			ssgi,
			surfel_resolve,
			probe_resolve,
			subsurface: targets
				.subsurface
				.map(|subsurface| SubsurfacePass::new(context, &pipeline_manager, depth, targets.lit, subsurface)),
			reflections,
			backdrop,
			stage_counters,
			pipeline_manager,
			debug_views,
		}
	}

	/// Returns the descriptor set that carries material-evaluation-only resources, including the environment.
	pub(crate) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation.descriptor_set
	}

	/// Prepares one opaque visibility layer, the scene `background`, and one nearest-surface transparent layer.
	///
	/// Returns `None` while any fixed pipeline is still compiling. `frame_work` is the work one sink records for the
	/// whole frame. The visibility pipeline manager passes it only to the first sink, whose camera the shadow views were
	/// made for, so that sink also fits the cascades to its surfaces. `history` describes how this pass recorded the sink in the previous frame at the same
	/// extent, or is `None` when the previous frame's images do not hold this sink's data. `exposure` is the shared
	/// lighting exposure uploaded for every sink in this frame. `indirect_diffuse` is the source the visibility pipeline
	/// manager picked to light this frame's indirect diffuse light, and `surfels` or `probes` the frame's surfel or probe
	/// work when it picked surfel GI or probe GI; the sink that records the frame-wide work also records the surfel grid
	/// or the probe updates. `gtao_settings` and
	/// `contact_shadow_settings` are the runtime controls the manager holds. `fog` is the newest height fog, which the
	/// sink's fog light march lights.
	pub(crate) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_work: Option<FrameWork<'a>>,
		dispatches: PhaseDispatches,
		render_info: &'a RenderInfo,
		shadow_work: &ShadowWork,
		history: Option<SinkHistory>,
		exposure: f32,
		indirect_diffuse: Option<IndirectDiffuseSource>,
		surfels: Option<SurfelFrame>,
		probes: Option<ProbeFrame>,
		gtao_settings: GtaoSettings,
		contact_shadow_settings: ContactShadowSettings,
		fog: Option<&crate::rendering::ExponentialHeightFog>,
		background: Option<&crate::rendering::render_pass::SceneBackground>,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<impl RenderPassFunction + use<'a>> {
		let pipeline_manager = &self.pipeline_manager;
		// The cascades were made for the camera of the sink that records the frame-wide work, so only its surfaces
		// can fit them.
		let receiver_fit = match frame_work {
			Some(_) => shadow_work.receiver_fit.as_slice(),
			None => &[],
		};
		let (skinning, mesh_work, shadows) = match frame_work {
			Some(FrameWork {
				skinning,
				mesh_work,
				shadows: shadow_maps,
				..
			}) => (
				Some((skinning, pipeline_manager.pipeline(skinning.pipeline)?)),
				Some((mesh_work, mesh_work.pipelines(pipeline_manager)?)),
				Some(shadow_maps.prepare(
					frame,
					pipeline_manager,
					dispatches,
					shadow_work,
					self.visibility.descriptor_sets,
					self.stage_counters,
					render_info.coverage_pipelines,
				)?),
			),
			None => (None, None, None),
		};
		let visibility_pipelines = opaque_layer_pipelines(
			self.visibility.pipelines.resolve(pipeline_manager)?,
			render_info.coverage_pipelines.map(|coverage| coverage.visibility),
		);
		let material_prepasses = match &self.material_prepasses {
			Some(prepasses) => Some((prepasses, prepasses.pipelines.resolve(pipeline_manager)?)),
			// Without prepasses, the opaque uber shader writes the background's zeros, so the frame waits for it as it
			// waits for pixel mapping.
			None if render_info.opaque_evaluations.is_empty() => return None,
			None => None,
		};
		let cascade_masks = [shadow_work.updated_cascades(), shadow_work.drawn_cascades];
		let cascade_fit = self.cascade_fit.prepare(
			frame,
			pipeline_manager,
			receiver_fit,
			shadow_work.cascade_resolution,
			cascade_masks,
			sink,
		)?;
		let fitted_suns = receiver_fit.len() as u32;
		let fits_receivers = fitted_suns > 0;
		let [light_cluster_pipeline] = self.light_clusters.pipelines.resolve(pipeline_manager)?;
		let [depth_pyramid_pipeline] = self.depth_pyramid.pipelines.resolve(pipeline_manager)?;
		let occlusion_pyramid = self.occlusion.prepare(pipeline_manager)?;
		let [sun_visibility_pipeline] = self.sun_visibility.resolve_pipelines.resolve(pipeline_manager)?;
		// A disabled pass neither records nor holds the frame back while its pipelines compile.
		let contact_shadow_pipeline = match &self.sun_visibility.trace_pipelines {
			Some(pipelines) if contact_shadow_settings.enabled => Some(pipelines.resolve(pipeline_manager)?[0]),
			_ => None,
		};
		let gtao_pipelines = match self.gtao.as_ref().filter(|_| gtao_settings.enabled) {
			Some(gtao) => Some((gtao, gtao.pipelines.resolve(pipeline_manager)?)),
			None => None,
		};
		let subsurface_pipelines = match &self.subsurface {
			Some(subsurface) => Some((subsurface, subsurface.pipelines.resolve(pipeline_manager)?)),
			None => None,
		};
		let lit_by_ssgi = indirect_diffuse == Some(IndirectDiffuseSource::ScreenSpace);
		let ssgi_pipelines = match self.ssgi.as_ref().filter(|_| lit_by_ssgi) {
			Some(ssgi) => Some((ssgi, ssgi.pipelines.resolve(pipeline_manager)?)),
			None => None,
		};
		let light_clusters = self.light_clusters.prepare(frame, sink, light_cluster_pipeline);
		let depth_pyramid = self.depth_pyramid.prepare(frame, sink, depth_pyramid_pipeline, fitted_suns);
		let sun_visibility = self.sun_visibility.prepare(
			frame,
			sink,
			&shadow_work.suns,
			contact_shadow_settings,
			contact_shadow_pipeline,
			sun_visibility_pipeline,
		);
		let gtao = gtao_pipelines.map(|(gtao, pipelines)| gtao.prepare(frame, sink, gtao_settings, pipelines));
		let subsurface = subsurface_pipelines.map(|(subsurface, pipelines)| subsurface.prepare(frame, sink, pipelines));
		// SSGI history exists only if SSGI also lit last frame: another source leaves no diffuse radiance for its rays.
		let ssgi_history = history.filter(|history| history.indirect_diffuse == Some(IndirectDiffuseSource::ScreenSpace));
		let ssgi = ssgi_pipelines.map(|(ssgi, pipelines)| ssgi.prepare(frame, sink, ssgi_history, exposure, pipelines));
		// Material evaluation writes the first views, and the resolve that lights the frame the two after them.
		let material_views = 0..super::shader_generator::MATERIAL_DEBUG_VIEW_NAMES.len();
		let material_debug_views = self.debug_views.shown(material_views.clone());
		let resolve_debug_views = self.debug_views.shown(material_views.end..material_views.end + 2);
		let lit_by_surfels = indirect_diffuse == Some(IndirectDiffuseSource::Surfels);
		// Surfel history likewise exists only if surfel GI lit last frame.
		let surfel_history = history.filter(|history| history.indirect_diffuse == Some(IndirectDiffuseSource::Surfels));
		let surfel_resolve = match (&self.surfel_resolve, &surfels) {
			(Some(resolve), Some(surfels)) if lit_by_surfels => {
				Some(resolve.prepare(frame, sink, surfels, surfel_history, exposure, resolve_debug_views))
			}
			_ => None,
		};
		// The grid serves every sink, so the sink that records the frame-wide work builds it before any resolve.
		let surfel_grid = surfels.filter(|_| frame_work.is_some() && surfel_resolve.is_some());
		let lit_by_probes = indirect_diffuse == Some(IndirectDiffuseSource::Probes);
		let probe_resolve = match (&self.probe_resolve, &probes) {
			(Some(resolve), Some(probes)) if lit_by_probes => {
				// The two views after material evaluation's are surfel GI's when it is built in.
				let probe_views = if self.surfel_resolve.is_some() {
					ShownDebugViews::default()
				} else {
					resolve_debug_views
				};
				Some(resolve.prepare(frame, sink, probes, exposure, probe_views))
			}
			_ => None,
		};
		// Like the grid, the probe updates serve every sink, so the sink that records the frame-wide work runs them.
		let probe_updates = probes.filter(|_| frame_work.is_some() && probe_resolve.is_some());
		// The acceleration structures build with the grid or the probe updates, so the frame that builds them traces
		// them too.
		let acceleration_structures = frame_work
			.and_then(|work| work.acceleration_structures)
			.filter(|_| surfel_grid.is_some() || probe_updates.is_some());
		self.reflections.prepare(frame, history);
		// The height fog pass lights the fog unshadowed while the march's pipelines compile, so they don't hold the
		// frame back.
		let fog_light = fog.and_then(|fog| self.fog_light.prepare(frame, pipeline_manager, sink, fog));
		let screen_space_lighting = ScreenSpaceLighting {
			gtao: gtao.is_some(),
			ssgi: ssgi.is_some(),
			// SSGI and the surfel and probe resolves write the same indirect diffuse images.
			indirect_diffuse: match (&ssgi, &surfel_resolve, &probe_resolve) {
				(Some(_), ..) => Some(IndirectDiffuseSource::ScreenSpace),
				(None, Some(_), _) => Some(IndirectDiffuseSource::Surfels),
				(None, None, Some(_)) => Some(IndirectDiffuseSource::Probes),
				// Lightmaps need no pass: material evaluation reads them itself.
				(None, None, None) => indirect_diffuse.filter(|source| *source == IndirectDiffuseSource::Baked),
			},
		};
		let pixel_lists = self
			.material_prepasses
			.as_ref()
			.map(|prepasses| prepasses.evaluation_dispatches);
		let opaque_materials = self.material_evaluation.prepare(
			&render_info.opaque_evaluations,
			&render_info.opaque_evaluation_mask,
			VisibilityPhase::Opaque,
			pixel_lists,
			sink.extent(),
			screen_space_lighting,
			material_debug_views,
		);
		let transparent_materials = self.material_evaluation.prepare(
			&render_info.transparent_evaluations,
			&render_info.transparent_evaluation_mask,
			VisibilityPhase::Transparent,
			pixel_lists,
			sink.extent(),
			screen_space_lighting,
			material_debug_views,
		);
		// Prepare the background last: it may record one-time work, such as building lookup tables, that would be
		// lost if this pass gave up on the frame after it. A background still compiling leaves the sky black for this
		// frame instead of holding the scene back.
		let background = background.and_then(|background| background.prepare(frame, sink, frame_allocator));
		// The generated environment is prepared last for the same reason, and recorded first: surfel and probe rays and
		// material evaluation all read it.
		let environment = frame_work
			.and_then(|work| work.environment)
			.and_then(|environment| environment.prepare(frame, sink, frame_allocator));
		let extent = sink.extent();
		// Blended materials read the scene behind them only from the copy, so it runs only in frames that draw one.
		let reads_backdrop = !dispatches.transparent.is_empty()
			&& render_info.transparent_evaluations.iter().any(|evaluation| {
				evaluation.reads_backdrop && evaluation_is_active(&render_info.transparent_evaluation_mask, evaluation.index)
			});
		let backdrop = reads_backdrop
			.then(|| self.backdrop.prepare(pipeline_manager, extent))
			.flatten();
		// The transparent phase waits for the copy it reads while the copy's pipeline compiles, as it waits for its own.
		let draws_transparent = !dispatches.transparent.is_empty() && (backdrop.is_some() || !reads_backdrop);
		// Each stage records its counter only when it records commands, so an idle stage reports no time.
		let counters = self.stage_counters;
		let skinning = skinning.filter(|_| !render_info.skinning_dispatches.is_empty());

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::CommonCommandBufferMode as _;

			c.start_region(|label| label.write_str("Visibility Render Model"));
			if let Some(environment) = environment {
				environment(c);
			}
			if let Some((pass, pipeline)) = skinning {
				c.counter(counters.skinning, |c| {
					// New ranges get their UVs beside the positions and normals skinning writes, before any stage reads them.
					ghi::command_buffer::CommandBufferRecording::copy_buffers(c, &render_info.posed_uv_copies);
					pass.record(c, &render_info.skinning_dispatches, pipeline)
				});
			}
			// Every pass below that draws meshlets dispatches the work this builds.
			if let Some((work, pipelines)) = mesh_work {
				work.record(c, pipelines);
			}
			// Cascades fitted to the camera's surfaces are drawn once the opaque layer's depth exists. The shadow
			// recording times its maps and pyramid with this sink's counters itself.
			if !fits_receivers && let Some(shadows) = &shadows {
				shadows(c);
			}
			// Both material evaluation layers read the clusters, and nothing before them does.
			c.counter(counters.light_clusters, &light_clusters);
			// The frame-wide surfel work reads nothing the raster below writes, so recording it first lets the GPU
			// overlap the two.
			if let Some(work) = &acceleration_structures {
				c.counter(counters.acceleration_structures, |c| {
					use ghi::command_buffer::CommandBufferRecording as _;

					// The top level may place the bottom levels this recording builds, so they record first.
					c.build_bottom_level_acceleration_structures(work.builds);
					c.build_top_level_acceleration_structure(work.top_level, work.instances);
				});
			}
			// The surfel grid serves every sink's resolve and the rays' hits, which read the cells around them. The support
			// rays read the radius the grid wrote; they record after the trace, so their counter doesn't take in the wait
			// for the acceleration structures. The estimate update folds the rays into the records the resolves read, and
			// the hits' spawns join the pool after it, so the next frame's grid lists them.
			if let Some(surfels) = &surfel_grid {
				c.counter(counters.surfel_grid, |c| {
					record_compute_stages(c, Some("Surfel Grid"), &surfels.grid)
				});
				c.counter(counters.surfel_trace, |c| surfels.rays.record(c));
				c.counter(counters.surfel_validate, |c| {
					record_compute_stages(c, None, &[surfels.validate])
				});
				c.counter(counters.surfel_integrate, |c| record_compute_stages(c, None, &surfels.update));
			}
			// The probe updates bring every probe the sinks' resolves read up to date. The trace reads the volume before
			// the update rewrites it, so the hits bounce the light the probes held last frame.
			if let Some(probes) = &probe_updates {
				c.counter(counters.probe_schedule, |c| {
					record_compute_stages(c, Some("Probe Schedule"), &probes.schedule)
				});
				c.counter(counters.probe_trace, |c| probes.rays.record(c));
				c.counter(counters.probe_update, |c| record_compute_stages(c, None, &[probes.update]));
			}

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
			if let Some((prepasses, pipelines)) = material_prepasses {
				c.counter(counters.material_prepasses, |c| {
					prepasses.record(c, extent, pipelines, VisibilityPhase::Opaque)
				});
			}
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
			// Each dispatch also times itself, without a barrier, so the overlap stays.
			let timed = counters.screen_space_dispatches;
			c.counter(counters.screen_space, |c| {
				if let Some(trace) = sun_visibility.and_then(|sun_visibility| sun_visibility.trace) {
					c.counter(timed.sun_visibility_trace, |c| record_compute_stages(c, None, &[trace]));
				}
				if let Some(gtao) = &gtao {
					c.counter(timed.gtao_evaluate, |c| record_compute_stages(c, None, &[gtao.evaluate]));
				}
				if let Some(ssgi) = &ssgi {
					c.counter(timed.ssgi_trace, |c| record_compute_stages(c, None, &[ssgi.trace]));
				}
				if let Some(surfels) = &surfel_resolve {
					c.counter(timed.surfel_resolve, |c| record_compute_stages(c, None, &[surfels.resolve]));
				}
				if let Some(resolve) = &probe_resolve {
					c.counter(timed.probe_resolve, |c| record_compute_stages(c, None, &[*resolve]));
				}
				if let Some(sun_visibility) = &sun_visibility {
					c.counter(timed.sun_visibility_resolve, |c| {
						record_compute_stages(c, None, &[sun_visibility.resolve])
					});
				}
				if let Some(gtao) = &gtao {
					c.counter(timed.gtao_blur, |c| record_compute_stages(c, None, &[gtao.blur]));
				}
				if let Some(ssgi) = &ssgi {
					c.counter(timed.ssgi_temporal, |c| record_compute_stages(c, None, &[ssgi.temporal]));
				}
				// The surfel spatial pass reads the history the resolve accumulated, so it records with the second stages.
				if let Some(surfels) = &surfel_resolve {
					c.counter(timed.surfel_allocate, |c| record_compute_stages(c, None, &[surfels.allocate]));
					c.counter(timed.surfel_spatial, |c| record_compute_stages(c, None, &[surfels.spatial]));
				}
				if let Some(gtao) = &gtao {
					c.counter(timed.gtao_upscale, |c| record_compute_stages(c, None, &[gtao.upscale]));
				}
			});
			// The march reads the shadow maps and the opaque depth, which every order above records before it.
			if let Some(stages) = &fog_light {
				c.counter(counters.fog_light, |c| record_compute_stages(c, None, stages));
			}
			c.counter(counters.material_evaluation, &opaque_materials);
			// Subsurface pixels add their spread diffuse light before the background and the backdrop copy, so blended
			// surfaces see it behind them.
			if let Some(stage) = &subsurface {
				c.counter(counters.subsurface, |c| {
					record_compute_stages(c, None, std::slice::from_ref(stage))
				});
			}
			// The background fills pixels no opaque surface covered, so transparent surfaces composite over it.
			if let Some(background) = background {
				c.counter(counters.background, background);
			}

			// The copy reads the opaque layer and the background, before the transparent draw overwrites the depth.
			if let Some(stage) = &backdrop {
				c.counter(counters.backdrop, |c| {
					record_compute_stages(c, None, std::slice::from_ref(stage))
				});
			}
			// The visibility buffer holds one transparent layer. Resolving every blend primitive together lets
			// normal depth testing select the nearest surface before source-over evaluation.
			if draws_transparent {
				c.counter(counters.transparent, |c| {
					draw(c, VisibilityPhase::Transparent, OcclusionPhase::Test);
					if let Some((prepasses, pipelines)) = material_prepasses {
						prepasses.record(c, extent, pipelines, VisibilityPhase::Transparent);
					}
					transparent_materials(c);
				});
			}
			c.end_region();
		})
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::configuration::{ConfigurationValue, apply_setting};

	fn text(value: &str) -> ConfigurationValue {
		ConfigurationValue::Text(value.to_string())
	}

	#[test]
	fn gi_defaults_to_the_technique_the_project_built_in() {
		let features = |ssgi, surfel_gi, probe_gi| VisibilityFeatures {
			ssgi,
			surfel_gi,
			probe_gi,
			..VisibilityFeatures::default()
		};

		assert_eq!(
			GiSettings::new(features(true, true, true)).technique,
			IndirectDiffuseSource::ScreenSpace
		);
		assert_eq!(
			GiSettings::new(features(false, true, true)).technique,
			IndirectDiffuseSource::Probes
		);
		assert_eq!(
			GiSettings::new(features(false, true, false)).technique,
			IndirectDiffuseSource::Surfels
		);
		assert!(GiSettings::new(features(true, false, false)).enabled);
	}

	/// Applies one `render.gi.` update to the settings of a project built with `features`.
	fn set(
		features: VisibilityFeatures,
		name: &str,
		value: ConfigurationValue,
	) -> Result<(GiSettings, ConfigurationValue), String> {
		apply_setting(GiSettings::new(features), GiSettings::SETTINGS, "", name, &value)
	}

	#[test]
	fn gi_enabled_accepts_booleans_and_startup_text() {
		let features = VisibilityFeatures::default();
		let (disabled, effective) = set(features, "enabled", text("false")).unwrap();
		assert!(!disabled.enabled);
		assert_eq!(effective, ConfigurationValue::Bool(false));
		let (enabled, _) =
			apply_setting(disabled, GiSettings::SETTINGS, "", "enabled", &ConfigurationValue::Bool(true)).unwrap();
		assert!(enabled.enabled);
		assert!(set(features, "enabled", text("off")).is_err());
	}

	#[test]
	fn gi_technique_selects_only_a_built_in_technique() {
		let both = VisibilityFeatures {
			surfel_gi: true,
			..VisibilityFeatures::default()
		};

		let (surfels, effective) = set(both, "technique", text("surfels")).unwrap();
		assert_eq!(surfels.technique, IndirectDiffuseSource::Surfels);
		assert_eq!(effective, text("surfels"));
		let (ssgi, effective) = apply_setting(surfels, GiSettings::SETTINGS, "", "technique", &text("SSGI")).unwrap();
		assert_eq!(ssgi.technique, IndirectDiffuseSource::ScreenSpace);
		assert_eq!(effective, text("ssgi"));
		assert!(set(both, "technique", text("voxels")).is_err());

		// A technique left out of the shaders can't be selected, and the reason names the parameter that builds it in.
		let reason = set(VisibilityFeatures::default(), "technique", text("surfels")).unwrap_err();
		assert!(reason.contains("`render.surfel-gi.enabled`"), "{reason}");
		let reason = set(both, "technique", text("probes")).unwrap_err();
		assert!(reason.contains("`render.probe-gi.enabled`"), "{reason}");

		let probes_built_in = VisibilityFeatures { probe_gi: true, ..both };
		let (probes, effective) = set(probes_built_in, "technique", text("Probes")).unwrap();
		assert_eq!(probes.technique, IndirectDiffuseSource::Probes);
		assert_eq!(effective, text("probes"));
	}

	#[test]
	fn gi_technique_selects_lightmaps_only_when_they_are_built_in() {
		let baked = VisibilityFeatures {
			ssgi: false,
			baked_gi: true,
			..VisibilityFeatures::default()
		};
		assert_eq!(GiSettings::new(baked).technique, IndirectDiffuseSource::Baked);

		let (lightmaps, effective) = set(baked, "technique", text("Baked")).unwrap();
		assert_eq!(lightmaps.technique, IndirectDiffuseSource::Baked);
		assert_eq!(effective, text("baked"));
		let reason = set(VisibilityFeatures::default(), "technique", text("baked")).unwrap_err();
		assert!(reason.contains("`render.baked-gi.enabled`"), "{reason}");
	}
}
