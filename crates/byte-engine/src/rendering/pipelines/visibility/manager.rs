//! Owns the visibility scene, adopts asynchronously loaded resources, and prepares each frame's passes.
//!
//! Loading completion is not the same as scene readiness: a renderable enters a frame only when its mesh, every
//! material, and every texture those materials sample are resident. [`VisibilityPipelineManager`] tracks that
//! closure in an availability graph and rebuilds the frame's instance lists from it.
//! It requests resources through the visibility loader façade and consumes only ready or unavailable domain
//! events; worker protocol and compilation state do not cross into this renderer-owned scene layer.

#![expect(
	clippy::disallowed_types,
	reason = "Not given one owner yet; see \"Give the remaining shared state one owner\" in todo.md"
)]

use std::sync::Arc;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use log::{error, warn};
use resource_management::resources::skeleton::SkinBinding;
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use tracing::debug_span;
use utils::hash::{HashMap, HashSet};
use utils::{AvailabilityGraph, Extent, StableVec};

use super::acceleration_structures::{SceneAccelerationStructures, TracedPrimitive};
use super::features::{
	CONTACT_SHADOWS_ENABLED_PARAMETER, DIRECTIONAL_LIGHTS_MAX_PARAMETER, GTAO_ENABLED_PARAMETER, MaterialEvaluationMode,
	RENDER_PARAMETERS, UberVariant, VisibilityFeatures,
};
use super::geometry::{GeometryCapacity, GeometryHandles, allocate_vertices};
use super::instance_rebuild::{
	Admission, Followed, InstanceOutputs, InstanceSources, Kept, LaneScratch, follow_instances, instance_phase,
	rebuild_instances, rebuild_lanes,
};
use super::layout::{
	CONE_SHADOW_VIEW_OFFSET, DEFAULT_SHADOW_MAP_BUDGET_MIB, DEFAULT_SHADOW_MAP_RESOLUTION, DIRECTIONAL_SHADOW_VIEW_OFFSET,
	MAX_BINDLESS_TEXTURES, MAX_LIGHTS, MAX_MATERIAL_PARAMETERS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS, NO_EVALUATION,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT, VERTEX_UV_BUFFER_STRIDE,
};
use super::lightmaps::Lightmaps;
use super::loader::{
	MaterialEvaluator, ResidentEnvironment, ResidentMaterial, ResidentTexture, TextureArray, VisibilityLoaderClient,
	VisibilityLoaderEvent,
};
use super::lod::{LodSettings, LodView, MeshLods};
use super::material_parameters::MaterialParameters;
use super::mesh_dispatch::MeshWork;
use super::ray_tracing::{HitShading, PROBE_SHADE_SHADER, SURFEL_SHADE_SHADER};
use super::render_pass::{
	ContactShadowSettings, DEFAULT_PROBE_RAY_BUDGET, DEFAULT_PROBE_SPACING, DEFAULT_SURFEL_CAPACITY, DEFAULT_SURFEL_RAY_BUDGET,
	DIRECTIONAL_SHADOW_RESOLUTION_MULTIPLE, FrameWork, GiSettings, GtaoSettings, IndirectDiffuseSource, MAX_SURFEL_RAY_BUDGET,
	PROBE_RAY_BUDGET_RANGE, PROBE_RAYS, PROBE_SPACING_RANGE, ProbeGiSettings, ProbeVolume, RayHits, RayTracing,
	SURFEL_CAPACITY_RANGE, SURFEL_RAY_BUDGET_STEP, ShadowMaps, ShadowWork, SinkHistory, SinkTargets, SunCascades,
	SurfelGiSettings, SurfelLight, SurfelPool, VisibilityRenderPass, create_backdrop_targets, create_diffuse_radiance_target,
	create_fog_light_target, create_gtao_targets, create_indirect_diffuse_targets, create_subsurface_targets,
	create_sun_visibility_targets, create_surfel_targets, create_velocity_targets,
};
use super::residency::MeshResidencies;
use super::scene::{
	InstanceCapacity, MaterialEvaluation, PosedVertices, RenderEntity, RenderInfo, RenderSkin, SinkState, VisibilityScene,
};
use super::screen_queries::ScreenQueries;
use super::set_layouts;
use super::shader_data::{FIXED_CUTOFF, IesProfileTexture, MaterialData, ShaderFrame, ShaderMesh, ShaderViewData};
use super::shadow_selection::{
	SHADOW_DEFAULT_EXPOSURE_SCALE, ShadowBudget, ShadowLayout, ShadowLightSelection, ShadowSlots, make_cone_shadow_view,
	make_point_shadow_view, retain_layout, select_shadow_lights, sun_cascade_view,
};
use super::shadow_updates::{
	DEFAULT_SHADOW_UPDATE_TEXELS, ShadowMapStates, ShadowViews, drawn_cascade_view, undrawn_cascade_view,
};
use super::skinning::{
	DualQuaternion, MAX_SKINNING_MATRICES, SkinningDispatch, SkinningPaletteKind, SkinningPass, append_dual_quaternion_palette,
};
use super::terrain::{TerrainChange, TerrainStreams};
use super::virtual_shadows::{
	DirectionalShadowTechnique, ShaderVirtualShadowFrame, VirtualShadowLevels, VirtualShadowSettings,
	place_virtual_shadow_levels, range_union,
};
use crate::application::parameters::Parameters;
use crate::configuration::{ConfigurationPort, Setting};
use crate::core::alley::Alley;
use crate::core::factory::{CreateMessage, Factory, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::core::message::DeleteMessage;
use crate::core::message_bus::MessageScope;
use crate::gameplay::Transform;
use crate::rendering::csm::{self, CascadeFitting, CascadeSplits};
use crate::rendering::lights::{ConeLight, DirectionalLight, Lights, LocalEmission, PointLight};
use crate::rendering::lookup_textures::{blue_noise_slice, create_blue_noise_texture, create_lookup_texture};
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::renderable::mesh::MeshKey;
use crate::rendering::{
	Environment, Lightmap, PipelineManagerClient, RenderableMesh, RenderableTerrain, Resource, SetBinder, SetMaterialParameter,
	ShownTransforms, Sink, UpdatePose, View,
};

/// The startup parameter, inside the `render` subtree, that sets the shadow-map memory every light shares, in
/// mebibytes. It must not be zero, which would leave every light without shadows.
///
/// The first directional light takes its cascades first, cone and point lights then share what remains by how much of
/// the screen they light, and other directional lights take what is left after them. Lights that do not fit stay lit
/// without shadows. One sun takes 32 MiB at the default cascade resolution, four times that per doubling of
/// [`DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER`]. A cone light takes 32 KiB to 8 MiB and a point light 192 KiB to 48 MiB,
/// by how large it looks in the largest sink that sees it.
pub const SHADOW_MAP_BUDGET_PARAMETER: &str = "shadow-maps.budget-mb";
/// The startup parameter, inside the `render` subtree, that sets how many shadow-map texels each frame draws: a cascade
/// costs its resolution squared, and a cone map or point cube face its size class's texels per side squared. It must
/// not be zero.
///
/// Cascades 0 and 1 of every shadowed sun draw every frame and count first. The other cascades, cone maps, and point
/// cube faces then draw by need within what's left, so a smaller budget spreads their updates over more frames. Texels
/// a frame leaves unspent carry over to the next. The rules are specified in the
/// [shadow map updates](/docs/reference/rendering#shadow-map-updates) of the visibility pipeline reference.
pub const SHADOW_UPDATE_TEXELS_PARAMETER: &str = "shadow-maps.update-texels";
/// The prefix, inside the `render` subtree, of the startup parameters that set each scene-wide geometry buffer's
/// element count, such as `render.geometry.triangle-capacity`. Meshes that would overflow a buffer are rejected at
/// upload, so size them for the largest resident scene. See [`GeometryCapacity`].
pub const GEOMETRY_CAPACITY_PARAMETER_PREFIX: &str = "geometry.";
/// The startup parameter, inside [`RENDER_PARAMETERS`], that turns the meshlet vertex stream on or off:
/// `render.geometry.meshlet-vertices`. See [`GeometryCapacity::meshlet_vertices`].
pub const MESHLET_VERTICES_PARAMETER: &str = "geometry.meshlet-vertices";
/// The prefix, inside the `render` subtree, of the startup parameters that size the per-frame instance tables:
/// `render.instances.capacity` and `render.instances.meshlet-group-capacity`. Instances past the capacity are left out
/// of the frame, so size them for the most instances shown at once. See [`InstanceCapacity`].
pub const INSTANCE_CAPACITY_PARAMETER_PREFIX: &str = "instances.";
/// The startup parameter, inside the `render` subtree, that caps how many lanes rebuild the frame's instances. One
/// keeps the rebuild on the render thread. By default the rebuild takes a lane for every 8,192 render entities, up to
/// the machine's performance cores.
pub const INSTANCE_REBUILD_LANES_PARAMETER: &str = "instances.rebuild-lanes";
/// The startup parameters, inside the `render` subtree, that set how far directional shadows reach, in meters, and
/// the share of their cascade splits that is logarithmic. See [`CascadeSplits`].
pub const DIRECTIONAL_SHADOW_DISTANCE_PARAMETER: &str = "directional-shadows.distance";
pub const DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER: &str = "directional-shadows.split-blend";
/// The startup parameter, inside the `render` subtree, that sets what directional shadow cascades cover:
/// `receivers` or `frustum`. See [`CascadeFitting`].
pub const DIRECTIONAL_SHADOW_FITTING_PARAMETER: &str = "directional-shadows.fitting";
/// The startup parameter, inside the `render` subtree, that sets the directional cascades' resolution in texels per
/// side, a positive multiple of 32, which every level of the cascade depth pyramids reduces in whole cells. Halving it quarters the
/// shadow map raster and depth pyramid work and doubles the size of a shadow texel on the ground.
pub const DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER: &str = "directional-shadows.resolution";
/// The startup parameter, inside the `render` subtree, that chooses how directional lights cast shadows: `cascades` or
/// `virtual`. See [`DirectionalShadowTechnique`].
pub const DIRECTIONAL_SHADOW_TECHNIQUE_PARAMETER: &str = "directional-shadows.technique";
/// The startup parameters, inside the `render` subtree, that size a virtual shadow map: its finest level's side in
/// meters, the scale on the distance receivers pick their level by, and the pages its pool holds. See
/// [`VirtualShadowSettings`].
pub const VIRTUAL_SHADOW_LEVEL_SIZE_PARAMETER: &str = "directional-shadows.first-level-size";
pub const VIRTUAL_SHADOW_LEVEL_BIAS_PARAMETER: &str = "directional-shadows.level-bias";
pub const VIRTUAL_SHADOW_POOL_PAGES_PARAMETER: &str = "directional-shadows.pool-pages";
/// The startup parameter, inside the `render` subtree, that sets how many surfels surfel GI keeps alive at once: a
/// power of two from 1,024 through 16,777,216. More surfels cover more of what the camera sees in finer detail, and
/// take more memory.
pub const SURFEL_CAPACITY_PARAMETER: &str = "surfel-gi.surfel-capacity";
/// The startup parameter, inside the `render` subtree, that sets how many rays surfels trace each frame: a multiple of
/// 1,024 from 1,024 through 1,048,576. A larger budget converges faster after the camera or the lighting changes.
pub const SURFEL_RAY_BUDGET_PARAMETER: &str = "surfel-gi.ray-budget";
/// The startup parameter, inside the `render` subtree, that sets how many rays probes trace each frame: a multiple of
/// 128, the rays of one probe update, from 128 through 1,048,576. A larger budget follows lighting changes faster.
pub const PROBE_RAY_BUDGET_PARAMETER: &str = "probe-gi.ray-budget";
/// The startup parameter, inside the `render` subtree, that sets how far apart the finest probe cascade's probes stand:
/// from 0.05 through 100 meters. Each coarser cascade doubles it. Closer probes resolve smaller rooms and keep light from
/// leaking through thinner walls; the four cascades then reach less far.
pub const PROBE_SPACING_PARAMETER: &str = "probe-gi.spacing";

/// The `VisibilityPipelineSettings` struct configures memory limits and shadow coverage for the visibility rendering
/// pipeline.
///
/// Read it with [`Self::from_parameters`], then hand it to the visibility pipeline manager.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibilityPipelineSettings {
	geometry_capacity: GeometryCapacity,
	instance_capacity: InstanceCapacity,
	instance_rebuild_lanes: Option<std::num::NonZeroUsize>,
	shadow_map_budget_mib: u32,
	shadow_update_texels: u64,
	cascade_splits: CascadeSplits,
	cascade_fitting: CascadeFitting,
	directional_shadow_map_resolution: u32,
	directional_shadow_technique: DirectionalShadowTechnique,
	virtual_shadows: VirtualShadowSettings,
	surfel_capacity: u32,
	surfel_ray_budget: u32,
	probe_ray_budget: u32,
	probe_spacing: f32,
	/// The optional features built into this project's shaders, the same ones BELD baked the material shaders with. A
	/// left-out feature creates no pass, and runtime configuration cannot turn it on.
	features: VisibilityFeatures,
}

impl Default for VisibilityPipelineSettings {
	fn default() -> Self {
		Self {
			geometry_capacity: GeometryCapacity::default(),
			instance_capacity: InstanceCapacity::default(),
			instance_rebuild_lanes: None,
			shadow_map_budget_mib: DEFAULT_SHADOW_MAP_BUDGET_MIB,
			shadow_update_texels: DEFAULT_SHADOW_UPDATE_TEXELS,
			cascade_splits: CascadeSplits::default(),
			cascade_fitting: CascadeFitting::default(),
			directional_shadow_map_resolution: DEFAULT_SHADOW_MAP_RESOLUTION,
			directional_shadow_technique: DirectionalShadowTechnique::default(),
			virtual_shadows: VirtualShadowSettings::default(),
			surfel_capacity: DEFAULT_SURFEL_CAPACITY,
			surfel_ray_budget: DEFAULT_SURFEL_RAY_BUDGET,
			probe_ray_budget: DEFAULT_PROBE_RAY_BUDGET,
			probe_spacing: DEFAULT_PROBE_SPACING,
			features: VisibilityFeatures::default(),
		}
	}
}

impl VisibilityPipelineSettings {
	/// Reads the settings from the `render` subtree of the startup parameters, such as
	/// `app.parameters().scope("render")`, panicking on any value it cannot use.
	///
	/// Every setting rejects a value it cannot parse or use instead of silently keeping its default. The features read
	/// here are the ones BELD baked from the same parameters, so the runtime creates exactly the passes the shaders
	/// expect.
	pub fn from_parameters(parameters: impl Parameters) -> Self {
		fn parse<T: std::str::FromStr>(parameters: &impl Parameters, name: &str) -> Option<T>
		where
			T::Err: std::fmt::Display,
		{
			parameters
				.get_parameter(name)
				.map(|parameter| parameter.parse().unwrap_or_else(|error| panic!("{error}")))
		}
		let mut settings = Self::default();
		if let Some(mebibytes) = parse(&parameters, SHADOW_MAP_BUDGET_PARAMETER) {
			settings.shadow_map_budget_mib = mebibytes;
		}
		if let Some(texels) = parse(&parameters, SHADOW_UPDATE_TEXELS_PARAMETER) {
			settings.shadow_update_texels = texels;
		}
		// Geometry capacity: each parameter overrides one scene-wide geometry buffer's element count.
		let geometry_capacity = &mut settings.geometry_capacity;
		for (stream, capacity) in [
			("vertex", &mut geometry_capacity.vertices),
			("vertex-index", &mut geometry_capacity.vertex_indices),
			("triangle", &mut geometry_capacity.triangles),
			("meshlet", &mut geometry_capacity.meshlets),
			("level", &mut geometry_capacity.levels),
			("skinning-vertex", &mut geometry_capacity.skinning_vertices),
		] {
			if let Some(value) = parse(&parameters, &format!("{GEOMETRY_CAPACITY_PARAMETER_PREFIX}{stream}-capacity")) {
				*capacity = value;
			}
		}
		if let Some(kept) = parse(&parameters, MESHLET_VERTICES_PARAMETER) {
			geometry_capacity.meshlet_vertices = kept;
		}
		let instance_capacity = &mut settings.instance_capacity;
		for (table, capacity) in [
			("capacity", &mut instance_capacity.instances),
			("meshlet-group-capacity", &mut instance_capacity.meshlet_groups),
		] {
			if let Some(value) = parse(&parameters, &format!("{INSTANCE_CAPACITY_PARAMETER_PREFIX}{table}")) {
				*capacity = value;
			}
		}
		settings.instance_rebuild_lanes = parse(&parameters, INSTANCE_REBUILD_LANES_PARAMETER);
		// Directional shadow coverage: each parameter overrides one part of the default splits.
		let default_splits = settings.cascade_splits;
		settings.cascade_splits = CascadeSplits::new(
			parse(&parameters, DIRECTIONAL_SHADOW_DISTANCE_PARAMETER).unwrap_or(default_splits.distance()),
			parse(&parameters, DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER).unwrap_or(default_splits.logarithmic_share()),
		)
		.unwrap_or_else(|reason| panic!("{reason}"));
		if let Some(fitting) = parse(&parameters, DIRECTIONAL_SHADOW_FITTING_PARAMETER) {
			settings.cascade_fitting = fitting;
		}
		if let Some(resolution) = parse(&parameters, DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER) {
			settings.directional_shadow_map_resolution = resolution;
		}
		if let Some(technique) = parse(&parameters, DIRECTIONAL_SHADOW_TECHNIQUE_PARAMETER) {
			settings.directional_shadow_technique = technique;
		}
		// Virtual shadow map sizing: each parameter overrides one part of the defaults.
		let virtual_defaults = settings.virtual_shadows;
		settings.virtual_shadows = VirtualShadowSettings::new(
			parse(&parameters, VIRTUAL_SHADOW_LEVEL_SIZE_PARAMETER).unwrap_or(virtual_defaults.first_level_size()),
			parse(&parameters, VIRTUAL_SHADOW_LEVEL_BIAS_PARAMETER).unwrap_or(virtual_defaults.level_bias()),
			parse(&parameters, VIRTUAL_SHADOW_POOL_PAGES_PARAMETER).unwrap_or(virtual_defaults.pool_pages()),
		)
		.unwrap_or_else(|reason| panic!("{reason}"));
		if let Some(capacity) = parse(&parameters, SURFEL_CAPACITY_PARAMETER) {
			settings.surfel_capacity = capacity;
		}
		if let Some(rays) = parse(&parameters, SURFEL_RAY_BUDGET_PARAMETER) {
			settings.surfel_ray_budget = rays;
		}
		if let Some(rays) = parse(&parameters, PROBE_RAY_BUDGET_PARAMETER) {
			settings.probe_ray_budget = rays;
		}
		if let Some(spacing) = parse(&parameters, PROBE_SPACING_PARAMETER) {
			settings.probe_spacing = spacing;
		}
		settings.features = VisibilityFeatures::from_parameters(parameters);
		settings.validate().unwrap_or_else(|reason| panic!("{reason}"));
		settings
	}

	/// Checks every value the defaults or the parameters can't hold on their own, and explains the first one out of
	/// range.
	fn validate(&self) -> Result<(), String> {
		self.geometry_capacity.validate()?;
		self.instance_capacity.validate()?;
		let resolution = self.directional_shadow_map_resolution;
		let (capacity, rays, probe_rays, spacing) = (
			self.surfel_capacity,
			self.surfel_ray_budget,
			self.probe_ray_budget,
			self.probe_spacing,
		);
		let rejection = if self.shadow_map_budget_mib == 0 {
			"Shadow map budget was not set. The most likely cause is a budget of 0 MiB, which holds no shadow map.".to_owned()
		} else if self.shadow_update_texels == 0 {
			"Shadow update budget was not set. The most likely cause is a budget of 0 texels, which draws no shadow map."
				.to_owned()
		} else if resolution == 0 || !resolution.is_multiple_of(DIRECTIONAL_SHADOW_RESOLUTION_MULTIPLE) {
			format!(
				"Directional shadow resolution was not set. The most likely cause is that {resolution} is not a positive multiple of {DIRECTIONAL_SHADOW_RESOLUTION_MULTIPLE} texels."
			)
		} else if !capacity.is_power_of_two() || !SURFEL_CAPACITY_RANGE.contains(&capacity) {
			format!(
				"Surfel capacity was not set. The most likely cause is that {capacity} is not a power of two from {} through {}. Set `{RENDER_PARAMETERS}.{SURFEL_CAPACITY_PARAMETER}` to a power of two in that range, such as {DEFAULT_SURFEL_CAPACITY}.",
				SURFEL_CAPACITY_RANGE.start(),
				SURFEL_CAPACITY_RANGE.end(),
			)
		} else if rays == 0 || !rays.is_multiple_of(SURFEL_RAY_BUDGET_STEP) || rays > MAX_SURFEL_RAY_BUDGET {
			format!(
				"Surfel ray budget was not set. The most likely cause is that {rays} is not a multiple of {SURFEL_RAY_BUDGET_STEP} from {SURFEL_RAY_BUDGET_STEP} through {MAX_SURFEL_RAY_BUDGET}. Set `{RENDER_PARAMETERS}.{SURFEL_RAY_BUDGET_PARAMETER}` to a multiple of {SURFEL_RAY_BUDGET_STEP} in that range, such as {DEFAULT_SURFEL_RAY_BUDGET}."
			)
		} else if !probe_rays.is_multiple_of(PROBE_RAYS) || !PROBE_RAY_BUDGET_RANGE.contains(&probe_rays) {
			format!(
				"Probe ray budget was not set. The most likely cause is that {probe_rays} is not a multiple of {PROBE_RAYS} from {} through {}. Set `{RENDER_PARAMETERS}.{PROBE_RAY_BUDGET_PARAMETER}` to a multiple of {PROBE_RAYS} in that range, such as {DEFAULT_PROBE_RAY_BUDGET}.",
				PROBE_RAY_BUDGET_RANGE.start(),
				PROBE_RAY_BUDGET_RANGE.end(),
			)
		} else if !PROBE_SPACING_RANGE.contains(&spacing) {
			format!(
				"Probe spacing was not set. The most likely cause is that {spacing} meters is outside {} through {} meters. Set `{RENDER_PARAMETERS}.{PROBE_SPACING_PARAMETER}` to a distance in that range, such as {DEFAULT_PROBE_SPACING}.",
				PROBE_SPACING_RANGE.start(),
				PROBE_SPACING_RANGE.end(),
			)
		} else {
			return Ok(());
		};
		Err(rejection)
	}

	/// Returns how many vertices, triangles, and meshlets the scene-wide geometry buffers hold.
	pub fn geometry_capacity(&self) -> GeometryCapacity {
		self.geometry_capacity
	}

	/// Returns the optional features built into this project's shaders.
	pub fn features(&self) -> VisibilityFeatures {
		self.features
	}

	/// Returns the most lanes that rebuild the frame's instances; see [`INSTANCE_REBUILD_LANES_PARAMETER`].
	fn instance_rebuild_lanes(&self) -> usize {
		self.instance_rebuild_lanes
			.map_or_else(crate::core::machine::performance_parallelism, std::num::NonZeroUsize::get)
	}

	/// Returns what one light of each kind costs from the shadow-map budget.
	fn shadow_budget(&self) -> ShadowBudget {
		ShadowBudget::new(
			self.shadow_map_budget_mib,
			self.directional_shadow_map_resolution,
			self.directional_shadow_technique,
			self.features.shadowed_suns,
		)
	}
}

/// The `Configured` struct holds one system's runtime settings with the configuration port that changes them, so the
/// manager declares and drains every port the same way.
struct Configured<S: 'static> {
	port: ConfigurationPort,
	table: &'static [Setting<S>],
	settings: S,
}

impl<S: Copy> Configured<S> {
	/// Declares `settings`, as `table` names them, on `port`. Next, call [`Self::drain`] each frame.
	fn new(port: ConfigurationPort, table: &'static [Setting<S>], settings: S) -> Self {
		port.declare(table.iter().map(|setting| setting.parameter(settings)));
		Self { port, table, settings }
	}

	/// Applies the updates the port received since the last drain.
	fn drain(&mut self) {
		self.port.drain(&mut self.settings, self.table);
	}
}

/// Keys of the readiness graph: a renderable is ready when its materials are, a material when its textures are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Availability {
	Renderable(Handle),
	Material(u32),
	Texture(u32),
}

/// One material's render-thread pipeline and authored alpha and sidedness contract.
struct LoadedMaterial {
	index: u32,
	evaluation: MaterialEvaluator<ghi::PipelineHandle>,
	/// Shared with the material lists, so rebuilding them does not copy names.
	name: Arc<str>,
	alpha_mode: AlphaMode,
	double_sided: bool,
	/// The material's key in the coverage shaders, when it is masked.
	coverage_arm: Option<u32>,
	texture_indices: Vec<u32>,
	/// Whether the material's graph reads the opaque scene behind its surface.
	reads_backdrop: bool,
}

/// The `UberShader` struct is one phase's compiled uber material evaluation shader, which evaluates the materials whose
/// arm keys it holds.
struct UberShader {
	/// The keys of its arms, in ascending order.
	keys: Box<[u32]>,
	pipeline: ghi::PipelineHandle,
	/// The name of its dispatch in the material lists, made once so rebuilding the lists does not allocate it.
	name: Arc<str>,
}

impl ResidentEnvironment {
	/// Returns the writes that bind this environment's maps in a sink's material evaluation set.
	fn descriptor_writes(self, set: SetBinder<'_>) -> [ghi::DescriptorWrite; 2] {
		[
			set.sampled("environment_irradiance", self.irradiance_image, self.sampler),
			set.sampled("environment_specular", self.specular_image, self.sampler),
		]
	}
}

/// Creates the opaque black environment sampled while no HDR environment is configured or its upload is pending.
pub(crate) fn create_fallback_environment(
	context: &mut ghi::implementation::Context,
	queue: ghi::QueueHandle,
	sampler: ghi::SamplerHandle,
) -> ResidentEnvironment {
	// Opaque alpha keeps material evaluation on this black environment instead of the analytical fallback reserved for
	// explicitly transparent environment texels.
	const FACE: [u8; 4] = [0, 0, 0, u8::MAX];
	let image = create_lookup_texture(
		context,
		queue,
		ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image)
			.name("Visibility Environment Fallback")
			.extent(Extent::square(1))
			// Materials sample the environment as a cubemap.
			.cube_compatible(),
		[FACE; 6].as_flattened(),
	);
	ResidentEnvironment {
		irradiance_image: image.into(),
		specular_image: image.into(),
		sampler,
		upward_illuminance: 0.0,
	}
}

/// The `EnvironmentState` struct keeps the environment the scene selected and the one bound, which shows the selected
/// one once its upload finishes.
struct EnvironmentState {
	requested: Option<RequestedEnvironment>,
	bound: ResidentEnvironment,
	/// The lux `bound` should deliver to an upward-facing surface, if its request was calibrated. It changes only with
	/// `bound`, so a selected environment's lux never scales the map of the one it replaces.
	illuminance: Option<f32>,
	/// The bound environment changed and existing sinks must be rewritten.
	descriptors_dirty: bool,
}

/// The `RequestedEnvironment` struct is an environment the scene selected and the lux it should deliver once bound.
struct RequestedEnvironment {
	id: String,
	illuminance: Option<f32>,
}

impl EnvironmentState {
	/// Binds the resident maps of the requested environment, with the lux it was requested with.
	fn bind(&mut self, environment: ResidentEnvironment) {
		self.bound = environment;
		self.illuminance = self.requested.as_ref().and_then(|requested| requested.illuminance);
		self.descriptors_dirty = true;
	}

	/// Returns the factor material evaluation applies to the bound environment map, which makes it deliver the
	/// requested lux to an upward-facing surface.
	///
	/// Without a request, or for a black map that can't be scaled, the map's own values are used unchanged.
	fn intensity(&self) -> f32 {
		match self.illuminance {
			Some(lux) if self.bound.upward_illuminance > 0.0 => lux / self.bound.upward_illuminance,
			_ => 1.0,
		}
	}
}

/// The `SkinningFrame` struct accumulates this frame's palettes without allocating after the scene's high-water mark.
#[derive(Default)]
struct SkinningFrame {
	matrices: Vec<math::AffineMatrix>,
	dual_quaternions: Vec<DualQuaternion>,
	/// The palette uploaded this frame for each renderable, mesh, and skin index into that mesh's bindings, shared by
	/// every primitive that uses it.
	cache: HashMap<(Handle, MeshKey, u32), (u32, SkinningPaletteKind)>,
}

impl SkinningFrame {
	/// Frame caches retain capacity but never retain entity or resource pointers beyond one rebuild.
	fn clear(&mut self) {
		self.matrices.clear();
		self.dual_quaternions.clear();
		self.cache.clear();
	}

	/// Returns the palette range for one renderable's binding, uploading it once per frame. `key` names the binding as
	/// its mesh and its index in that mesh's bindings, and `binding` is that binding.
	///
	/// Rigid poses are converted to dual quaternions; everything else keeps the matrix palette.
	fn palette(
		&mut self,
		handle: Handle,
		key: (MeshKey, u32),
		binding: &SkinBinding,
		pose: &[math::AffineMatrix],
	) -> Option<(u32, SkinningPaletteKind)> {
		let cache_key = (handle, key.0, key.1);
		if let Some(palette) = self.cache.get(&cache_key) {
			return Some(*palette);
		}
		let matrix_base = self.matrices.len();
		let matrix_end = matrix_base + binding.entries.len();
		self.matrices.resize(matrix_end, math::AffineMatrix::identity());
		if let Err(error) = binding.write_matrix_palette(pose, &mut self.matrices[matrix_base..matrix_end]) {
			self.matrices.truncate(matrix_base);
			error!("Visibility skin palette could not be written: {error}");
			return None;
		}
		let dual_quaternion_base = self.dual_quaternions.len();
		let (palette_base, kind, used) =
			if append_dual_quaternion_palette(&self.matrices[matrix_base..matrix_end], &mut self.dual_quaternions) {
				self.matrices.truncate(matrix_base);
				(
					dual_quaternion_base,
					SkinningPaletteKind::DualQuaternion,
					self.dual_quaternions.len(),
				)
			} else {
				(matrix_base, SkinningPaletteKind::Matrix, matrix_end)
			};
		assert!(
			used <= MAX_SKINNING_MATRICES,
			"Visibility skin palette limit exceeded. The most likely cause is that active skins require more joint transforms than the visibility pipeline supports."
		);
		self.cache.insert(cache_key, (palette_base as u32, kind));
		Some((palette_base as u32, kind))
	}
}

/// Resolves one light's IES profile to its intensity scale and, once resident, its texture with the dimmer applied.
///
/// Analytic lights scale by one. A profile light scales by its dimmer while its texture is pending, then by the
/// texture's dimmed calibrated candela scale.
fn resolve_ies_profile(light: &Lights, profiles: &HashMap<String, IesProfileTexture>) -> (f32, Option<IesProfileTexture>) {
	let Some(profile) = light.local().and_then(LocalEmission::ies_profile) else {
		return (1.0, None);
	};
	match profiles.get(profile.resource_id()) {
		Some(texture) => {
			let texture = IesProfileTexture {
				intensity_scale_candela: texture.intensity_scale_candela * profile.dimmer(),
				..*texture
			};
			(texture.intensity_scale_candela, Some(texture))
		}
		None => (profile.dimmer(), None),
	}
}

/// Returns whether a runtime-enabled feature can run: a feature the project left out of its shaders stays off, with a
/// warning, because `enabled` was turned on after startup.
fn built_in_or_off(enabled: bool, built_in: bool, parameter: &str) -> bool {
	if enabled && !built_in {
		log::warn!(
			"`{RENDER_PARAMETERS}.{parameter}` was not enabled. The most likely cause is that it was `false` at startup, so its feature was left out of the project's shaders. Set it to `true` in `config.json`, then bake again with `beld bake`. See {}.",
			crate::online_docs_url("use/resources/bake#leave-render-features-out-of-the-bake")
		);
	}
	enabled && built_in
}

/// The frames between two logs of the surfel pool's counts, of the probe volume's, and of the kept meshlets.
const GI_LOG_INTERVAL: u64 = 256;

/// Logs how many meshlets the task stage kept for each view of `sink` in one frame, as `render.lod.count-meshlets`
/// asks: the camera's, the shadow views' together, and each shadow view that kept any, by its index in the scene's
/// views.
fn log_kept_meshlets(sink: usize, kept: &[u32]) {
	use std::fmt::Write as _;

	let (camera, shadows) = (kept[0], &kept[1..]);
	let mut views = String::new();
	for (view, count) in shadows.iter().enumerate().filter(|(_, count)| **count > 0) {
		// Writing to a string can't fail.
		let _ = write!(views, " view {} kept {count};", view + 1);
	}
	log::info!(
		"Sink {sink} kept {camera} meshlets for its camera and {} for its shadow views.{views}",
		shadows.iter().map(|count| u64::from(*count)).sum::<u64>()
	);
}

/// The debug views of a sink a resolve can light: material evaluation's, then the resolve's two, which the resolve
/// writes from the range after material evaluation's.
type ResolveDebugViewNames = [&'static str; super::shader_generator::MATERIAL_DEBUG_VIEW_NAMES.len() + 2];

/// Returns material evaluation's debug view names followed by a resolve's `views`.
const fn resolve_debug_view_names(views: [&'static str; 2]) -> ResolveDebugViewNames {
	let material = super::shader_generator::MATERIAL_DEBUG_VIEW_NAMES;
	let mut names = [""; super::shader_generator::MATERIAL_DEBUG_VIEW_NAMES.len() + 2];
	let mut index = 0;
	while index < material.len() {
		names[index] = material[index];
		index += 1;
	}
	names[index] = views[0];
	names[index + 1] = views[1];
	names
}

/// The debug views of a sink that surfel GI can light.
const SURFEL_DEBUG_VIEW_NAMES: ResolveDebugViewNames = resolve_debug_view_names(["surfels", "surfel-coverage"]);
/// The debug views of a sink that probe GI, and not surfel GI, can light.
const PROBE_DEBUG_VIEW_NAMES: ResolveDebugViewNames = resolve_debug_view_names(["probes", "probe-light"]);

/// Chooses the source that lights this frame's indirect diffuse light, or `None` when the environment lights it alone.
///
/// `gi` selects the technique, which lights the frame only when `features` built it in. Surfel GI and probe GI need a
/// device that traces rays, `ray_tracing`; elsewhere SSGI lights frames in their place when it is built in, so selecting
/// either never takes away the light SSGI gives.
fn indirect_diffuse_source(gi: GiSettings, features: VisibilityFeatures, ray_tracing: bool) -> Option<IndirectDiffuseSource> {
	if !gi.enabled {
		return None;
	}
	let technique = match gi.technique {
		technique if technique.traces_rays() && !ray_tracing => IndirectDiffuseSource::ScreenSpace,
		technique => technique,
	};
	technique.built_in(features).then_some(technique)
}

/// The `VisibilityPipelineManager` struct provides the visibility-buffer implementation of the world render domain.
///
/// Register it through [`crate::rendering::Renderer::add_pipeline_manager`]. Scene changes arrive through the
/// world listeners it subscribes to when created; frames flow through the [`PipelineManager`] callbacks.
pub struct VisibilityPipelineManager {
	/// Domain façade for requesting resources and consuming readiness changes.
	loader: VisibilityLoaderClient,
	/// Retained storage for readiness events awaiting renderer adoption.
	resource_events: Vec<VisibilityLoaderEvent>,
	pipeline_manager: PipelineManagerClient,
	/// World creation messages adopted by [`PipelineManager::update`].
	resource_listener: DefaultListener<CreateMessage<Resource>>,
	cone_light_listener: DefaultListener<CreateMessage<ConeLight>>,
	directional_light_listener: DefaultListener<CreateMessage<DirectionalLight>>,
	point_light_listener: DefaultListener<CreateMessage<PointLight>>,
	mesh_listener: DefaultListener<CreateMessage<RenderableMesh>>,
	terrain_listener: DefaultListener<CreateMessage<RenderableTerrain>>,
	/// Every renderable terrain, the nodes each one draws, and when each node mesh was last needed.
	terrains: TerrainStreams,
	/// The scene changes terrains asked for, kept so frames reuse the room.
	terrain_changes: Vec<TerrainChange>,
	/// The node meshes terrains asked for this frame, kept so frames reuse the room.
	terrain_requests: Vec<crate::rendering::renderable::mesh::MeshSource>,
	environment_listener: DefaultListener<CreateMessage<Environment>>,
	lightmap_listener: DefaultListener<CreateMessage<Lightmap>>,
	/// The screen scene queries the world creates, which [`ScreenQueries`] keeps until a completed frame answers them.
	screen_query_listener: DefaultListener<CreateMessage<crate::rendering::ScreenSceneQuery>>,
	screen_queries: ScreenQueries,
	/// Where the answers to screen scene queries are created.
	screen_query_hits: Factory<crate::rendering::ScreenSceneHit>,
	/// The lightmap the world names, and where each lit primitive lies in it once it loaded.
	lightmaps: Lightmaps,
	fog_listener: DefaultListener<CreateMessage<crate::rendering::ExponentialHeightFog>>,
	/// The newest height fog, which each sink's fog light march lights.
	fog: Option<crate::rendering::ExponentialHeightFog>,
	/// World edits adopted at the start of [`PipelineManager::prepare`].
	pose_listener: DefaultListener<UpdatePose>,
	material_parameter_listener: DefaultListener<SetMaterialParameter>,
	/// Canonical dynamic material parameter values, copied into each frame-local buffer after they change.
	material_parameters: MaterialParameters,
	material_parameters_buffer: ghi::BufferHandle<[[f32; 4]; MAX_MATERIAL_PARAMETERS]>,
	deletions_listener: DefaultListener<DeleteMessage>,
	/// Canonical material table, copied into each frame-local buffer after it changes.
	materials: Box<[MaterialData; MAX_MATERIALS]>,
	materials_buffer: ghi::BufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Canonical evaluation slot of every material table entry, which [`Self::rebuild_material_lists`] assigns. It is
	/// copied into each frame-local buffer together with `materials`.
	material_evaluations: Box<[u32; MAX_MATERIALS]>,
	material_evaluations_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	/// Which frame sequences' copies of the material and evaluation buffers hold the current tables. Every frame
	/// sequence keeps its own copy, so a change reaches each copy on a frame of that copy's sequence.
	materials_copies_current: [bool; ghi::MAX_FRAMES_IN_FLIGHT],
	/// The instance table, which every frame sequence's copy is brought up to date from. Keeping it lets a frame patch
	/// the instances that changed instead of writing them all.
	instance_records: Vec<ShaderMesh>,
	/// What the instances were rebuilt with, which following the camera reads; see [`super::instance_rebuild`].
	instance_kept: Vec<Kept>,
	/// Whether something the instances are rebuilt from changed, other than where the camera is: the scene, the
	/// loaded resources, the materials, or the detail view's error scale. The next frame rebuilds them.
	instances_stale: bool,
	/// Whether a primitive moved, or its mesh's resident detail levels changed, since the instances were last brought
	/// up to date. The next frame follows them, which picks those primitives again.
	instances_moved: bool,
	/// The mesh primitives whose resident detail levels changed this frame, kept across frames to reuse its room.
	refreshed_lods: HashSet<(MeshKey, u32)>,
	/// The slots of the instance table that changed since each frame sequence's copy was last brought up to date.
	stale_records: [std::ops::Range<usize>; ghi::MAX_FRAMES_IN_FLIGHT],
	/// Which frame sequences' copies of the phases' instance lists are older than the lists.
	stale_work: [bool; ghi::MAX_FRAMES_IN_FLIGHT],
	/// The detail view the instances were last rebuilt or followed for.
	instances_lod_view: Option<LodView>,
	/// The work ranges of the current instances.
	mesh_work: MeshWork,
	skinning_pass: SkinningPass,
	skinning_frame: SkinningFrame,
	/// The resident streams, whose vertex ranges posed skinned instances take and give back.
	geometry: GeometryHandles,
	/// Whether the last frame recorded the pending UV copies of new posed ranges, so the next one drops them.
	posed_uv_copies_recorded: bool,
	/// Adopted materials indexed by material table slot.
	loaded_materials: Vec<Option<LoadedMaterial>>,
	/// Each uber material evaluation shader, in the order of [`UberVariant::ALL`], once one compiled. Only the uber
	/// material evaluation mode loads them.
	uber_shaders: [Option<UberShader>; 4],
	/// The variants whose uber shader is loading, or baking again, so materials it lacks wait for it instead of starting
	/// another bake.
	uber_shaders_loading: [bool; 4],
	/// The arm keys of the published coverage shaders, in ascending order, once they compiled.
	coverage_keys: Option<Box<[u32]>>,
	/// Whether the coverage shaders are loading, or baking again, so masked materials they lack wait for them instead of
	/// starting another bake.
	coverage_shaders_loading: bool,
	/// Calibrated IES profile textures that completed their GPU upload, keyed by resource ID.
	loaded_ies_profiles: HashMap<String, IesProfileTexture>,
	availability: AvailabilityGraph<Availability>,
	environment: EnvironmentState,
	/// The startup limits and shadow coverage.
	settings: VisibilityPipelineSettings,
	/// The shadow maps every sink samples. The first recorded sink renders them each frame.
	shadow_maps: ShadowMaps,
	/// How many maps of each kind the shadow-map images hold, kept across frames by [`retain_layout`].
	shadow_layout: ShadowLayout,
	/// Where each shadowed cone and point light's map lived last frame, which keeps its size class steady while its
	/// span hovers at a power of two, and its layer while it keeps its class.
	shadow_slots: ShadowSlots,
	/// What every shadow map was drawn with, so maps that don't draw this frame keep their view.
	shadow_states: ShadowMapStates,
	/// The virtual shadow map's level placement the GPU last recorded, which the next placement scrolls from.
	virtual_shadow_levels: Option<VirtualShadowLevels>,
	/// The instance records that changed since the virtual shadow map last recorded, and how many records there were
	/// then, so its invalidation also sees the records that left.
	virtual_shadow_changes: std::ops::Range<usize>,
	virtual_shadow_records: usize,
	gtao: Configured<GtaoSettings>,
	gi: Configured<GiSettings>,
	surfel_gi: Configured<SurfelGiSettings>,
	probe_gi: Configured<ProbeGiSettings>,
	/// Whether the device traces rays, which surfel GI and probe GI need to light a frame.
	ray_tracing: bool,
	/// The surfels that surfel GI lights every sink from. Absent when the project left surfel GI out of its shaders or
	/// the device doesn't trace rays.
	surfel_pool: Option<SurfelPool>,
	/// The probes that probe GI lights every sink from. Absent when the project left probe GI out of its shaders or
	/// the device doesn't trace rays.
	probe_volume: Option<ProbeVolume>,
	/// The acceleration structures surfel GI and probe GI trace rays through. Absent without a surfel pool or a probe
	/// volume.
	acceleration_structures: Option<SceneAccelerationStructures>,
	/// The hits of the rays of whichever of surfel GI and probe GI lights the frame. Absent without a surfel pool or a
	/// probe volume.
	ray_hits: Option<RayHits>,
	/// The ray tracing pipeline that shades surfel hits with each material's closest-hit shader. Absent without
	/// acceleration structures or a surfel pool.
	surfel_shading: Option<HitShading>,
	/// The ray tracing pipeline that shades probe hits. Absent without acceleration structures or a probe volume.
	probe_shading: Option<HitShading>,
	/// The primitives the frame draws, as the acceleration structures place them. Kept across frames for its
	/// allocation.
	traced_primitives: Vec<TracedPrimitive>,
	/// What each instance rebuild lane lists, kept so frames reuse the room.
	rebuild_scratch: Vec<LaneScratch>,
	/// Whether the warning that a ray-traced GI technique can't light frames on this device was reported while one stays
	/// selected.
	reported_surfel_gi_fallback: bool,
	contact_shadow: Configured<ContactShadowSettings>,
	lod: Configured<LodSettings>,
	/// Decides which meshes load finer detail levels and whose finer levels to release for room.
	mesh_residencies: MeshResidencies,
	/// The sinks whose visibility pass recorded in the previous frame, with the view and extent they used. Only their
	/// per-frame images hold usable history, and only while the extent is unchanged.
	recorded_sinks: SmallVec<[Sink; 4]>,
	/// The exposure the previous frame's light was multiplied by. Every recorded sink shares it.
	recorded_exposure: f32,
	/// The exposure the surfel pool's light was stored at by the last frame surfel GI lit.
	surfel_exposure: f32,
	/// The source that lit the previous frame's indirect diffuse light in every recorded sink, which decides whose
	/// history their images hold.
	recorded_indirect_diffuse: Option<IndirectDiffuseSource>,
	/// Whether the light-count and shadow-budget warnings were reported while their limit stays exceeded.
	reported_limits: [bool; 3],
	pub(crate) scene: VisibilityScene,
	/// The blue noise every sink's screen-space passes draw their per-pixel random numbers from.
	blue_noise: ghi::ImageHandle,
}

impl VisibilityPipelineManager {
	/// Creates the scene buffers and base descriptor set around an already running resource client.
	///
	/// The manager subscribes to `world`'s resources, lights, meshes, environments, poses, transforms, and
	/// deletions here, so it sees every message published after setup.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		queue: ghi::QueueHandle,
		world: &MessageScope,
		geometry: GeometryHandles,
		mut loader: VisibilityLoaderClient,
		pipeline_manager: PipelineManagerClient,
		gtao_configuration: ConfigurationPort,
		gi_configuration: ConfigurationPort,
		surfel_gi_configuration: ConfigurationPort,
		probe_gi_configuration: ConfigurationPort,
		contact_shadow_configuration: ConfigurationPort,
		lod_configuration: ConfigurationPort,
		settings: VisibilityPipelineSettings,
	) -> Self {
		let gtao = Configured::new(gtao_configuration, GtaoSettings::SETTINGS, GtaoSettings::default());
		let gi = Configured::new(gi_configuration, GiSettings::SETTINGS, GiSettings::new(settings.features));
		let surfel_gi = Configured::new(
			surfel_gi_configuration,
			SurfelGiSettings::SETTINGS,
			SurfelGiSettings::new(
				settings.features.surfel_gi,
				settings.surfel_capacity,
				settings.surfel_ray_budget,
			),
		);
		let probe_gi = Configured::new(
			probe_gi_configuration,
			ProbeGiSettings::SETTINGS,
			ProbeGiSettings::new(settings.features.probe_gi, settings.probe_ray_budget, settings.probe_spacing),
		);
		let contact_shadow = Configured::new(
			contact_shadow_configuration,
			ContactShadowSettings::SETTINGS,
			ContactShadowSettings::default(),
		);
		let lod = Configured::new(lod_configuration, LodSettings::SETTINGS, LodSettings::default());
		let environment = create_fallback_environment(context, queue, loader.environment_sampler);
		let blue_noise = create_blue_noise_texture(context, queue);
		if settings.features.material_evaluation == MaterialEvaluationMode::Uber {
			loader.request_uber_shaders();
		}
		// Every mode tests masked materials in the coverage shaders.
		loader.request_coverage_shaders();
		let skinning_pass = SkinningPass::new(context, &pipeline_manager, &geometry);
		// The CPU writes each table into each frame's copy; the ones it copies whole are transfer destinations too.
		let host_buffer = |name, uses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage | uses)
				.name(name)
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame)
		};
		let copied = ghi::Uses::TransferDestination;
		let materials_buffer = context.build_buffer(host_buffer("Materials Data", copied));
		let material_evaluations_buffer = context.build_buffer(host_buffer("Material Evaluations", copied));
		let material_parameters_buffer = context.build_buffer(host_buffer("Material Parameters", copied));
		let views_buffer = context.build_buffer(host_buffer("Visibility Views Data", ghi::Uses::empty()));
		let frame_buffer = context.build_buffer(host_buffer("Visibility Frame Data", ghi::Uses::empty()));
		let instances = settings.instance_capacity.instances as usize;
		let meshes_buffer = context.build_buffer(host_buffer("Visibility Meshes Data", ghi::Uses::empty()).length(instances));
		let lighting_data = context.build_buffer(host_buffer("Lighting Data", copied));
		let scene_layout = set_layouts::layout(set_layouts::SCENE);
		let descriptor_set = scene_layout.create_set(context, "Scene Descriptor Set");
		let mesh_work = MeshWork::new(
			context,
			&pipeline_manager,
			scene_layout.binder(descriptor_set),
			meshes_buffer,
			geometry.mesh_levels,
			settings.instance_capacity.instances,
			settings.instance_capacity.meshlet_groups,
		);
		let shadow_maps = ShadowMaps::new(
			context,
			&pipeline_manager,
			settings.directional_shadow_map_resolution,
			settings.directional_shadow_technique,
			settings.virtual_shadows,
			settings.instance_capacity.instances,
		);
		let ray_tracing = context.supports_ray_tracing();
		let surfel_pool = (settings.features.surfel_gi && ray_tracing).then(|| {
			SurfelPool::new(
				context,
				&pipeline_manager,
				settings.surfel_capacity,
				settings.surfel_ray_budget,
				environment,
				lighting_data,
			)
		});
		let probe_volume = (settings.features.probe_gi && ray_tracing).then(|| {
			ProbeVolume::new(
				context,
				&pipeline_manager,
				settings.probe_ray_budget,
				settings.probe_spacing,
				environment,
				lighting_data,
			)
		});
		let acceleration_structures = (surfel_pool.is_some() || probe_volume.is_some())
			.then(|| SceneAccelerationStructures::new(context, &geometry, settings.instance_capacity.instances))
			.flatten();
		// Only one technique lights a frame, so they share room for the hits of the larger ray budget.
		let ray_hits = [
			surfel_pool.as_ref().map(|_| settings.surfel_ray_budget),
			probe_volume.as_ref().map(|_| settings.probe_ray_budget),
		]
		.into_iter()
		.flatten()
		.max()
		.map(|ray_budget| RayHits::new(context, &pipeline_manager, ray_budget));
		let surfel_shading = acceleration_structures
			.as_ref()
			.filter(|_| surfel_pool.is_some())
			.map(|_| HitShading::new(context, SURFEL_SHADE_SHADER, "surfel GI"));
		let probe_shading = acceleration_structures
			.as_ref()
			.filter(|_| probe_volume.is_some())
			.map(|_| HitShading::new(context, PROBE_SHADE_SHADER, "probe GI"));
		let scene = scene_layout.binder(descriptor_set);
		context.write(&[
			scene.buffer("views", views_buffer),
			scene.buffer("frame", frame_buffer),
			scene.texture("blue_noise", blue_noise),
			scene.buffer("meshes", meshes_buffer),
			scene.buffer("vertex_positions", geometry.vertex_positions),
			scene.buffer("vertex_normals", geometry.vertex_normals),
			scene.buffer("vertex_uvs", geometry.vertex_uvs),
			scene.buffer("vertex_indices", geometry.vertex_indices),
			scene.buffer("meshlet_vertices", geometry.meshlet_vertices),
			scene.buffer("primitive_indices", geometry.primitive_indices),
			scene.buffer("meshlets", geometry.meshlets),
			scene.buffer("mesh_levels", geometry.mesh_levels),
			scene.buffer("materials", materials_buffer),
			scene.buffer("material_evaluations", material_evaluations_buffer),
			scene.buffer("material_parameters", material_parameters_buffer),
			scene.buffer("vertex_lightmap_uvs", geometry.vertex_lightmap_uvs),
		]);
		Self {
			blue_noise,
			loader,
			resource_events: Vec::new(),
			pipeline_manager,
			resource_listener: world.factory::<Resource>().listener(),
			cone_light_listener: world.factory::<ConeLight>().listener(),
			directional_light_listener: world.factory::<DirectionalLight>().listener(),
			point_light_listener: world.factory::<PointLight>().listener(),
			mesh_listener: world.factory::<RenderableMesh>().listener(),
			terrain_listener: world.factory::<RenderableTerrain>().listener(),
			terrains: TerrainStreams::default(),
			terrain_changes: Vec::new(),
			terrain_requests: Vec::new(),
			environment_listener: world.factory::<Environment>().listener(),
			lightmap_listener: world.factory::<Lightmap>().listener(),
			screen_query_listener: world.factory::<crate::rendering::ScreenSceneQuery>().listener(),
			screen_queries: ScreenQueries::default(),
			screen_query_hits: world.factory(),
			lightmaps: Lightmaps::default(),
			fog_listener: world.factory::<crate::rendering::ExponentialHeightFog>().listener(),
			fog: None,
			pose_listener: world.channel::<UpdatePose>().listener(),
			material_parameter_listener: world.channel::<SetMaterialParameter>().listener(),
			material_parameters: MaterialParameters::new(),
			material_parameters_buffer,
			deletions_listener: world.channel::<DeleteMessage>().listener(),
			materials: Box::new([MaterialData::default(); MAX_MATERIALS]),
			materials_buffer,
			material_evaluations: Box::new([NO_EVALUATION; MAX_MATERIALS]),
			material_evaluations_buffer,
			// The default tables must reach every frame sequence too.
			materials_copies_current: [false; ghi::MAX_FRAMES_IN_FLIGHT],
			instance_records: vec![bytemuck::Zeroable::zeroed(); settings.instance_capacity.instances as usize],
			instance_kept: Vec::new(),
			instances_stale: true,
			instances_moved: false,
			refreshed_lods: HashSet::default(),
			stale_records: std::array::from_fn(|_| 0..0),
			stale_work: [true; ghi::MAX_FRAMES_IN_FLIGHT],
			instances_lod_view: None,
			mesh_work,
			skinning_pass,
			skinning_frame: SkinningFrame::default(),
			geometry,
			posed_uv_copies_recorded: false,
			loaded_materials: Vec::new(),
			uber_shaders: [None, None, None, None],
			uber_shaders_loading: [settings.features.material_evaluation == MaterialEvaluationMode::Uber; 4],
			coverage_keys: None,
			coverage_shaders_loading: true,
			loaded_ies_profiles: HashMap::default(),
			availability: AvailabilityGraph::with_capacity(
				instances + MAX_MATERIALS + MAX_BINDLESS_TEXTURES,
				instances + MAX_MATERIALS * MAX_MATERIAL_TEXTURES,
			),
			environment: EnvironmentState {
				requested: None,
				bound: environment,
				illuminance: None,
				descriptors_dirty: false,
			},
			settings,
			shadow_maps,
			shadow_layout: ShadowLayout::default(),
			shadow_slots: ShadowSlots::new(),
			shadow_states: ShadowMapStates::default(),
			virtual_shadow_levels: None,
			virtual_shadow_changes: 0..0,
			virtual_shadow_records: 0,
			gtao,
			gi,
			surfel_gi,
			probe_gi,
			ray_tracing,
			surfel_pool,
			probe_volume,
			acceleration_structures,
			ray_hits,
			surfel_shading,
			probe_shading,
			traced_primitives: Vec::new(),
			rebuild_scratch: Vec::new(),
			reported_surfel_gi_fallback: false,
			contact_shadow,
			lod,
			mesh_residencies: MeshResidencies::default(),
			recorded_sinks: SmallVec::new(),
			recorded_exposure: 1.0,
			surfel_exposure: 1.0,
			recorded_indirect_diffuse: None,
			reported_limits: [false; 3],
			scene: VisibilityScene {
				render_entities: StableVec::new(),
				skinning_poses: HashMap::default(),
				renderables: Default::default(),
				lights: StableVec::new(),
				light_handles: HashMap::default(),
				layout: scene_layout,
				descriptor_set,
				views_buffer,
				frame_buffer,
				meshes_buffer,
				lighting_data,
				render_info: Default::default(),
				sink_states: Vec::new(),
			},
		}
	}

	/* Scene changes */

	/// Applies the frame's scene edits before anything is drawn: poses, then the transforms the frame shows, then
	/// deletions.
	fn adopt_scene_updates(&mut self, transforms: &ShownTransforms) {
		// Whether anything the instances are rebuilt from changed.
		let mut changed = false;
		while let Some(message) = self.pose_listener.read() {
			self.scene.write_skinned_pose(message.handle(), message.global_matrices());
			changed = true;
		}
		while let Some(message) = self.material_parameter_listener.read() {
			self.material_parameters.receive(message);
			changed = true;
		}
		// A primitive that moved keeps its place among the instances, so only its own pick is made again.
		let kept = &mut self.instance_kept;
		let mut moved = false;
		for (handle, transform) in transforms.changed() {
			// A terrain's nodes draw at its transform.
			for handle in std::iter::once(*handle).chain(self.terrains.instances(*handle)) {
				self.scene.update_transform(handle, transform, |slot| {
					// A primitive added since the last rebuild has no kept slot yet, and the rebuild it waits for picks it.
					if let Some(kept) = kept.get_mut(slot) {
						kept.release();
					}
					moved = true;
				});
			}
		}
		self.instances_moved |= moved;
		let mut changes = std::mem::take(&mut self.terrain_changes);
		while let Some(message) = self.deletions_listener.read() {
			changed = true;
			let handle = message.into_handle();
			self.terrains.remove(handle, &mut changes);
			self.scene.remove_light(handle);
			// A deleted renderable also drops the material parameters set on it.
			self.remove_mesh_instance(handle);
			self.material_parameters.forget_entity(handle);
			if self.lightmaps.forget(handle) {
				self.refresh_instance_lightmaps();
			}
		}
		self.apply_terrain_changes(&mut changes, transforms);
		self.terrain_changes = changes;
		if changed {
			self.instances_changed();
		}
	}

	/// Records that something the frame's instances are rebuilt from changed, so the next frame rebuilds them. Call
	/// it wherever the scene's entities, their resources, or their materials change.
	fn instances_changed(&mut self) {
		self.instances_stale = true;
	}

	/// Adds one light where its handle is shown and requests its optional photometric texture dependency.
	fn create_light(&mut self, handle: Handle, light: Lights, transforms: &ShownTransforms) {
		if let Some(profile) = light.local().and_then(LocalEmission::ies_profile) {
			self.loader.request_ies_profile(profile.resource_id().to_owned());
		}
		self.scene.add_light(handle, light);
		if let Some(transform) = transforms.get(handle) {
			self.scene.update_transform(handle, transform, |_| {});
		}
	}

	/// Selects an environment and requests its baked lighting resources.
	fn create_environment(&mut self, environment: Environment) {
		let id = environment.resource_id().to_owned();
		self.environment.requested = Some(RequestedEnvironment {
			id: id.clone(),
			illuminance: environment.illuminance(),
		});
		if let Some(resident) = self.loader.request_environment(id) {
			self.environment.bind(resident);
		}
	}

	/// Requests the renderable's mesh and keeps the scene instance pending until the mesh is resident.
	fn request_mesh(&mut self, handle: Handle, renderable: RenderableMesh, transforms: &ShownTransforms) {
		// Creation messages are upserts; the replacement starts where the handle is shown.
		self.remove_mesh_instance(handle);
		let source = renderable.source().clone();
		let mesh_key = self.loader.request_mesh(source);
		if self.loader.mesh(mesh_key).is_some() {
			self.scene.renderables.insert_resident(handle);
			self.add_renderable(handle, mesh_key, model_of(handle, transforms));
		} else {
			self.scene.renderables.wait(handle, mesh_key);
		}
	}

	/// Removes only the lifecycle state this handle owns; new handles need no cleanup.
	fn remove_mesh_instance(&mut self, handle: Handle) {
		let ranges = &self.geometry.ranges;
		// Nothing reads a range after its instance leaves the frame, except frames still in flight, and the backends
		// order the next owner's writes after those reads.
		if !self.scene.remove_renderable(handle, |range| {
			ranges
				.lock()
				.unwrap_or_else(|error| error.into_inner())
				.give_back_vertices(range);
		}) {
			return;
		}
		self.instances_changed();
		self.material_parameters.release_entity(handle);
		self.availability.remove(&Availability::Renderable(handle)).expect(
			"Visibility renderable availability could not be removed. The most likely cause is that another graph node depends on a renderable.",
		);
	}

	/* Resource adoption */

	/// Finishes renderer-specific adoption of loaded resources and publishes only fully usable ones.
	///
	/// `generated_environment` holds the maps of the environment the scene background generates, such as the atmosphere
	/// sky's, once a frame filled them. It lights the scene while the scene selects no [`Environment`].
	fn adopt_resource_completions(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		transforms: &ShownTransforms,
		generated_environment: Option<crate::rendering::EnvironmentMaps>,
	) {
		let mut events = std::mem::take(&mut self.resource_events);
		self.loader.update(frame, &mut events);
		// Every event changes what some instance draws: a mesh, a level, a material, a texture, or a lightmap. A level
		// that arrives or leaves changes only its own primitive's instances, which are picked again without a rebuild.
		if events.iter().any(|event| {
			!matches!(
				event,
				VisibilityLoaderEvent::MeshLevelsLoaded { .. } | VisibilityLoaderEvent::MeshLevelsUnavailable { .. }
			)
		}) {
			self.instances_changed();
		}
		// Material readiness changes rebuild the material lists once, after every event of the frame.
		let mut materials_changed = false;
		for event in events.drain(..) {
			match event {
				VisibilityLoaderEvent::MeshReady { key } => {
					let Some(mesh) = self.loader.mesh(key) else {
						continue;
					};
					// The base block holds each primitive's coarsest level.
					let coarsest = mesh
						.primitives
						.iter()
						.map(|primitive| primitive.lods.levels().len() as u8 - 1);
					self.mesh_residencies.add_mesh(key, coarsest);
					for handle in self.scene.renderables.mesh_ready(key) {
						self.add_renderable(handle, key, model_of(handle, transforms));
					}
				}
				VisibilityLoaderEvent::MeshLevelsLoaded { key, primitives } => {
					for levels in &primitives {
						if levels.block.is_some() {
							self.mesh_residencies.loaded(key, levels.primitive, levels.finest);
							self.refreshed_lods.insert((key, levels.primitive));
						} else if let Some((released, released_primitive)) =
							self.mesh_residencies
								.without_room(key, levels.primitive, ghi::MAX_FRAMES_IN_FLIGHT as u64)
						{
							self.loader.release_mesh_levels(released, released_primitive);
							self.refreshed_lods.insert((released, released_primitive));
						}
					}
					self.mesh_residencies.load_finished(key, primitives);
				}
				VisibilityLoaderEvent::MeshLevelsUnavailable { key, error } => {
					self.mesh_residencies.unavailable(key);
					warn!(
						"Visibility finer detail levels of mesh {key:?} are unavailable; its instances keep their coarser levels. {error}"
					);
				}
				VisibilityLoaderEvent::MaterialReady(material, evaluation) => {
					self.adopt_material(material, evaluation);
					materials_changed = true;
				}
				VisibilityLoaderEvent::UberShaderReady { variant, keys, pipeline } => {
					self.adopt_uber_shader(
						variant,
						UberShader {
							keys,
							pipeline,
							name: if variant.translucent {
								"Translucent Uber Shader".into()
							} else {
								"Uber Shader".into()
							},
						},
					);
					materials_changed = true;
				}
				VisibilityLoaderEvent::CoverageShadersReady { keys, pipelines } => {
					self.scene.render_info.coverage_pipelines = Some(pipelines);
					self.adopt_coverage_shaders(keys);
					materials_changed = true;
				}
				VisibilityLoaderEvent::CoverageShadersUnavailable { error } => {
					self.coverage_shaders_loading = false;
					error!("The coverage shaders of the masked passes are unavailable: {error}");
				}
				VisibilityLoaderEvent::UberShaderUnavailable { variant, error } => {
					self.uber_shaders_loading[variant.index()] = false;
					error!(
						"The uber material evaluation shader {} is unavailable: {error}",
						variant.shader()
					);
				}
				VisibilityLoaderEvent::MaterialUnavailable { index } => {
					self.availability.set_key_available(&Availability::Material(index), false);
					materials_changed = true;
				}
				VisibilityLoaderEvent::TextureReady(texture) => materials_changed |= self.adopt_texture(frame, texture),
				VisibilityLoaderEvent::LightmapReady(lightmap) => {
					let texture = &lightmap.texture;
					frame.write(&[self
						.scene
						.binder()
						.texture_element(texture.array.binding(), texture.image, texture.index)]);
					if self
						.lightmaps
						.adopt(&lightmap.id, lightmap.holder, &lightmap.instances, texture.index)
					{
						self.refresh_instance_lightmaps();
					}
				}
				VisibilityLoaderEvent::EnvironmentReady { id, environment } => {
					if self
						.environment
						.requested
						.as_ref()
						.is_some_and(|requested| requested.id == id)
					{
						self.environment.bind(environment);
					}
				}
				VisibilityLoaderEvent::TerrainReady { id, terrain } => {
					let mut changes = std::mem::take(&mut self.terrain_changes);
					self.terrains.adopt(&id, terrain, &mut changes);
					self.apply_terrain_changes(&mut changes, transforms);
					self.terrain_changes = changes;
				}
				VisibilityLoaderEvent::MeshUnavailable { key, error } => {
					// Nothing will make the mesh resident, so its renderables stop waiting; creating one again requests it again.
					let mut dropped = 0;
					while let Some(handle) = self.scene.renderables.waiter(key) {
						self.remove_mesh_instance(handle);
						dropped += 1;
					}
					warn!(
						"Visibility mesh {key:?} could not be loaded, so its {dropped} renderables are not drawn. The most likely cause is a missing or invalid mesh asset. Fix the asset, then create the renderables again. {error}"
					);
				}
				VisibilityLoaderEvent::Unavailable { resource, error } => {
					warn!("Visibility {resource} is unavailable: {error}");
				}
			}
		}
		self.resource_events = events;
		self.refresh_entity_lods();
		// Every replacement is in its slot now, so nothing binds the images it replaced.
		self.loader.delete_replaced_images(frame);
		if materials_changed {
			self.rebuild_material_lists();
		}
		// The generated environment lights the scene until the scene selects one, from the frame after its first fill, so
		// nothing reads its maps before they hold light.
		if self.environment.requested.is_none()
			&& let Some(crate::rendering::EnvironmentMaps {
				irradiance: irradiance_image,
				specular: specular_image,
			}) = generated_environment
		{
			if self.environment.bound.specular_image != specular_image {
				self.environment.bind(ResidentEnvironment {
					irradiance_image,
					specular_image,
					sampler: self.loader.environment_sampler,
					// Generated light is physical already, so no requested lux rescales it.
					upward_illuminance: 0.0,
				});
			}
		}
		if self.environment.descriptors_dirty {
			self.environment.descriptors_dirty = false;
			for sink_state in &self.scene.sink_states {
				frame.write(
					&self
						.environment
						.bound
						.descriptor_writes(sink_state.render_pass.material_evaluation()),
				);
			}
			// Surfel and probe rays that miss see the same environment.
			if let Some(pool) = &self.surfel_pool {
				frame.write(&pool.environment_writes(self.environment.bound));
			}
			if let Some(volume) = &self.probe_volume {
				frame.write(&volume.environment_writes(self.environment.bound));
			}
		}
	}

	/// Gives every resident instance the rectangle the resident lightmap holds for it, after the lightmap changed.
	fn refresh_instance_lightmaps(&mut self) {
		for entity in self.scene.render_entities.iter_mut() {
			entity.lightmap = self.lightmaps.instance(entity.handle, entity.primitive);
		}
		self.instances_changed();
	}

	/// Publishes one texture the loader already transferred, and returns whether it is a material texture slot's first
	/// image and a loaded material samples it.
	fn adopt_texture(&mut self, frame: &mut ghi::implementation::Frame, texture: ResidentTexture) -> bool {
		let ResidentTexture {
			id,
			array,
			index,
			image,
			photometry,
		} = texture;
		frame.write(&[self.scene.binder().texture_element(array.binding(), image, index)]);
		if array == TextureArray::IesProfile {
			self.adopt_ies_profile(id, index, photometry);
			return false;
		}
		let texture = self.availability.get_or_insert(Availability::Texture(index), false);
		// A finer image replacing a coarse one changes no material's readiness, so the material lists stay.
		if self.availability.is_ready(texture) {
			return false;
		}
		self.availability.set_available(texture, true);
		self.loaded_materials
			.iter()
			.flatten()
			.any(|material| material.texture_indices.contains(&index))
	}

	/// Records IES profile `id`, resident in `ies_profiles` slot `index`, for the lights that name it, or reports a
	/// profile whose calibration the loader rejected.
	fn adopt_ies_profile(
		&mut self,
		id: String,
		index: u32,
		photometry: Option<resource_management::resources::image::ImagePhotometry>,
	) {
		match photometry {
			Some(photometry) if photometry.intensity_scale_candela.is_finite() && photometry.intensity_scale_candela > 0.0 => {
				self.loaded_ies_profiles.insert(
					id,
					IesProfileTexture {
						profile_index: index,
						intensity_scale_candela: photometry.intensity_scale_candela,
					},
				);
			}
			_ if self.scene.lights.iter().any(|(_, light, _)| {
				light
					.local()
					.and_then(LocalEmission::ies_profile)
					.is_some_and(|profile| profile.resource_id() == id.as_str())
			}) =>
			{
				warn!(
					"Visibility IES profile is invalid: {id}. The most likely cause is that the image was not baked from a usable .ies file or has an invalid candela scale. See {}",
					crate::online_docs_url("use/lighting#use-an-ies-profile")
				);
			}
			_ => {}
		}
	}

	/// Adopts material metadata and its `evaluation` into the canonical table and wires its texture dependencies. A
	/// material the uber shaders evaluate becomes available once its phase's uber shader holds its arm.
	///
	/// Next, call [`Self::rebuild_material_lists`] once the frame's events are adopted.
	fn adopt_material(&mut self, material: ResidentMaterial, evaluation: MaterialEvaluator<ghi::PipelineHandle>) {
		let ResidentMaterial {
			id,
			index,
			alpha_mode,
			double_sided,
			coverage_arm,
			texture_slots: textures,
			parameters,
			hit_group,
			reads_backdrop,
			..
		} = material;
		for shading in [&mut self.surfel_shading, &mut self.probe_shading].into_iter().flatten() {
			shading.set_material(index, hit_group.as_ref());
		}
		let (record, overflow) = MaterialData::describe(&textures, coverage_arm, alpha_mode);
		if overflow {
			warn!(
				"Visibility material {id} has too many texture slots. The most likely cause is that the material shader expects more textures than the visibility material data supports."
			);
		}
		self.materials[index as usize] = record;

		let (evaluated, tested) = (self.covers(evaluation, alpha_mode), self.tests(coverage_arm));
		let covered = evaluated && tested;
		let texture_indices = textures.into_iter().flatten().collect::<Vec<_>>();
		let material = self.availability.get_or_insert(Availability::Material(index), false);
		// Keep the material unavailable while replacing its dependency set so renderables cannot observe a
		// transiently complete branch.
		self.availability.set_available(material, false);
		self.availability.clear_dependencies(material).expect(
			"Visibility material dependencies could not be replaced. The most likely cause is a stale material availability handle.",
		);
		for texture_index in &texture_indices {
			let texture = self.availability.get_or_insert(Availability::Texture(*texture_index), false);
			self.availability.add_dependency(material, texture).expect(
				"Visibility material dependency could not be registered. The most likely cause is a cyclic or stale resource relationship.",
			);
		}
		self.availability.set_available(material, covered);
		let slot = index as usize;
		if slot >= self.loaded_materials.len() {
			self.loaded_materials.resize_with(slot + 1, || None);
		}
		let name: Arc<str> = id.into();
		if !evaluated {
			// Only an arm can be uncovered: a material's own pipeline always shades it.
			if let MaterialEvaluator::Arm(key) = evaluation {
				self.uncovered(UberVariant::of_arm(key, matches!(alpha_mode, AlphaMode::Blend)), &name);
			}
		}
		if !tested {
			self.untested(&name);
		}
		self.material_parameters.adopt_material(index, name.clone(), parameters);
		self.loaded_materials[slot] = Some(LoadedMaterial {
			index,
			evaluation,
			name,
			alpha_mode,
			double_sided,
			coverage_arm,
			texture_indices,
			reads_backdrop,
		});
		self.materials_copies_current = [false; ghi::MAX_FRAMES_IN_FLIGHT];
	}

	/// Returns whether `evaluation` can shade a material of `alpha_mode` now: always for its own pipeline, and for an arm
	/// once the uber shader of its phase and kind holds it.
	fn covers(&self, evaluation: MaterialEvaluator<ghi::PipelineHandle>, alpha_mode: AlphaMode) -> bool {
		match evaluation {
			MaterialEvaluator::Pipeline(_) => true,
			MaterialEvaluator::Arm(key) => self.uber_shaders
				[UberVariant::of_arm(key, matches!(alpha_mode, AlphaMode::Blend)).index()]
			.as_ref()
			.is_some_and(|shader| shader.keys.binary_search(&key).is_ok()),
		}
	}

	/// Returns whether the published coverage shaders test a material whose coverage key is `coverage_arm`: always for
	/// a material they don't test, and for a masked one once they hold its arm.
	fn tests(&self, coverage_arm: Option<u32>) -> bool {
		coverage_arm.is_none_or(|key| {
			self.coverage_keys
				.as_ref()
				.is_some_and(|keys| keys.binary_search(&key).is_ok())
		})
	}

	/// Publishes the newly compiled coverage shaders, which hold the arms of `keys`, and makes every masked material
	/// available exactly when they hold its arm and its evaluation can shade it.
	///
	/// Next, call [`Self::rebuild_material_lists`] once the frame's events are adopted.
	fn adopt_coverage_shaders(&mut self, keys: Box<[u32]>) {
		self.coverage_keys = Some(keys);
		self.coverage_shaders_loading = false;
		for slot in 0..self.loaded_materials.len() {
			let Some(material) = &self.loaded_materials[slot] else {
				continue;
			};
			if material.coverage_arm.is_none() {
				continue;
			}
			let tested = self.tests(material.coverage_arm);
			let covered = tested && self.covers(material.evaluation, material.alpha_mode);
			let (index, name) = (material.index, material.name.clone());
			self.availability.set_key_available(&Availability::Material(index), covered);
			if !tested {
				self.untested(&name);
			}
		}
	}

	/// Handles a masked material whose arm the coverage shaders lack. Development builds bake them again from every
	/// material the store holds; release builds can't, so they report the material.
	///
	/// While the coverage shaders are loading, or baking again for another material, their arrival checks the materials
	/// again instead.
	fn untested(&mut self, material: &str) {
		if self.coverage_shaders_loading {
			return;
		}
		#[cfg(debug_assertions)]
		{
			let _ = material;
			self.coverage_shaders_loading = true;
			self.loader.rebake_coverage_shaders();
		}
		#[cfg(not(debug_assertions))]
		error!(
			"Visibility material {material} is not drawn. The most likely cause is that the masked passes' coverage shaders were baked before this masked material. Bake the resources again with `beld bake`. See {}.",
			crate::online_docs_url("use/resources/bake")
		);
	}

	/// Publishes the newly compiled uber `shader` of `variant`, and makes every material of the variant available
	/// exactly when the shader holds its arm.
	///
	/// Next, call [`Self::rebuild_material_lists`] once the frame's events are adopted.
	fn adopt_uber_shader(&mut self, variant: UberVariant, shader: UberShader) {
		self.uber_shaders[variant.index()] = Some(shader);
		self.uber_shaders_loading[variant.index()] = false;
		for slot in 0..self.loaded_materials.len() {
			let Some(material) = &self.loaded_materials[slot] else {
				continue;
			};
			let MaterialEvaluator::Arm(key) = material.evaluation else {
				continue;
			};
			if UberVariant::of_arm(key, matches!(material.alpha_mode, AlphaMode::Blend)) != variant {
				continue;
			}
			let evaluated = self.covers(material.evaluation, material.alpha_mode);
			let covered = evaluated && self.tests(material.coverage_arm);
			let (index, name) = (material.index, material.name.clone());
			self.availability.set_key_available(&Availability::Material(index), covered);
			if !evaluated {
				self.uncovered(variant, &name);
			}
		}
	}

	/// Handles a material whose arm the uber shader of its `variant` lacks. Development builds bake the shader
	/// again from every material the store holds; release builds can't, so they report the material.
	///
	/// While the variant's shader is loading, or baking again for another material, its arrival checks the materials
	/// again instead.
	fn uncovered(&mut self, variant: UberVariant, material: &str) {
		let phase = variant.index();
		if self.uber_shaders_loading[phase] {
			return;
		}
		#[cfg(debug_assertions)]
		{
			let _ = material;
			self.uber_shaders_loading[phase] = true;
			self.loader.rebake_uber_shader(variant);
		}
		#[cfg(not(debug_assertions))]
		error!(
			"Visibility material {material} is not drawn. The most likely cause is that the uber material evaluation shaders were baked before this material. Bake the resources again with `beld bake`. See {}.",
			crate::online_docs_url("use/resources/bake#choose-how-materials-are-evaluated")
		);
	}

	/// Rebuilds the opaque and transparent material evaluation lists and gives every ready material its evaluation slot.
	///
	/// Materials whose programs differ only in the textures they bind request equal pipelines, which the pipeline
	/// manager compiles once, so grouping ready materials by pipeline finds them without comparing programs. Each
	/// group gets one slot in the list of its phase, and each phase records one dispatch per slot it draws instead of
	/// one per material. A
	/// material that isn't ready keeps [`NO_EVALUATION`]; no admitted instance uses it.
	///
	/// In the uber mode, each phase's list holds its uber shaders alone, the translucent materials' after the others',
	/// and a ready material's table entry holds its arm key, which its shader matches.
	fn rebuild_material_lists(&mut self) {
		let render_info = &mut self.scene.render_info;
		render_info.opaque_evaluations.clear();
		render_info.transparent_evaluations.clear();
		self.material_evaluations.fill(NO_EVALUATION);
		self.materials_copies_current = [false; ghi::MAX_FRAMES_IN_FLIGHT];
		// An instance's evaluation slot comes from these lists.
		self.instances_stale = true;
		if self.settings.features.material_evaluation == MaterialEvaluationMode::Uber {
			// Each phase's uber shader evaluates every ready material, so it reads the backdrop when any of them does.
			let mut reads_backdrop = false;
			for material in self.loaded_materials.iter().flatten() {
				if let MaterialEvaluator::Arm(key) = material.evaluation
					&& self.availability.is_key_ready(&Availability::Material(material.index))
				{
					self.material_evaluations[material.index as usize] = key;
					reads_backdrop |= material.reads_backdrop;
				}
			}
			// `UberVariant::ALL` lists each phase's translucent shader after its other one.
			for variant in UberVariant::ALL {
				let Some(shader) = &self.uber_shaders[variant.index()] else {
					continue;
				};
				// The opaque phase's translucent shader leaves the background to the other one, so it waits for it.
				if variant.translucent && !variant.transparent && self.uber_shaders[UberVariant::BACKGROUND.index()].is_none() {
					continue;
				}
				let evaluations = if variant.transparent {
					&mut render_info.transparent_evaluations
				} else {
					&mut render_info.opaque_evaluations
				};
				evaluations.push(MaterialEvaluation {
					name: shader.name.clone(),
					index: variant.evaluation_index(),
					pipeline: shader.pipeline,
					reads_backdrop,
				});
			}
			return;
		}
		// The availability graph combines pipeline and texture readiness before a material reaches an evaluation.
		let mut ready = self
			.loaded_materials
			.iter()
			.flatten()
			.filter(|material| self.availability.is_key_ready(&Availability::Material(material.index)))
			.filter_map(|material| match material.evaluation {
				MaterialEvaluator::Pipeline(pipeline) => Some((pipeline, material)),
				MaterialEvaluator::Arm(_) => None,
			})
			.collect::<Vec<_>>();
		ready.sort_unstable_by_key(|(pipeline, material)| (*pipeline, material.index));
		for (evaluation_index, materials) in ready.chunk_by(|(left, _), (right, _)| left == right).enumerate() {
			let evaluation_index = evaluation_index as u32;
			for (_, material) in materials {
				self.material_evaluations[material.index as usize] = evaluation_index;
			}
			// The loader specializes every pipeline for its material's phase, so a slot belongs to one phase.
			let (pipeline, first) = materials[0];
			let evaluations = if matches!(first.alpha_mode, AlphaMode::Blend) {
				&mut render_info.transparent_evaluations
			} else {
				&mut render_info.opaque_evaluations
			};
			evaluations.push(MaterialEvaluation {
				name: match materials.len() {
					1 => first.name.clone(),
					count => format!("{} and {} more", first.name, count - 1).into(),
				},
				index: evaluation_index,
				pipeline,
				// Materials that share a pipeline share their program, so the first one speaks for the slot.
				reads_backdrop: first.reads_backdrop,
			});
		}
	}

	/// Gives the render entities of every primitive in `refreshed_lods` the detail levels they hold now, after finer
	/// levels arrived or left, and has the next frame pick those entities again.
	fn refresh_entity_lods(&mut self) {
		if self.refreshed_lods.is_empty() {
			return;
		}
		// One pass over the entities serves every primitive refreshed this frame. Each refreshed entity gives up its
		// pick, so the next frame follows it like a moved one and rewrites only its record.
		let (refreshed, loader, kept) = (&self.refreshed_lods, &self.loader, &mut self.instance_kept);
		for (slot, entity) in self.scene.render_entities.indexed_iter_mut() {
			if !refreshed.contains(&(entity.mesh, entity.primitive)) {
				continue;
			}
			if let Some(lods) = loader.primitive_lods(entity.mesh, entity.primitive as usize) {
				entity.lods = lods;
			}
			// An entity added since the last rebuild has no kept slot yet, and the rebuild it waits for picks it.
			if let Some(kept) = kept.get_mut(slot) {
				kept.release();
			}
		}
		self.refreshed_lods.clear();
		self.instances_moved = true;
	}

	/// Adds one resident renderable at `model`.
	fn add_renderable(&mut self, handle: Handle, key: MeshKey, model: ghi::pod::Mat4x3f) {
		// The mesh stays borrowed from the loader while the other fields change, so nothing copies it.
		let Some(mesh) = self.loader.mesh(key) else {
			return;
		};
		if let Some(structures) = &mut self.acceleration_structures {
			structures.register_mesh(key, mesh);
		}
		let availability = self.availability.get_or_insert(Availability::Renderable(handle), true);
		for (index, primitive) in mesh.primitives.iter().enumerate() {
			let material = self
				.availability
				.get_or_insert(Availability::Material(primitive.material_index), false);
			self.availability
				.add_dependency(availability, material)
				.expect("Visibility renderable dependency could not be registered. The most likely cause is a cyclic or stale resource relationship.");
			// The mesh may already hold finer levels another renderable asked for.
			let lods: MeshLods = self.loader.primitive_lods(key, index).unwrap_or(primitive.lods);
			self.scene.add_render_entity(RenderEntity {
				handle,
				mesh: key,
				primitive: index as u32,
				availability,
				// Each frame names the level and the parameter block the instance reads.
				shader_mesh: ShaderMesh::new(model, primitive.material_index),
				bounding_sphere: primitive.bounding_sphere,
				lods,
				residency: self.mesh_residencies.slot(key, index as u32),
				lightmap: self.lightmaps.instance(handle, index as u32),
				skinning: primitive.skin.map(|skin| RenderSkin {
					skin,
					source_vertex_offset: primitive
						.skinning_source_vertex_offset
						.expect("Skinned primitive has no GPU source range. The most likely cause is that skin streams were not uploaded with the mesh resource."),
					vertex_count: primitive.skinning_vertex_count,
					skeleton_node_count: mesh.skeleton_node_count,
					posed_vertices: PosedVertices::Unallocated,
				}),
			});
		}
		// A renderable of a mesh that is already resident arrives with no loader event.
		self.instances_changed();
	}

	/// Starts drawing the renderable terrain `handle`, and requests its node tree when no other terrain drew it yet.
	fn create_terrain(&mut self, handle: Handle, terrain: RenderableTerrain, transforms: &ShownTransforms) {
		let mut changes = std::mem::take(&mut self.terrain_changes);
		let id = terrain.resource_id().clone();
		if self.terrains.add(handle, id.clone(), &mut changes) {
			self.loader.request_terrain(id);
		}
		self.apply_terrain_changes(&mut changes, transforms);
		self.terrain_changes = changes;
	}

	/// Selects every terrain's nodes for frame `frame` from `lod_view`'s camera, adds and removes their instances,
	/// requests the node meshes they want, and releases the node meshes no terrain needed for a while.
	fn stream_terrains(&mut self, frame: u64, lod_view: Option<LodView>, transforms: &ShownTransforms) {
		let Some(lod_view) = lod_view else {
			return;
		};
		let mut changes = std::mem::take(&mut self.terrain_changes);
		let mut requests = std::mem::take(&mut self.terrain_requests);
		let loader = &self.loader;
		self.terrains.select(
			frame,
			|terrain| Some(lod_view.in_space(&model_of(terrain, transforms))),
			|source| requests.push(source),
			|key| loader.mesh(key).is_some(),
			&mut changes,
		);
		for source in requests.drain(..) {
			self.loader.request_mesh(source);
		}
		self.terrain_requests = requests;
		self.apply_terrain_changes(&mut changes, transforms);
		self.terrain_changes = changes;

		// Rays hit a mesh through acceleration structures that stay built while the mesh is resident, so meshes that
		// rays may hit stay.
		if self.acceleration_structures.is_some() {
			return;
		}
		let mut idle = self.terrains.idle_meshes(frame).collect::<SmallVec<[(usize, MeshKey); 16]>>();
		// Each release moves the last requested mesh into its slot, so slots go from the highest down.
		idle.sort_unstable_by(|(a, _), (b, _)| b.cmp(a));
		for (slot, key) in idle {
			// A mesh that isn't resident yet, that a renderable still draws, or whose finer levels are loading, stays
			// until a later frame.
			if self.loader.mesh(key).is_none()
				|| self.scene.render_entities.iter().any(|entity| entity.mesh == key)
				|| !self.mesh_residencies.release(key)
			{
				continue;
			}
			self.loader.release_mesh(key);
			self.terrains.released(slot);
		}
	}

	/// Applies the scene changes terrains asked for, and empties `changes`.
	fn apply_terrain_changes(&mut self, changes: &mut Vec<TerrainChange>, transforms: &ShownTransforms) {
		for change in changes.drain(..) {
			match change {
				TerrainChange::Remove(instance) => self.remove_mesh_instance(instance),
				TerrainChange::Add { terrain, instance, mesh } => {
					if self.loader.mesh(mesh).is_none() {
						continue;
					}
					self.scene.renderables.insert_resident(instance);
					self.add_renderable(instance, mesh, model_of(terrain, transforms));
				}
			}
		}
	}

	/* Frame preparation */

	/// Answers the screen scene queries the frame of `sequence` read when it last ran, which completed before this one
	/// started, then queues the queries the world created since and answers those no sink of `sinks` shows.
	fn adopt_screen_queries(&mut self, frame: &ghi::implementation::Frame, sequence: usize, sinks: &[Sink]) {
		let hits = &self.screen_query_hits;
		if self.screen_queries.has_recorded(sequence) {
			let sink_states = &self.scene.sink_states;
			self.screen_queries.complete(
				sequence,
				|sink| {
					let state = sink_states.iter().find(|state| state.id == sink)?;
					state.render_pass.read_screen_queries(frame).copied()
				},
				|hit| {
					hits.create(hit);
				},
			);
		}
		let mut created = false;
		while let Some(message) = self.screen_query_listener.read() {
			self.screen_queries.push(message.handle(), *message.data());
			created = true;
		}
		if created {
			let shown = sinks
				.iter()
				.filter(|sink| self.scene.sink_states.iter().any(|state| state.id == sink.index()))
				.filter_map(Sink::camera)
				.collect::<SmallVec<[Handle; 16]>>();
			self.screen_queries.answer_unshown(&shown, |hit| {
				hits.create(hit);
			});
		}
	}

	/// Applies queued GTAO, GI, surfel GI, probe GI, contact-shadow, and detail level controls before any sink records
	/// this frame's commands.
	fn apply_runtime_settings(&mut self) {
		self.lod.drain();
		self.gtao.drain();
		self.gi.drain();
		self.surfel_gi.drain();
		self.probe_gi.drain();
		self.contact_shadow.drain();
		let features = self.settings.features;
		// A feature left out of the shaders stays off, whatever the runtime asks for.
		self.gtao.settings.enabled = built_in_or_off(self.gtao.settings.enabled, features.gtao, GTAO_ENABLED_PARAMETER);
		self.contact_shadow.settings.enabled = built_in_or_off(
			self.contact_shadow.settings.enabled,
			features.contact_shadows,
			CONTACT_SHADOWS_ENABLED_PARAMETER,
		);
		crate::rendering::warn_once(
			&mut self.reported_surfel_gi_fallback,
			self.gi.settings.enabled && self.gi.settings.technique.traces_rays() && !self.ray_tracing,
			|| {
				let technique = match self.gi.settings.technique {
					IndirectDiffuseSource::Probes => "Probe GI",
					_ => "Surfel GI",
				};
				format!(
					"{technique} can't light frames on this device. The most likely cause is that the device doesn't trace rays: only the Metal backend does, and only when `{RENDER_PARAMETERS}.ghi.features.ray-tracing` is on. SSGI lights indirect diffuse light in its place while it is built in; run on a Mac with Apple silicon, or set `{RENDER_PARAMETERS}.ghi.features.ray-tracing` to `true` in `config.json` and restart. See {}.",
					crate::online_docs_url("reference/parameters#graphics-device")
				)
			},
		);
	}

	/// Rebuilds the frame's instance lists from whole renderables whose dependencies are ready, and uploads skin palettes.
	///
	/// Each instance draws the detail level `lod_view` picks for it when it is resident, or the finest resident level
	/// coarser than it. Without a camera it draws the finest resident level. The levels instances want go to
	/// [`MeshResidencies`], which decides what loads next. Large scenes rebuild on several of `alley`'s lanes, with the
	/// same result as one; see [`super::instance_rebuild`].
	///
	/// With `follow`, nothing but the camera changed since the instances were rebuilt or last followed, so they are
	/// brought up to date by picking again only the ones whose pick expired; an entity that can't be followed
	/// rebuilds them all after all.
	///
	/// Returns what changed in the kept instances, which the caller copies to the frame's buffers. Its `outdated` says
	/// the next frame must rebuild: an instance had to be finished in order, as a posed skinned instance is every
	/// frame.
	fn update_active_instances(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		lod_view: Option<LodView>,
		follow: bool,
		alley: &mut Alley,
	) -> Followed {
		let _span = debug_span!(
			"Visibility::rebuild_active_instances",
			entities = self.scene.render_entities.len()
		)
		.entered();
		let capacity = self.instance_records.len();

		let availability = &self.availability;
		let loaded_materials = &self.loaded_materials;
		let material_evaluations = &self.material_evaluations;
		let sources = InstanceSources {
			entities: &self.scene.render_entities,
			// A renderable enters a frame as one object; never expose the subset whose materials loaded first.
			admit: |entity: &RenderEntity| {
				if !availability.is_ready(entity.availability) {
					return None;
				}
				let material = loaded_materials
					.get(entity.shader_mesh.material_index() as usize)
					.and_then(Option::as_ref)?;
				Some(Admission {
					phase: instance_phase(&material.alpha_mode, material.double_sided),
					evaluation: match material.evaluation {
						MaterialEvaluator::Pipeline(_) => material_evaluations[entity.shader_mesh.material_index() as usize],
						// The phase has one uber shader for its translucent materials and one for the others.
						MaterialEvaluator::Arm(key) => {
							UberVariant::of_arm(key, matches!(material.alpha_mode, AlphaMode::Blend)).evaluation_index()
						}
					},
					double_sided: material.double_sided,
				})
			},
			lod_view,
			residencies: &self.mesh_residencies,
			parameters: &self.material_parameters,
			traces_rays: self.acceleration_structures.is_some(),
		};

		let lanes = rebuild_lanes(sources.entities.slots_len(), self.settings.instance_rebuild_lanes());
		let render_info = &mut self.scene.render_info;
		/// Returns the phase lists and the evaluation masks of `render_info`, in the order the rebuild fills them.
		fn outputs(
			render_info: &mut RenderInfo,
		) -> (
			[&mut Vec<super::scene::Instance>; super::instance_rebuild::PHASES],
			[&mut super::layout::ActiveEvaluationMask; 2],
		) {
			let RenderInfo {
				opaque_instances,
				masked_instances,
				double_sided_instances,
				double_sided_masked_instances,
				transparent_instances,
				opaque_evaluation_mask,
				transparent_evaluation_mask,
				..
			} = render_info;
			(
				[
					opaque_instances,
					masked_instances,
					double_sided_instances,
					double_sided_masked_instances,
					transparent_instances,
				],
				[opaque_evaluation_mask, transparent_evaluation_mask],
			)
		}
		// Following measures each kept pick against the camera, so a frame without one rebuilds.
		if follow && lod_view.is_some() {
			let (phases, masks) = outputs(render_info);
			let followed = follow_instances(
				alley,
				&sources,
				lanes,
				InstanceOutputs {
					meshes: &mut self.instance_records,
					traced: &mut self.traced_primitives,
					phases,
					masks,
					lanes: &mut self.rebuild_scratch,
					kept: &mut self.instance_kept,
				},
			);
			if !followed.outdated {
				self.mesh_residencies
					.end_frame(|key, primitives| self.loader.request_mesh_levels(key, primitives));
				return followed;
			}
		}

		render_info.clear_active_instances();
		if std::mem::take(&mut self.posed_uv_copies_recorded) {
			render_info.posed_uv_copies.clear();
		}
		self.skinning_frame.clear();
		let (phases, masks) = outputs(render_info);
		let left_out = rebuild_instances(
			alley,
			&sources,
			lanes,
			InstanceOutputs {
				meshes: &mut self.instance_records,
				traced: &mut self.traced_primitives,
				phases,
				masks,
				lanes: &mut self.rebuild_scratch,
				kept: &mut self.instance_kept,
			},
		);
		let mesh_data = &mut self.instance_records;

		// Finish, in slot order, the instances whose parameter block or skin changes shared state.
		for deferred in self.rebuild_scratch.iter().flat_map(|lane| &lane.deferred) {
			let entity = self
				.scene
				.render_entities
				.get_slot_mut(deferred.slot as usize)
				.expect("a deferred instance's entity is in the scene");
			let shader_mesh = &mut mesh_data[deferred.slot as usize];
			shader_mesh.set_parameter_block(self.material_parameters.block(entity.handle, shader_mesh.material_index()));
			let Some(skin) = &mut entity.skinning else {
				continue;
			};
			let Some(pose) = self.scene.skinning_poses.get(&entity.handle) else {
				continue;
			};
			assert_eq!(
				pose.len(),
				skin.skeleton_node_count as usize,
				"Visibility skin pose has the wrong matrix count. The most likely cause is that the pose was written for a different skeleton."
			);
			// The first posed frame takes the instance's own vertices, which every later stage reads in place of the
			// mesh's bind pose. Without room it keeps drawing the bind pose; the allocator logged why.
			if skin.posed_vertices == PosedVertices::Unallocated && skin.vertex_count > 0 {
				skin.posed_vertices = allocate_vertices(&self.geometry.ranges, skin.vertex_count, "a posed skinned instance")
					.map_or(PosedVertices::Unavailable, PosedVertices::Allocated);
				// Skinning writes only positions and normals, so the bind-pose UVs are copied beside them once.
				if let PosedVertices::Allocated(range) = &skin.posed_vertices {
					let uv_stride = VERTEX_UV_BUFFER_STRIDE as usize;
					self.scene.render_info.posed_uv_copies.push(ghi::BufferCopyDescriptor::new(
						self.geometry.vertex_uvs.into(),
						deferred.vertex_base as usize * uv_stride,
						self.geometry.vertex_uvs.into(),
						range.start as usize * uv_stride,
						skin.vertex_count as usize * uv_stride,
					));
				}
			}
			// The entity's mesh stays resident while the entity is in the scene, so its binding is there.
			let binding = self
				.loader
				.mesh(entity.mesh)
				.and_then(|mesh| mesh.skins.get(skin.skin as usize))
				.expect("Skinned instance's mesh holds no binding at its skin index. The most likely cause is that the mesh was released while an instance still drew it.");
			if let PosedVertices::Allocated(range) = &skin.posed_vertices
				&& let Some((palette_base, kind)) =
					self.skinning_frame
						.palette(entity.handle, (entity.mesh, skin.skin), binding, pose)
			{
				self.scene.render_info.skinning_dispatches.push(SkinningDispatch {
					source_vertex_base: skin.source_vertex_offset,
					destination_vertex_base: range.start,
					palette_base,
					palette_count: binding.entries.len() as u32,
					vertex_count: skin.vertex_count,
					palette_kind: kind as u32,
				});
				shader_mesh.set_posed_vertices(range.start);
			}
		}
		crate::rendering::warn_once(&mut self.reported_limits[2], left_out > 0, || {
			format!(
				"Visibility left {left_out} instances out of a frame. The most likely cause is that the scene shows more mesh primitives than the {capacity} instances the pipeline holds; raise `render.instances.capacity`."
			)
		});
		self.skinning_pass
			.write_palettes(frame, &self.skinning_frame.matrices, &self.skinning_frame.dual_quaternions);
		self.mesh_residencies
			.end_frame(|key, primitives| self.loader.request_mesh_levels(key, primitives));
		Followed {
			records: 0..self.instance_kept.len(),
			relisted: true,
			// An instance finished here changes every frame, as a posed one does, or was given its parameter block
			// just now, which the next rebuild records. Either way the next frame can't keep these instances.
			outdated: self.rebuild_scratch.iter().any(|lane| !lane.deferred.is_empty()),
		}
	}

	/// Returns each shadowed sun's cascades, fitted to the camera frustum, by sun slot, and the view every map of
	/// `shadows` would be drawn from this frame. A sun under a virtual shadow map has no cascades.
	///
	/// `ies_scales` holds each light's IES intensity scale by light index.
	fn shadow_views(
		&self,
		main_view: View,
		shadows: &ShadowLightSelection<'_>,
		ies_scales: &[(f32, Option<IesProfileTexture>)],
	) -> (SunCascades, ShadowViews) {
		let cascade_suns = if shadows.virtual_suns { &[][..] } else { &shadows.suns[..] };
		let cascades: SunCascades = cascade_suns
			.iter()
			.map(|sun| {
				csm::make_cascade_frames(
					main_view,
					sun.direction,
					SHADOW_CASCADE_COUNT,
					self.settings.directional_shadow_map_resolution,
					self.settings.cascade_splits,
				)
				.collect::<SmallVec<[_; SHADOW_CASCADE_COUNT]>>()
				.into_inner()
				.expect("Cascade count does not match the shadow views. The most likely cause is that make_cascade_frames was called with another cascade count.")
			})
			.collect();
		let views = ShadowViews {
			cascades: cascades
				.iter()
				.map(|frames| {
					std::array::from_fn(|cascade| ShaderViewData {
						far: frames[cascade].slice_far,
						..ShaderViewData::from(frames[cascade].view)
					})
				})
				.collect(),
			cones: shadows
				.cones
				.iter()
				.map(|cone| {
					let scale = ies_scales[cone.index].0;
					make_cone_shadow_view(cone.light, cone.transform, SHADOW_DEFAULT_EXPOSURE_SCALE, scale).into()
				})
				.collect(),
			points: shadows
				.points
				.iter()
				.map(|point| {
					let scale = ies_scales[point.index].0;
					std::array::from_fn(|face| {
						make_point_shadow_view(point.light, point.transform, face, SHADOW_DEFAULT_EXPOSURE_SCALE, scale).into()
					})
				})
				.collect(),
		};
		(cascades, views)
	}

	/// Writes the camera view and every shadow view: the view each map was drawn from, so a map that doesn't draw this
	/// frame is sampled as it was drawn, or this frame's view for a map that holds nothing yet.
	///
	/// With receivers fitting, cascades get this frame's CPU fit instead; the cascade fit pass then puts in each
	/// cascade's drawn view, see [`super::render_pass::CascadeFitPass`]. Without it, a cascade that holds nothing gets a
	/// view no receiver falls in.
	fn write_views(
		&self,
		frame: &mut ghi::implementation::Frame,
		main_view: View,
		current: &ShadowViews,
		states: &ShadowMapStates,
	) {
		let views = frame.map(self.scene.views_buffer);
		views.fill(ShaderViewData::from(main_view));
		let gpu_fit = self.settings.cascade_fitting == CascadeFitting::Receivers;
		for (slot, cascades) in current.cascades.iter().enumerate() {
			for (cascade, current) in cascades.iter().enumerate() {
				views[sun_cascade_view(slot) + cascade] = match states.cascade_view(slot, cascade) {
					_ if gpu_fit => *current,
					Some(drawn) => drawn_cascade_view(drawn, current.far),
					None => undrawn_cascade_view(current),
				};
			}
		}
		for (position, current) in current.cones.iter().enumerate() {
			views[CONE_SHADOW_VIEW_OFFSET + position] = *states.cone_view(position).unwrap_or(current);
		}
		for (position, faces) in current.points.iter().enumerate() {
			for (face, current) in faces.iter().enumerate() {
				views[POINT_SHADOW_VIEW_OFFSET + position * POINT_SHADOW_FACE_COUNT + face] =
					*states.point_face_view(position, face).unwrap_or(current);
			}
		}
	}
}

/// Returns the model matrix `handle` is shown at, or the identity when it has no transform.
fn model_of(handle: Handle, transforms: &ShownTransforms) -> ghi::pod::Mat4x3f {
	transforms
		.get(handle)
		.map_or_else(math::Matrix::identity, Transform::get_matrix)
		.into()
}

impl PipelineManager for VisibilityPipelineManager {
	/// Drains creation messages in dependency order: resources first so their loads start before any entity
	/// needs them, then lights, meshes, and environments.
	fn update(&mut self, transforms: &ShownTransforms) {
		while let Some(message) = self.resource_listener.read() {
			self.loader.request_resource(message.into_data());
		}
		// Concrete routes keep application-defined creation strongly typed. The scene erases each light only at its
		// storage boundary.
		while let Some(message) = self.cone_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into(), transforms);
		}
		while let Some(message) = self.directional_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into(), transforms);
		}
		while let Some(message) = self.point_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into(), transforms);
		}
		while let Some(message) = self.mesh_listener.read() {
			self.request_mesh(message.handle(), message.into_data(), transforms);
		}
		while let Some(message) = self.terrain_listener.read() {
			self.create_terrain(message.handle(), message.into_data(), transforms);
		}
		while let Some(message) = self.fog_listener.read() {
			self.fog = Some(*message.data());
		}
		while let Some(message) = self.environment_listener.read() {
			self.create_environment(message.into_data());
		}
		while let Some(message) = self.lightmap_listener.read() {
			let holder = message.handle();
			// The ID moves into the lightmaps, and the loader gets the one copy it keeps.
			if let Some(id) = self.lightmaps.request(holder, message.into_data().into_resource_id()) {
				self.loader.request_lightmap(id.to_owned());
			}
		}
	}

	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		transforms: &ShownTransforms,
		alley: &mut Alley,
		time: crate::time::MediaTime,
		background: &mut crate::rendering::SceneBackgroundFrame<'_>,
	) -> SmallVec<[(usize, RenderPassReturn<'a>); 16]> {
		debug_span!("Visibility::adopt_scene_updates").in_scope(|| self.adopt_scene_updates(transforms));
		self.apply_runtime_settings();
		let generated_environment = background.environment_maps();
		debug_span!("Visibility::adopt_resource_completions")
			.in_scope(|| self.adopt_resource_completions(frame, transforms, generated_environment));
		// A jittered frame reads textures one level finer, since the temporal blend averages away the aliasing the finer
		// level would bring, and a scene rendered below its output extent reads them as fine as the output's pixels; see
		// the jitter rules in the rendering reference.
		let texture_footprint = sinks.first().map_or(1.0, |sink| {
			let render_scale = sink.render_extent().width() as f32 / sink.output_extent().width().max(1) as f32;
			render_scale * if sink.is_jittered() { 0.5 } else { 1.0 }
		});
		// The masked cutoff follows the first sink's jitter, as the texture footprint does: only a temporal blend averages
		// the noise into partial coverage.
		let noise_slice = match sinks.first() {
			Some(sink) if sink.is_jittered() => blue_noise_slice(sink, frame.key().frame_index()),
			_ => FIXED_CUTOFF,
		};
		*frame.map(self.scene.frame_buffer) = ShaderFrame {
			time: time.as_seconds_f32(),
			texture_footprint,
			noise_slice,
			counts_meshlets: u32::from(self.lod.settings.count_meshlets),
			// Detail levels follow the first sink's camera, as the instances' levels do below.
			shadow_level_error: self
				.lod
				.settings
				.shadow_level_error(sinks.first().and_then(Sink::detail_level)),
		};
		// Each frame sequence has its own buffers, so changed tables are copied into each one on its own frame.
		let sequence = frame.key().sequence_index() as usize;
		if !self.materials_copies_current[sequence] {
			self.materials_copies_current[sequence] = true;
			frame.map(self.materials_buffer).copy_from_slice(&*self.materials);
			frame
				.map(self.material_evaluations_buffer)
				.copy_from_slice(&*self.material_evaluations);
		}
		self.adopt_screen_queries(frame, sequence, sinks);
		// Detail levels follow the first sink's camera, like the views and exposure below.
		let lod_view = sinks.first().map(|sink| LodView::new(sink, self.lod.settings));
		// Terrains place this frame's node instances before the frame lists its instances.
		self.stream_terrains(frame.key().frame_index(), lod_view, transforms);
		// A camera that only moved, zoomed, or changed size leaves most instances as they are, so they are followed,
		// not rebuilt. A forced level or a switch of projection picks every level anew.
		let moved = lod_view != self.instances_lod_view;
		self.instances_stale |= match (&lod_view, &self.instances_lod_view) {
			(Some(view), Some(last)) => !view.scales_alike(last),
			(None, None) => false,
			_ => true,
		};
		self.instances_lod_view = lod_view;
		if self.instances_stale || moved || std::mem::take(&mut self.instances_moved) {
			let updated = self.update_active_instances(frame, lod_view, !self.instances_stale, alley);
			self.instances_stale = updated.outdated;
			self.virtual_shadow_changes = range_union(self.virtual_shadow_changes.clone(), updated.records.clone());
			for stale in &mut self.stale_records {
				*stale = range_union(stale.clone(), updated.records.clone());
			}
			if updated.relisted {
				self.stale_work = [true; ghi::MAX_FRAMES_IN_FLIGHT];
			}
		} else {
			// Nothing the instances are made from changed, so the frame draws the ones it has.
			self.mesh_residencies
				.repeat_frame(|key, primitives| self.loader.request_mesh_levels(key, primitives));
		}
		// Every frame sequence has its own copy of the instance table, so each takes what changed since its last frame.
		let stale = std::mem::replace(&mut self.stale_records[sequence], 0..0);
		if !stale.is_empty() {
			frame.map(self.scene.meshes_buffer)[stale.clone()].copy_from_slice(&self.instance_records[stale]);
		}
		// Instances write their own parameter blocks as the frame names them, so the changes are copied after.
		if let Some((first, changes)) = self.material_parameters.take_changes(sequence) {
			frame.map(self.material_parameters_buffer)[first..first + changes.len()].copy_from_slice(changes);
		}
		// The GPU builds the work from the instances each phase lists, so only a new list is written.
		if std::mem::take(&mut self.stale_work[sequence]) {
			self.mesh_work.write_lists(frame, &self.scene.render_info);
		}
		let dispatches = self.mesh_work.dispatches();

		crate::rendering::warn_once(&mut self.reported_limits[0], self.scene.lights.len() > MAX_LIGHTS, || {
			format!(
				"Too many lights for the visibility pipeline. The most likely cause is that the scene contains more than {MAX_LIGHTS} lights."
			)
		});
		// Resolve each light's IES profile once; selection, shadow views, and the lighting upload read it by index.
		let profiles = &self.loaded_ies_profiles;
		let mut ies_scales = Vec::with_capacity_in(self.scene.lights.len().min(MAX_LIGHTS), frame_allocator);
		ies_scales.extend(
			self.scene
				.lights
				.iter()
				.take(MAX_LIGHTS)
				.map(|(_, light, _)| resolve_ies_profile(light, profiles)),
		);
		let budget = self.settings.shadow_budget();
		let select_span = debug_span!("Visibility::select_shadow_lights", lights = self.scene.lights.len()).entered();
		let shadows = select_shadow_lights(
			self.scene.lights.iter().map(|(_, light, transform)| (light, transform)),
			sinks,
			&budget,
			|index| ies_scales[index].0,
			&self.shadow_slots,
		);
		drop(select_span);
		crate::rendering::warn_once(&mut self.reported_limits[1], shadows.unshadowed_count > 0, || {
			format!(
				"{} lights remain lit without shadows. The most likely cause is that more visible lights need shadows than the {} set by the `{SHADOW_MAP_BUDGET_PARAMETER}` render parameter holds, that the directional cascades' resolution leaves too little of it, or that more directional lights shine than the {} the `{DIRECTIONAL_LIGHTS_MAX_PARAMETER}` render parameter lets cast shadows. Raise the parameter that fits, and bake the resources again after changing `{DIRECTIONAL_LIGHTS_MAX_PARAMETER}`.",
				shadows.unshadowed_count,
				utils::ByteSize(u64::from(self.settings.shadow_map_budget_mib) << 20),
				self.settings.features.shadowed_suns,
			)
		});
		self.shadow_layout = retain_layout(self.shadow_layout, shadows.layout(), &budget);
		self.shadow_slots = shadows.slots();
		// Shadow views follow the first sink's camera, and each map's importance the largest sink.
		let (cascades, shadow_views) = debug_span!("Visibility::shadow_views").in_scope(|| {
			sinks
				.first()
				.map(|sink| self.shadow_views(sink.view(), &shadows, &ies_scales))
				.unwrap_or_default()
		});
		let sink_pixels = sinks
			.iter()
			.map(|sink| sink.extent().width() as f32 * sink.extent().height() as f32)
			.fold(0.0, f32::max);
		// The frame's states become the kept ones only once the frame-wide work records, below.
		let mut shadow_states = self.shadow_states.clone();
		let shadow_draws = debug_span!("Visibility::update_shadow_states").in_scope(|| {
			shadow_states.update(
				&shadows,
				self.shadow_layout,
				&shadow_views,
				sink_pixels,
				self.settings.directional_shadow_map_resolution,
				self.settings.shadow_update_texels,
			)
		});
		if let Some(sink) = sinks.first() {
			// Scene geometry rasterizes, and materials rebuild their pixels, through the jittered view.
			self.write_views(frame, sink.jittered_view(), &shadow_views, &shadow_states);
		}
		// Under virtual shadow maps the first sun's levels take the directional views, level `l` view `1 + l`, placed
		// around the first sink's camera.
		let virtual_shadows = match (sinks.first(), shadows.suns.first()) {
			(Some(sink), Some(sun)) if self.shadow_maps.virtual_maps.enabled() => {
				let eye = sink.view().eye();
				let camera = math::Point::new(eye.x, eye.y, eye.z);
				let levels = place_virtual_shadow_levels(
					sun.direction,
					camera,
					self.settings.virtual_shadows.first_level_size(),
					self.virtual_shadow_levels.as_ref(),
				);
				let views = frame.map(self.scene.views_buffer);
				for (level, view) in levels.views.iter().enumerate() {
					views[DIRECTIONAL_SHADOW_VIEW_OFFSET + level] = ShaderViewData::from(*view);
				}
				// The rewritten records, and the ones that appeared or left since the map last recorded.
				let records = self.instance_kept.len();
				let previous = self.virtual_shadow_records;
				let changed = range_union(
					self.virtual_shadow_changes.clone(),
					previous.min(records)..previous.max(records),
				);
				let data = ShaderVirtualShadowFrame::new(
					&levels,
					self.settings.virtual_shadows,
					camera,
					self.settings.cascade_splits.distance(),
					changed.start as u32..changed.end as u32,
					records as u32,
				);
				Some((levels, data))
			}
			_ => None,
		};
		// A local light casts shadows only once its map holds a drawn one.
		let shadow_for = |index| shadow_states.shadow_for(&shadows, index);
		// Like the views above, exposure comes from the first sink; every sink shares one lighting upload.
		let exposure = sinks.first().map_or(1.0, Sink::exposure_scale);
		debug_span!("Visibility::write_lighting").in_scope(|| {
			self.scene
				.write_lighting(frame, shadow_for, exposure, self.environment.intensity(), &ies_scales)
		});
		let shadow_work = ShadowWork {
			suns: shadows.suns.clone(),
			cascade_resolution: self.settings.directional_shadow_map_resolution,
			receiver_fit: (self.settings.cascade_fitting == CascadeFitting::Receivers)
				.then_some(cascades)
				.unwrap_or_default(),
			drawn_cascades: shadow_states.drawn_cascades(shadows.layout().suns),
			layout: self.shadow_layout,
			draws: shadow_draws,
		};

		let indirect_diffuse = indirect_diffuse_source(self.gi.settings, self.settings.features, self.ray_tracing);
		let loaded_materials = &self.loaded_materials;
		let alpha_mode = |material: u32| {
			loaded_materials
				.get(material as usize)
				.and_then(Option::as_ref)
				.map_or(&AlphaMode::Blend, |material| &material.alpha_mode)
		};
		// Surfel and probe rays trace once their shading pipeline published, with the material hit groups it has. The
		// meshes the frame draws for the first time get their hit records before the table is written from them, so it
		// holds every record an instance of the frame reports.
		let shading = match indirect_diffuse {
			Some(IndirectDiffuseSource::Surfels) => self.surfel_shading.as_mut(),
			Some(IndirectDiffuseSource::Probes) => self.probe_shading.as_mut(),
			_ => None,
		};
		let ray_tracing_span = debug_span!("Visibility::prepare_ray_tracing").entered();
		let tracing = match (&mut self.acceleration_structures, shading) {
			(Some(structures), Some(shading)) => {
				let record_materials = structures.admit(frame, &self.traced_primitives, alpha_mode);
				shading
					.prepare(frame, &self.pipeline_manager, record_materials)
					.map(|(pipeline, table)| RayTracing {
						scene: self.scene.descriptor_set,
						ray_traced_scene: structures.descriptor_set(),
						pipeline,
						table,
					})
			}
			_ => None,
		};
		let surfels = match (indirect_diffuse, &self.surfel_pool, sinks.first(), tracing, &self.ray_hits) {
			(Some(IndirectDiffuseSource::Surfels), Some(pool), Some(sink), Some(tracing), Some(hits)) => pool.prepare(
				frame,
				&self.pipeline_manager,
				sink,
				SurfelLight {
					exposure,
					previous_exposure: self.surfel_exposure,
					environment_intensity: self.environment.intensity(),
				},
				tracing,
				hits,
			),
			_ => None,
		};
		let probes = match (indirect_diffuse, &self.probe_volume, sinks.first(), tracing, &self.ray_hits) {
			(Some(IndirectDiffuseSource::Probes), Some(volume), Some(sink), Some(tracing), Some(hits)) => volume.prepare(
				frame,
				&self.pipeline_manager,
				sink,
				self.environment.intensity(),
				tracing,
				hits,
			),
			_ => None,
		};
		// Until its pipelines compile, surfel GI or probe GI can't light a frame, and SSGI lights it when the project
		// built SSGI in.
		let indirect_diffuse = match (indirect_diffuse, surfels, probes) {
			(Some(IndirectDiffuseSource::Surfels), None, _) | (Some(IndirectDiffuseSource::Probes), _, None) => {
				self.settings.features.ssgi.then_some(IndirectDiffuseSource::ScreenSpace)
			}
			(source, ..) => source,
		};
		if let Some(pool) = self.surfel_pool.as_ref().filter(|_| surfels.is_some())
			&& frame.key().frame_index().is_multiple_of(GI_LOG_INTERVAL)
			&& let Some(counts) = pool.counts(frame)
		{
			log::debug!(
				"Surfel GI keeps {} surfels alive, {} of them spawned by rays' hits, in {} cell entries; the last grid recycled {}, and the frame before it spawned {} and lost {} while its rays' hits asked for {}. Young and changing surfels wanted {} rays and traced {}; settled ones wanted {} and traced {}.",
				counts.last_alive,
				counts.last_bounced,
				counts.cell_entries,
				counts.last_recycled,
				counts.last_spawned,
				counts.last_lost,
				counts.last_hit_spawns,
				counts.priority_wanted,
				counts.priority_rays,
				counts.regular_wanted,
				counts.regular_rays,
			);
		}
		if let Some(volume) = self.probe_volume.as_ref().filter(|_| probes.is_some())
			&& frame.key().frame_index().is_multiple_of(GI_LOG_INTERVAL)
			&& let Some(schedule) = volume.schedule(frame)
		{
			log::debug!(
				"Probe GI holds {} active, {} idle, {} buried, and {} unset probes; the frame before updated {}.",
				schedule.last_active,
				schedule.last_idle,
				schedule.last_buried,
				schedule.last_unset,
				schedule.last_count,
			);
		}
		// The acceleration structures build with the surfel grid or the probe updates, so only a frame that surfel GI or
		// probe GI lights builds them.
		let acceleration_structures = match (surfels.is_some() || probes.is_some(), &mut self.acceleration_structures) {
			(true, Some(structures)) => Some(structures.prepare(frame, &mut self.traced_primitives, frame_allocator)),
			_ => None,
		};
		drop(ray_tracing_span);
		let frame_work = FrameWork {
			skinning: &self.skinning_pass,
			mesh_work: &self.mesh_work,
			shadows: &self.shadow_maps,
			virtual_shadows: virtual_shadows.as_ref().map(|(_, data)| *data),
			acceleration_structures,
		};
		let render_info = &self.scene.render_info;
		let previously_recorded_sinks = &self.recorded_sinks;
		let recorded_exposure = self.recorded_exposure;
		let recorded_indirect_diffuse = self.recorded_indirect_diffuse;
		let mut recorded_sinks = SmallVec::<[Sink; 4]>::new();
		let mut frame_work_recorded = false;
		let screen_queries = &mut self.screen_queries;
		let count_meshlets = self.lod.settings.count_meshlets;
		let commands = sinks
			.iter()
			.filter_map(|sink| {
				let state = self.scene.sink_states.iter().find(|state| state.id == sink.index())?;
				Some((sink, &state.render_pass))
			})
			.enumerate()
			.filter_map(|(command_index, (sink, render_pass))| {
				// Frame-wide work runs once per frame, with the first sink, whose camera the views were made for. Later sinks
				// record after it in the same command buffer, so they sample the maps it rendered.
				let frame_work = (command_index == 0).then_some(frame_work);
				// A sink that did not record last frame, or was resized since, has no usable history.
				let history = previously_recorded_sinks
					.iter()
					.find(|previous| previous.index() == sink.index() && previous.extent() == sink.extent())
					.map(|previous| SinkHistory {
						view: previous.view(),
						exposure: recorded_exposure,
						indirect_diffuse: recorded_indirect_diffuse,
						read_targets: previous.read_targets(),
					});
				let _span = debug_span!("Visibility::prepare_sink", sink = sink.index()).entered();
				if count_meshlets
					&& frame.key().frame_index().is_multiple_of(GI_LOG_INTERVAL)
					&& let Some(kept) = render_pass.kept_meshlets(frame)
				{
					log_kept_meshlets(sink.index(), kept);
				}
				// The sink reads the pixels of the oldest queries that name its camera.
				let taken = sink.camera().map(|camera| screen_queries.take(camera)).unwrap_or_default();
				let points = taken
					.iter()
					.map(|(_, query)| query.point())
					.collect::<SmallVec<[[u32; 2]; 16]>>();
				let query_stages = render_pass.prepare_screen_queries(frame, &points);
				let command = render_pass.prepare(
					frame,
					sink,
					frame_work,
					dispatches,
					render_info,
					&shadow_work,
					history,
					exposure,
					indirect_diffuse,
					surfels,
					probes,
					self.gtao.settings,
					self.contact_shadow.settings,
					self.fog.as_ref(),
					&mut *background,
					query_stages,
					count_meshlets,
					frame_allocator,
				);
				// Queries a frame doesn't read wait for the next one, ahead of newer queries.
				if command.is_some() && query_stages.is_some() {
					screen_queries.record(sequence, sink.index(), &taken);
				} else {
					screen_queries.put_back(taken);
				}
				let command = command?;
				recorded_sinks.push(*sink);
				frame_work_recorded |= command_index == 0;
				Some((sink.index(), allocate_render_command(frame_allocator, command)))
			})
			.collect();
		// The reads name instance slots, which the answers turn into the entities the slots held in this frame.
		if self.screen_queries.has_recorded(sequence) {
			let entities = &self.scene.render_entities;
			self.screen_queries.remember_slots(
				sequence,
				(0..entities.slots_len()).map(|slot| entities.get_slot(slot).map(|entity| entity.handle)),
			);
		}
		if frame_work_recorded {
			self.shadow_states = shadow_states;
			if let Some((levels, _)) = virtual_shadows {
				self.virtual_shadow_levels = Some(levels);
				self.virtual_shadow_changes = 0..0;
				self.virtual_shadow_records = self.instance_kept.len();
			}
			// The frame-wide work stored the surfels' light at this frame's exposure.
			if surfels.is_some() {
				self.surfel_exposure = exposure;
			}
		}
		// The sink that records the frame-wide work records the builds the acceleration structures picked, if any.
		if let Some(structures) = &mut self.acceleration_structures {
			structures.commit(frame_work_recorded);
		}
		// The frame-wide work records the UV copies of new posed ranges with the skinning dispatches.
		self.posed_uv_copies_recorded = frame_work_recorded;
		self.recorded_sinks = recorded_sinks;
		self.recorded_exposure = exposure;
		self.recorded_indirect_diffuse = indirect_diffuse;
		commands
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		// Lit and depth keep a copy per frame in flight: reflection rays read the light and depth the previous frame left.
		let lit = render_pass_builder.create_history_target(
			ghi::image::Builder::new(
				crate::rendering::SCENE_COLOR_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Lit"),
			1,
		);
		let depth = render_pass_builder.create_history_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Depth")
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
			1,
		);
		let primitive_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Image).name("Primitive Index"),
		);
		let instance_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Image).name("Instance Index"),
		);
		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Lit", "main");
		render_pass_builder.create_scene_background(crate::rendering::render_pass::SceneBackgroundTargets {
			color: lit.into(),
			depth: depth.into(),
		});
		let indirect_diffuse = create_indirect_diffuse_targets(render_pass_builder);
		let diffuse_radiance = self
			.settings
			.features
			.ssgi
			.then(|| create_diffuse_radiance_target(render_pass_builder));
		let sun_visibility = create_sun_visibility_targets(render_pass_builder);
		let gtao = create_gtao_targets(render_pass_builder);
		let fog_light = create_fog_light_target(render_pass_builder);
		let backdrop = create_backdrop_targets(render_pass_builder);
		let subsurface = self
			.settings
			.features
			.subsurface
			.then(|| create_subsurface_targets(render_pass_builder));
		let velocity = create_velocity_targets(render_pass_builder);
		let stage_counters = super::render_pass::StageCounters::new(render_pass_builder);
		let surfel_gi = self.surfel_pool.is_some();
		let surfels = surfel_gi.then(|| create_surfel_targets(render_pass_builder));
		let probe_gi = self.probe_volume.is_some();
		// A pass declares at most eight views, so the two after material evaluation's are surfel GI's when it is built
		// in, and probe GI's otherwise.
		let debug_views = render_pass_builder.debug_views(if surfel_gi {
			&SURFEL_DEBUG_VIEW_NAMES
		} else if probe_gi {
			&PROBE_DEBUG_VIEW_NAMES
		} else {
			&super::shader_generator::MATERIAL_DEBUG_VIEW_NAMES
		});

		let context = render_pass_builder.context();
		let render_pass = VisibilityRenderPass::new(
			context,
			self.pipeline_manager.clone(),
			self.scene.descriptor_set,
			self.scene.views_buffer.into(),
			self.scene.lighting_data,
			SinkTargets {
				lit,
				depth,
				primitive_index: primitive_index.into(),
				instance_index: instance_index.into(),
				indirect_diffuse,
				diffuse_radiance,
				surfels,
				sun_visibility,
				gtao,
				fog_light,
				backdrop,
				subsurface,
				velocity,
			},
			&self.shadow_maps,
			stage_counters,
			self.settings.features,
			self.settings.instance_capacity.meshlet_groups,
			probe_gi,
			debug_views,
			self.blue_noise,
			self.geometry.previous_vertex_positions,
			self.settings.instance_capacity.instances,
		);
		context.write(&self.environment.bound.descriptor_writes(render_pass.material_evaluation()));
		self.scene.sink_states.push(SinkState {
			id: sink_id,
			render_pass,
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::rendering::lights::{LightColor, PhotometricIntensity, PointLight};

	#[test]
	fn resolved_ies_profile_applies_the_per_light_dimmer() {
		let profile_light = Lights::Point(
			PointLight::new(
				LightColor::LinearSrgb([1.0, 1.0, 1.0]),
				PhotometricIntensity::Ies {
					profile: "lights/office.ies".into(),
					dimmer: 0.5,
				},
			)
			.expect("physical IES point light"),
		);
		let analytic_light = Lights::Point(
			PointLight::new(
				LightColor::Kelvin(4_500.0),
				PhotometricIntensity::LuminousIntensity {
					candela: 100.0,
					reference_distance_m: 1.0,
				},
			)
			.expect("physical point light"),
		);
		let profile = IesProfileTexture {
			profile_index: 19,
			intensity_scale_candela: 180.0,
		};
		let mut profiles = HashMap::default();

		assert_eq!(resolve_ies_profile(&profile_light, &profiles), (0.5, None));
		assert_eq!(resolve_ies_profile(&analytic_light, &profiles), (1.0, None));
		profiles.insert("lights/office.ies".to_string(), profile);

		assert_eq!(
			resolve_ies_profile(&profile_light, &profiles),
			(
				90.0,
				Some(IesProfileTexture {
					profile_index: 19,
					intensity_scale_candela: 90.0,
				})
			)
		);
		assert_eq!(resolve_ies_profile(&analytic_light, &profiles), (1.0, None));
	}

	#[test]
	fn gi_technique_lights_the_frame_when_built_in_and_enabled() {
		use IndirectDiffuseSource::{Probes, ScreenSpace, Surfels};

		let features = |ssgi, surfel_gi, probe_gi| VisibilityFeatures {
			ssgi,
			surfel_gi,
			probe_gi,
			..VisibilityFeatures::default()
		};
		let gi = |enabled, technique| GiSettings {
			enabled,
			technique,
			..GiSettings::new(VisibilityFeatures::default())
		};
		// (GI settings, built-in SSGI, surfel GI, and probe GI, ray tracing) and the source that lights the frame.
		let cases = [
			((gi(true, Surfels), (true, true, false), true), Some(Surfels)),
			((gi(true, Surfels), (false, true, false), true), Some(Surfels)),
			((gi(true, ScreenSpace), (true, true, false), true), Some(ScreenSpace)),
			((gi(true, Probes), (false, false, true), true), Some(Probes)),
			((gi(true, Probes), (true, true, true), true), Some(Probes)),
			// Without ray tracing, SSGI lights frames in place of surfel GI or probe GI when it is built in.
			((gi(true, Surfels), (true, true, false), false), Some(ScreenSpace)),
			((gi(true, Probes), (true, false, true), false), Some(ScreenSpace)),
			((gi(true, Surfels), (false, true, false), false), None),
			((gi(true, Probes), (false, false, true), false), None),
			// A technique left out of the shaders lights nothing.
			((gi(true, ScreenSpace), (false, true, false), true), None),
			((gi(true, Probes), (true, true, false), true), None),
			((gi(false, ScreenSpace), (true, true, false), true), None),
			((gi(false, Surfels), (true, true, false), true), None),
			((gi(false, Probes), (false, false, true), true), None),
		];
		for ((settings, (ssgi, surfel_gi, probe_gi), ray_tracing), source) in cases {
			assert_eq!(
				indirect_diffuse_source(settings, features(ssgi, surfel_gi, probe_gi), ray_tracing),
				source,
				"{settings:?}, SSGI {ssgi}, surfel GI {surfel_gi}, probe GI {probe_gi}, ray tracing {ray_tracing}"
			);
		}

		// Lightmaps were traced in the bake, so they light frames on a device that doesn't trace rays.
		let baked = VisibilityFeatures {
			baked_gi: true,
			..VisibilityFeatures::default()
		};
		let lightmaps = gi(true, IndirectDiffuseSource::Baked);
		assert_eq!(
			indirect_diffuse_source(lightmaps, baked, false),
			Some(IndirectDiffuseSource::Baked)
		);
		assert_eq!(indirect_diffuse_source(lightmaps, VisibilityFeatures::default(), true), None);
	}

	/// Verifies the GI sizes are read from the render parameters, and that each is accepted at the ends of its range and
	/// rejected past them with the value and the rule it breaks.
	#[test]
	fn gi_sizes_are_read_from_the_render_parameters_and_kept_in_range() {
		use crate::application::Parameter;

		let settings = VisibilityPipelineSettings::from_parameters(
			&[
				Parameter::new(SURFEL_CAPACITY_PARAMETER, "131072"),
				Parameter::new(SURFEL_RAY_BUDGET_PARAMETER, "32768"),
				Parameter::new(PROBE_RAY_BUDGET_PARAMETER, "32768"),
				Parameter::new(PROBE_SPACING_PARAMETER, "2.5"),
			][..],
		);
		assert_eq!(
			(
				settings.surfel_capacity,
				settings.surfel_ray_budget,
				settings.probe_ray_budget,
				settings.probe_spacing
			),
			(131_072, 32_768, 32_768, 2.5)
		);

		let defaults = VisibilityPipelineSettings::default();
		let surfels = |surfel_capacity| VisibilityPipelineSettings {
			surfel_capacity,
			..defaults
		};
		let surfel_rays = |surfel_ray_budget| VisibilityPipelineSettings {
			surfel_ray_budget,
			..defaults
		};
		let probe_rays = |probe_ray_budget| VisibilityPipelineSettings {
			probe_ray_budget,
			..defaults
		};
		let spacing = |probe_spacing| VisibilityPipelineSettings {
			probe_spacing,
			..defaults
		};
		for accepted in [
			surfels(1_024),
			surfels(16_777_216),
			surfel_rays(1_024),
			surfel_rays(1_048_576),
			probe_rays(128),
			probe_rays(1_048_576),
			spacing(0.05),
			spacing(100.0),
		] {
			assert_eq!(accepted.validate(), Ok(()), "{accepted:?}");
		}
		let rejected = [0, 512, 3_000, 33_554_432]
			.map(|capacity| (surfels(capacity), format!("{capacity} is not a power of two")))
			.into_iter()
			.chain([0, 1_000, 1_049_600].map(|rays| (surfel_rays(rays), format!("{rays} is not a multiple of 1024"))))
			.chain([0, 100, 1_048_704].map(|rays| (probe_rays(rays), format!("{rays} is not a multiple of 128"))))
			.chain([0.01, 101.0, f32::NAN].map(|meters| (spacing(meters), "is outside 0.05 through 100 meters".to_owned())));
		for (settings, reason) in rejected {
			let error = settings.validate().unwrap_err();
			assert!(error.contains(&reason), "{error}");
		}
	}
}
