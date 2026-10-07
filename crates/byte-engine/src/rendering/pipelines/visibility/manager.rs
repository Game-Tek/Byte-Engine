//! Owns the visibility scene, adopts asynchronously loaded resources, and prepares each frame's passes.
//!
//! Loading completion is not the same as scene readiness: a renderable enters a frame only when its mesh, every
//! material, and every texture those materials sample are resident. [`VisibilityPipelineManager`] tracks that
//! closure in an availability graph and rebuilds the frame's instance lists from it.
//! It requests resources through the visibility loader façade and consumes only ready or unavailable domain
//! events; worker protocol and compilation state do not cross into this renderer-owned scene layer.

use std::sync::Arc;

use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use log::{error, warn};
use resource_management::resources::skeleton::SkinBinding;
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::hash::HashMap;
use utils::{AvailabilityGraph, Extent, StableVec};

use super::geometry::{GeometryCapacity, GeometryHandles, MeshData};
use super::layout::{
	CONE_SHADOW_VIEW_OFFSET, DEFAULT_SHADOW_MAP_BUDGET_MIB, DEFAULT_SHADOW_MAP_RESOLUTION, ENVIRONMENT_BINDING,
	MATERIAL_EVALUATIONS_BINDING, MATERIALS_DATA_BINDING, MAX_BINDLESS_TEXTURES, MAX_INSTANCES, MAX_LIGHTS,
	MAX_MATERIAL_TEXTURES, MAX_MATERIALS, MESH_DATA_BINDING, MESHLET_DATA_BINDING, NO_EVALUATION, POINT_SHADOW_FACE_COUNT,
	POINT_SHADOW_VIEW_OFFSET, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SKINNED_VERTICES_BINDING,
	SPECULAR_ENVIRONMENT_BINDING, TEXTURES_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use super::loader::{ResidentEnvironment, ResidentMaterial, ResidentTexture, VisibilityLoaderClient, VisibilityLoaderEvent};
use super::mesh_dispatch::MeshDispatchWorkBuffer;
use super::render_pass::{
	CONTACT_SHADOWS_CONFIGURATION_PREFIX, ContactShadowSettings, GTAO_CONFIGURATION_PREFIX, GtaoSettings,
	SSGI_CONFIGURATION_PREFIX, ShadowMaps, ShadowWork, SinkHistory, SinkTargets, SsgiSettings, SunCascades,
	VisibilityRenderPass, create_radiance_history_target, create_ssgi_targets, create_sun_visibility_targets,
};
use super::scene::{Instance, MaterialEvaluation, RenderEntity, RenderSkin, SinkState, VisibilityScene};
use super::shader_data::{IesProfileTexture, MESH_FLAG_DOUBLE_SIDED, MaterialData, ShaderMesh, ShaderViewData};
use super::shadow_selection::{
	SHADOW_DEFAULT_EXPOSURE_SCALE, ShadowBudget, ShadowLayout, ShadowLightSelection, make_cone_shadow_view,
	make_point_shadow_view, retain_layout, select_shadow_lights, sun_cascade_view,
};
use super::skinning::{
	DualQuaternion, MAX_SKINNED_VERTICES, MAX_SKINNING_MATRICES, SkinningDispatch, SkinningPaletteKind, SkinningPass,
	append_dual_quaternion_palette,
};
use crate::core::factory::{CreateMessage, Handle};
use crate::core::listener::{DefaultListener, Listener as _};
use crate::core::message::DeleteMessage;
use crate::gameplay::Transform;
use crate::gameplay::transform::TransformationUpdate;
use crate::gameplay::world::DefaultWorld;
use crate::rendering::csm::{self, CascadeFitting, CascadeSplits};
use crate::rendering::lights::{ConeLight, DirectionalLight, Lights, LocalEmission, PointLight};
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::renderable::mesh::MeshKey;
use crate::rendering::{Environment, PipelineManagerClient, RenderableMesh, Resource, Sink, UpdatePose, View};

/// The startup parameters that set the local-light shadow pool capacities.
/// The startup parameter that sets the shadow-map memory every light shares, in mebibytes. See
/// [`VisibilityPipelineSettings::with_shadow_map_budget_mib`].
pub const SHADOW_MAP_BUDGET_PARAMETER: &str = "render.shadow-maps.budget-mb";
/// The prefix of the startup parameters that set each scene-wide geometry buffer's element count, such as
/// `render.geometry.triangle-capacity`. See [`GeometryCapacity`].
pub const GEOMETRY_CAPACITY_PARAMETER_PREFIX: &str = "render.geometry.";
/// The startup parameters that set how far directional shadows reach, in meters, and the share of their cascade
/// splits that is logarithmic. See [`CascadeSplits`].
pub const DIRECTIONAL_SHADOW_DISTANCE_PARAMETER: &str = "render.directional-shadows.distance";
pub const DIRECTIONAL_SHADOW_SPLIT_BLEND_PARAMETER: &str = "render.directional-shadows.split-blend";
/// The startup parameter that sets what directional shadow cascades cover: `receivers` or `frustum`. See
/// [`CascadeFitting`].
pub const DIRECTIONAL_SHADOW_FITTING_PARAMETER: &str = "render.directional-shadows.fitting";
/// The startup parameter that sets the directional cascades' resolution in texels per side. See
/// [`VisibilityPipelineSettings::with_directional_shadow_map_resolution`].
pub const DIRECTIONAL_SHADOW_RESOLUTION_PARAMETER: &str = "render.directional-shadows.resolution";

/// The `VisibilityPipelineSettings` struct configures memory limits and shadow coverage for the visibility rendering
/// pipeline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VisibilityPipelineSettings {
	geometry_capacity: GeometryCapacity,
	shadow_map_budget_mib: u32,
	cascade_splits: CascadeSplits,
	cascade_fitting: CascadeFitting,
	directional_shadow_map_resolution: u32,
}

impl Default for VisibilityPipelineSettings {
	fn default() -> Self {
		Self {
			geometry_capacity: GeometryCapacity::default(),
			shadow_map_budget_mib: DEFAULT_SHADOW_MAP_BUDGET_MIB,
			cascade_splits: CascadeSplits::default(),
			cascade_fitting: CascadeFitting::default(),
			directional_shadow_map_resolution: DEFAULT_SHADOW_MAP_RESOLUTION,
		}
	}
}

impl VisibilityPipelineSettings {
	/// Sets how many vertices, triangles, and meshlets the scene-wide geometry buffers hold.
	///
	/// Meshes that would overflow a buffer are rejected at upload, so size this for the largest resident scene.
	pub fn with_geometry_capacity(mut self, capacity: GeometryCapacity) -> Result<Self, String> {
		capacity.validate()?;
		self.geometry_capacity = capacity;
		Ok(self)
	}

	pub fn geometry_capacity(&self) -> GeometryCapacity {
		self.geometry_capacity
	}

	/// Sets how far directional shadows reach from the camera and how their cascades divide that range.
	pub fn with_cascade_splits(mut self, cascade_splits: CascadeSplits) -> Self {
		self.cascade_splits = cascade_splits;
		self
	}

	pub fn cascade_splits(&self) -> CascadeSplits {
		self.cascade_splits
	}

	/// Sets what each directional shadow cascade covers.
	pub fn with_cascade_fitting(mut self, cascade_fitting: CascadeFitting) -> Self {
		self.cascade_fitting = cascade_fitting;
		self
	}

	pub fn cascade_fitting(&self) -> CascadeFitting {
		self.cascade_fitting
	}

	/// Sets the directional cascades' resolution in texels per side. Halving it quarters the shadow map raster and
	/// depth pyramid work and doubles the size of a shadow texel on the ground.
	///
	/// # Errors
	///
	/// Returns an error unless `resolution` is a positive multiple of 16, which the cascade depth pyramid reduces in
	/// whole cells.
	pub fn with_directional_shadow_map_resolution(mut self, resolution: u32) -> Result<Self, String> {
		if resolution == 0 || !resolution.is_multiple_of(16) {
			return Err(format!(
				"Directional shadow resolution was not set. The most likely cause is that {resolution} is not a positive multiple of 16 texels."
			));
		}
		self.directional_shadow_map_resolution = resolution;
		Ok(self)
	}

	/// Sets the shadow-map memory, in mebibytes, that every shadow-casting light shares.
	///
	/// The first directional light takes its cascades first, cone and point lights then share what remains by how much
	/// of the screen they light, and other directional lights take what is left after them. Lights that do not fit stay
	/// lit without shadows. One sun takes 32 MiB at the default cascade resolution, four times that per doubling of
	/// [`Self::with_directional_shadow_map_resolution`]; a cone light takes 2 MiB and a point light 12 MiB.
	///
	/// # Errors
	///
	/// Returns an error when `mebibytes` is zero, which would leave every light without shadows.
	pub fn with_shadow_map_budget_mib(mut self, mebibytes: u32) -> Result<Self, String> {
		if mebibytes == 0 {
			return Err(
				"Shadow map budget was not set. The most likely cause is a budget of 0 MiB, which holds no shadow map."
					.to_owned(),
			);
		}
		self.shadow_map_budget_mib = mebibytes;
		Ok(self)
	}

	pub fn shadow_map_budget_mib(&self) -> u32 {
		self.shadow_map_budget_mib
	}

	/// Returns what one light of each kind costs from the shadow-map budget.
	fn shadow_budget(&self) -> ShadowBudget {
		ShadowBudget::new(self.shadow_map_budget_mib, self.directional_shadow_map_resolution)
	}
}

/// Keys of the readiness graph: a renderable is ready when its materials are, a material when its textures are.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Availability {
	Renderable(Handle),
	Material(u32),
	Texture(u32),
}

/// A renderable waiting for its mesh resource.
struct PendingRenderable {
	handle: Handle,
	mesh_key: MeshKey,
}

/// One material's render-thread pipeline and authored alpha and sidedness contract.
struct LoadedMaterial {
	index: u32,
	pipeline: ghi::PipelineHandle,
	/// Shared with the material lists, so rebuilding them does not copy names.
	name: Arc<str>,
	alpha_mode: AlphaMode,
	double_sided: bool,
	texture_indices: Vec<u32>,
}

impl ResidentEnvironment {
	fn descriptor_writes(self, descriptor_set: ghi::DescriptorSetHandle) -> [ghi::DescriptorWrite; 2] {
		[
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				ENVIRONMENT_BINDING.slot(),
				self.diffuse_image,
				self.sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				SPECULAR_ENVIRONMENT_BINDING.slot(),
				self.specular_image,
				self.sampler,
				ghi::Layouts::Read,
			),
		]
	}
}

/// Creates the opaque black environment sampled while no HDR environment is configured or its upload is pending.
fn create_fallback_environment(context: &mut ghi::implementation::Context, sampler: ghi::SamplerHandle) -> ResidentEnvironment {
	let image = context.build_image(
		ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name("Visibility Environment Fallback")
			.extent(Extent::square(1))
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	// Opaque alpha keeps material evaluation on this black environment instead of the analytical fallback
	// reserved for explicitly transparent environment texels.
	context.get_texture_slice_mut(image).copy_from_slice(&[0, 0, 0, u8::MAX]);
	context.sync_texture(image);
	ResidentEnvironment {
		diffuse_image: image.into(),
		specular_image: image.into(),
		sampler,
		upward_illuminance: 0.0,
	}
}

/// Environment selection: the requested resource, every environment that finished uploading, and what is bound.
struct EnvironmentState {
	requested: Option<String>,
	/// The lux the requested environment should deliver to an upward-facing surface, if it's calibrated.
	illuminance: Option<f32>,
	bound: ResidentEnvironment,
	/// The bound environment changed and existing sinks must be rewritten.
	descriptors_dirty: bool,
}

impl EnvironmentState {
	fn bind(&mut self, environment: ResidentEnvironment) {
		self.bound = environment;
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
	/// The palette uploaded this frame for each renderable and skin binding, shared by every primitive that uses it.
	cache: HashMap<(Handle, *const SkinBinding), (u32, SkinningPaletteKind)>,
}

impl SkinningFrame {
	/// Frame caches retain capacity but never retain entity or resource pointers beyond one rebuild.
	fn clear(&mut self) {
		self.matrices.clear();
		self.dual_quaternions.clear();
		self.cache.clear();
	}

	/// Returns the palette range for one renderable's binding, uploading it once per frame.
	///
	/// Rigid poses are converted to dual quaternions; everything else keeps the matrix palette.
	fn palette(
		&mut self,
		handle: Handle,
		binding: &Arc<SkinBinding>,
		pose: &[math::AffineMatrix],
	) -> Option<(u32, SkinningPaletteKind)> {
		let binding_ptr = Arc::as_ptr(binding);
		if let Some(palette) = self.cache.get(&(handle, binding_ptr)) {
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
		self.cache.insert((handle, binding_ptr), (palette_base as u32, kind));
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

/// Applies queued runtime settings under `prefix`.
///
/// Each update is answered: parameters outside `prefix` with `namespace_error`, others with the effective value or
/// the reason `with_parameter` rejected them.
fn drain_settings<S: Copy>(
	port: &crate::configuration::ConfigurationPort,
	prefix: &str,
	namespace_error: &'static str,
	settings: &mut S,
	with_parameter: impl Fn(
		S,
		&str,
		&crate::configuration::ConfigurationValue,
	) -> Result<(S, crate::configuration::ConfigurationValue), String>,
) {
	while let Some(update) = port.read() {
		let Some(parameter) = update.parameter().strip_prefix(prefix) else {
			port.not_set(update.id(), namespace_error);
			continue;
		};
		match with_parameter(*settings, parameter, update.value()) {
			Ok((updated, effective_value)) => {
				*settings = updated;
				port.set(update.id(), effective_value);
			}
			Err(reason) => port.not_set(update.id(), reason),
		}
	}
}

/// The `TransformSamples` struct keeps the two most recent transforms of one renderable and which step the latest
/// arrived in.
struct TransformSamples {
	previous: Transform,
	current: Transform,
	/// The step `current` was published in. Older than the latest step once the renderable stopped publishing.
	step: u64,
	/// Whether a frame still has to show the latest state.
	dirty: bool,
}

impl TransformSamples {
	/// Starts both samples at rest at `transform`, so the renderable is shown there until a step moves it. Next, call
	/// [`Self::sample`] or [`Self::snap`] to queue it for a frame.
	fn snapped(transform: &Transform) -> Self {
		Self {
			previous: transform.clone(),
			current: transform.clone(),
			step: 0,
			dirty: false,
		}
	}

	/// Keeps `transform` as the state at the end of `step`.
	///
	/// A second sample within the same step replaces the first, and a renderable that skipped steps starts the
	/// new segment from where it stood still.
	fn sample(&mut self, transform: &Transform, step: u64) {
		if self.step != step {
			self.previous = self.current.clone();
			self.step = step;
		}
		self.current = transform.clone();
		self.dirty = true;
	}

	/// Shows `transform` at once: a write made outside a step is a placement, not motion.
	fn snap(&mut self, transform: &Transform) {
		self.previous = transform.clone();
		self.current = transform.clone();
		self.dirty = true;
	}

	/// Returns the transform a frame should show, or `None` when the last shown one still stands.
	///
	/// A renderable published by the latest step (`step`) moves from `previous` to `current` with `alpha`; once a
	/// step passes without a sample it rests at `current` and stops reporting.
	fn shown(&mut self, step: u64, alpha: f32) -> Option<Transform> {
		if !self.dirty {
			return None;
		}
		if self.step == step {
			Some(self.previous.interpolate(&self.current, alpha))
		} else {
			self.dirty = false;
			Some(self.current.clone())
		}
	}
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
	/// Transform updates consumed after resource completions and before instance rebuilds.
	transforms_listener: DefaultListener<TransformationUpdate>,
	/// World creation messages adopted by [`PipelineManager::update`].
	resource_listener: DefaultListener<CreateMessage<Resource>>,
	cone_light_listener: DefaultListener<CreateMessage<ConeLight>>,
	directional_light_listener: DefaultListener<CreateMessage<DirectionalLight>>,
	point_light_listener: DefaultListener<CreateMessage<PointLight>>,
	mesh_listener: DefaultListener<CreateMessage<RenderableMesh>>,
	environment_listener: DefaultListener<CreateMessage<Environment>>,
	/// World edits adopted at the start of [`PipelineManager::prepare`].
	pose_listener: DefaultListener<UpdatePose>,
	deletions_listener: DefaultListener<DeleteMessage>,
	/// Canonical material table, copied into each frame-local buffer after it changes.
	materials: Box<[MaterialData; MAX_MATERIALS]>,
	materials_buffer: ghi::DynamicBufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Canonical evaluation slot of every material table entry, which [`Self::rebuild_material_lists`] assigns. It is
	/// copied into each frame-local buffer together with `materials`.
	material_evaluations: Box<[u32; MAX_MATERIALS]>,
	material_evaluations_buffer: ghi::DynamicBufferHandle<[u32; MAX_MATERIALS]>,
	/// Which frame sequences' copies of the material and evaluation buffers hold the current tables. Every frame
	/// sequence keeps its own copy, so a change reaches each copy on a frame of that copy's sequence.
	materials_copies_current: [bool; ghi::MAX_FRAMES_IN_FLIGHT],
	mesh_dispatch_work: MeshDispatchWorkBuffer,
	skinning_pass: SkinningPass,
	skinning_frame: SkinningFrame,
	pending_renderables: Vec<PendingRenderable>,
	/// The two most recent transform samples per renderable, retained so a mesh that loads later starts in the
	/// right place and so frames can show where a renderable was between two steps.
	renderable_transforms: HashMap<Handle, TransformSamples>,
	/// The handles whose samples a frame still has to show, so frames skip renderables at rest.
	dirty_transforms: Vec<Handle>,
	/// The number of simulation steps marked so far; a sample tagged with it belongs to the latest step.
	step_count: u64,
	/// Adopted materials indexed by material table slot.
	loaded_materials: Vec<Option<LoadedMaterial>>,
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
	gtao_configuration: crate::configuration::ConfigurationPort,
	gtao_settings: GtaoSettings,
	ssgi_configuration: crate::configuration::ConfigurationPort,
	ssgi_settings: SsgiSettings,
	contact_shadow_configuration: crate::configuration::ConfigurationPort,
	contact_shadow_settings: ContactShadowSettings,
	/// The sinks whose visibility pass recorded in the previous frame, with the view and extent they used. Only their
	/// per-frame images hold usable history, and only while the extent is unchanged.
	recorded_sinks: SmallVec<[Sink; 4]>,
	/// The exposure the previous frame's light was multiplied by. Every recorded sink shares it.
	recorded_exposure: f32,
	/// Whether the previous frame's recorded sinks ran SSGI, so their SSGI images hold history.
	recorded_ssgi: bool,
	/// Whether the light-count and shadow-budget warnings were reported while their limit stays exceeded.
	reported_limits: [bool; 2],
	pub(crate) scene: VisibilityScene,
}

impl VisibilityPipelineManager {
	/// Creates the scene buffers and base descriptor set around an already running resource client.
	///
	/// The manager subscribes to `world`'s resources, lights, meshes, environments, poses, transforms, and
	/// deletions here, so it sees every message published after setup.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		world: &DefaultWorld,
		geometry: GeometryHandles,
		loader: VisibilityLoaderClient,
		pipeline_manager: PipelineManagerClient,
		gtao_configuration: crate::configuration::ConfigurationPort,
		ssgi_configuration: crate::configuration::ConfigurationPort,
		contact_shadow_configuration: crate::configuration::ConfigurationPort,
		settings: VisibilityPipelineSettings,
	) -> Self {
		let environment = create_fallback_environment(context, loader.environment_sampler);
		let skinning_pass = SkinningPass::new(context, &pipeline_manager, geometry);
		let host_buffer = |name, uses| {
			ghi::buffer::Builder::new(uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
		};
		let materials_buffer = context.build_dynamic_buffer(host_buffer(
			"Materials Data",
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
		));
		let material_evaluations_buffer = context.build_dynamic_buffer(host_buffer(
			"Material Evaluations",
			ghi::Uses::Storage | ghi::Uses::TransferDestination,
		));
		let views_buffer = context.build_dynamic_buffer(host_buffer("Visibility Views Data", ghi::Uses::Storage));
		let meshes_buffer = context.build_dynamic_buffer(host_buffer("Visibility Meshes Data", ghi::Uses::Storage));
		let lighting_buffer =
			context.build_dynamic_buffer(host_buffer("Light Data", ghi::Uses::Storage | ghi::Uses::TransferDestination));
		let descriptor_set = context.create_descriptor_set(Some("Base Descriptor Set"));
		let mesh_dispatch_work = MeshDispatchWorkBuffer::new(context, descriptor_set);
		let shadow_maps = ShadowMaps::new(context, &pipeline_manager, settings.directional_shadow_map_resolution);
		let write = |binding: ghi::ShaderResourceDescriptor, buffer| {
			ghi::DescriptorWrite::buffer(descriptor_set, binding.slot(), buffer)
		};
		context.write(&[
			write(VIEWS_DATA_BINDING, views_buffer.into()),
			write(MESH_DATA_BINDING, meshes_buffer.into()),
			write(VERTEX_POSITIONS_BINDING, geometry.vertex_positions.into()),
			write(VERTEX_NORMALS_BINDING, geometry.vertex_normals.into()),
			write(SKINNED_VERTICES_BINDING, skinning_pass.skinned_vertices_buffer.into()),
			write(VERTEX_UV_BINDING, geometry.vertex_uvs.into()),
			write(VERTEX_INDICES_BINDING, geometry.vertex_indices.into()),
			write(PRIMITIVE_INDICES_BINDING, geometry.primitive_indices.into()),
			write(MESHLET_DATA_BINDING, geometry.meshlets.into()),
			write(MATERIALS_DATA_BINDING, materials_buffer.into()),
			write(MATERIAL_EVALUATIONS_BINDING, material_evaluations_buffer.into()),
		]);
		Self {
			loader,
			resource_events: Vec::new(),
			pipeline_manager,
			transforms_listener: world.transforms_channel().listener(),
			resource_listener: world.factory::<Resource>().listener(),
			cone_light_listener: world.factory::<ConeLight>().listener(),
			directional_light_listener: world.factory::<DirectionalLight>().listener(),
			point_light_listener: world.factory::<PointLight>().listener(),
			mesh_listener: world.factory::<RenderableMesh>().listener(),
			environment_listener: world.factory::<Environment>().listener(),
			pose_listener: world.poses_channel().listener(),
			deletions_listener: world.deletions_listener(),
			materials: Box::new([MaterialData::default(); MAX_MATERIALS]),
			materials_buffer,
			material_evaluations: Box::new([NO_EVALUATION; MAX_MATERIALS]),
			material_evaluations_buffer,
			// The default tables must reach every frame sequence too.
			materials_copies_current: [false; ghi::MAX_FRAMES_IN_FLIGHT],
			mesh_dispatch_work,
			skinning_pass,
			skinning_frame: SkinningFrame::default(),
			pending_renderables: Vec::new(),
			renderable_transforms: HashMap::default(),
			dirty_transforms: Vec::new(),
			step_count: 0,
			loaded_materials: Vec::new(),
			loaded_ies_profiles: HashMap::default(),
			availability: AvailabilityGraph::with_capacity(
				MAX_INSTANCES + MAX_MATERIALS + MAX_BINDLESS_TEXTURES,
				MAX_INSTANCES + MAX_MATERIALS * MAX_MATERIAL_TEXTURES,
			),
			environment: EnvironmentState {
				requested: None,
				illuminance: None,
				bound: environment,
				descriptors_dirty: false,
			},
			settings,
			shadow_maps,
			shadow_layout: ShadowLayout::default(),
			gtao_configuration,
			gtao_settings: GtaoSettings::default(),
			ssgi_configuration,
			ssgi_settings: SsgiSettings::default(),
			contact_shadow_configuration,
			contact_shadow_settings: ContactShadowSettings::default(),
			recorded_sinks: SmallVec::new(),
			recorded_exposure: 1.0,
			recorded_ssgi: false,
			reported_limits: [false; 2],
			scene: VisibilityScene {
				render_entities: StableVec::new(),
				skinning_poses: HashMap::default(),
				render_entity_handles: HashMap::default(),
				lights: StableVec::new(),
				light_handles: HashMap::default(),
				descriptor_set,
				views_buffer,
				meshes_buffer,
				lighting_buffer,
				render_info: Default::default(),
				sink_states: Vec::new(),
			},
		}
	}

	/* Scene changes */

	/// Keeps a published transform as a sample of `step`, or as a placement when it arrived outside a step, and
	/// queues the handle for the next frame when it was at rest.
	fn record_transform(&mut self, handle: Handle, transform: &Transform, step: Option<u64>) {
		let samples = self
			.renderable_transforms
			.entry(handle)
			.or_insert_with(|| TransformSamples::snapped(transform));
		if !samples.dirty {
			self.dirty_transforms.push(handle);
		}
		match step {
			Some(step) => samples.sample(transform, step),
			None => samples.snap(transform),
		}
	}

	/// Shows transforms written outside a step where they are, then moves every renderable that changed since its
	/// last shown transform to where it lies `alpha` of the way between its two samples.
	fn process_transform_updates(&mut self, alpha: f32) {
		while let Some(message) = self.transforms_listener.read() {
			self.record_transform(message.handle(), message.transform(), None);
		}
		let (transforms, scene, step) = (&mut self.renderable_transforms, &mut self.scene, self.step_count);
		// A handle leaves the queue once its latest state is shown, or when it was removed.
		self.dirty_transforms.retain(|handle| {
			let Some(samples) = transforms.get_mut(handle) else {
				return false;
			};
			if let Some(transform) = samples.shown(step, alpha) {
				scene.update_transform(*handle, &transform);
			}
			samples.dirty
		});
	}

	/// Applies the frame's scene edits before anything is drawn: poses, then transforms at `alpha`, then deletions.
	fn adopt_scene_updates(&mut self, alpha: f32) {
		while let Some(message) = self.pose_listener.read() {
			self.scene.write_skinned_pose(message.handle(), message.global_matrices());
		}
		self.process_transform_updates(alpha);
		while let Some(message) = self.deletions_listener.read() {
			let handle = message.into_handle();
			self.scene.remove_light(handle);
			// A deleted renderable also drops the transform retained for its asynchronous creation.
			self.remove_mesh_instance(handle);
			self.renderable_transforms.remove(&handle);
		}
	}

	/// Adds one light and requests its optional photometric texture dependency.
	fn create_light(&mut self, handle: Handle, light: Lights) {
		if let Some(profile) = light.local().and_then(LocalEmission::ies_profile) {
			self.loader.request_texture(profile.resource_id().to_owned());
		}
		self.scene.add_light(handle, light);
	}

	/// Selects an environment and requests its baked lighting resources.
	fn create_environment(&mut self, environment: Environment) {
		let id = environment.resource_id().to_owned();
		self.environment.requested = Some(id.clone());
		self.environment.illuminance = environment.illuminance();
		if let Some(resident) = self.loader.request_environment(id) {
			self.environment.bind(resident);
		}
	}

	/// Requests the renderable's mesh and keeps the scene instance pending until the mesh is resident.
	fn request_mesh(&mut self, handle: Handle, renderable: RenderableMesh) {
		// Creation messages are upserts, but the latest independently published transform must survive replacement.
		self.remove_mesh_instance(handle);
		let source = renderable.source().clone();
		let (mesh_key, resident) = self.loader.request_mesh(source);
		if let Some(mesh) = resident {
			self.add_renderable(handle, &mesh);
		} else {
			self.pending_renderables.push(PendingRenderable { handle, mesh_key });
		}
	}

	fn remove_mesh_instance(&mut self, handle: Handle) {
		self.pending_renderables.retain(|pending| pending.handle != handle);
		self.scene.remove_renderable(handle);
		self.availability.remove(&Availability::Renderable(handle)).expect(
			"Visibility renderable availability could not be removed. The most likely cause is that another graph node depends on a renderable.",
		);
	}

	/* Resource adoption */

	/// Finishes renderer-specific adoption of loaded resources and publishes only fully usable ones.
	fn adopt_resource_completions(&mut self, frame: &mut ghi::implementation::Frame) {
		let mut events = std::mem::take(&mut self.resource_events);
		self.loader.update(frame, &mut events);
		// Material readiness changes rebuild the material lists once, after every event of the frame.
		let mut materials_changed = false;
		for event in events.drain(..) {
			match event {
				VisibilityLoaderEvent::MeshReady { key, mesh } => self.resolve_pending_renderables(key, &mesh),
				VisibilityLoaderEvent::MaterialReady(material, pipeline) => {
					self.adopt_material(material, pipeline);
					materials_changed = true;
				}
				VisibilityLoaderEvent::MaterialUnavailable { index } => {
					self.availability.set_key_available(&Availability::Material(index), false);
					materials_changed = true;
				}
				VisibilityLoaderEvent::TextureReady(texture) => materials_changed |= self.adopt_texture(frame, texture),
				VisibilityLoaderEvent::EnvironmentReady { id, environment } => {
					if self.environment.requested.as_deref() == Some(id.as_str()) {
						self.environment.bind(environment);
					}
				}
				VisibilityLoaderEvent::Unavailable { resource, error } => {
					warn!("Visibility {resource} is unavailable: {error}");
				}
			}
		}
		self.resource_events = events;
		if materials_changed {
			self.rebuild_material_lists();
		}
		if self.environment.descriptors_dirty {
			self.environment.descriptors_dirty = false;
			for sink_state in &self.scene.sink_states {
				frame.write(
					&self
						.environment
						.bound
						.descriptor_writes(sink_state.render_pass.material_evaluation_descriptor_set()),
				);
			}
		}
	}

	/// Publishes one texture the loader already transferred, and returns whether a loaded material samples it.
	fn adopt_texture(&mut self, frame: &mut ghi::implementation::Frame, texture: ResidentTexture) -> bool {
		let ResidentTexture {
			id,
			index,
			image,
			sampler,
			photometry,
		} = texture;
		frame.write(&[ghi::DescriptorWrite::combined_image_sampler_array(
			self.scene.descriptor_set,
			TEXTURES_BINDING.slot(),
			image,
			sampler,
			ghi::Layouts::Read,
			index,
		)]);
		match photometry {
			Some(photometry) if photometry.intensity_scale_candela.is_finite() && photometry.intensity_scale_candela > 0.0 => {
				self.loaded_ies_profiles.insert(
					id,
					IesProfileTexture {
						texture_index: index,
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
					crate::online_docs_url("reference/lighting#use-an-ies-profile")
				);
			}
			_ => {}
		}
		let texture = self.availability.get_or_insert(Availability::Texture(index), false);
		self.availability.set_available(texture, true);
		self.loaded_materials
			.iter()
			.flatten()
			.any(|material| material.texture_indices.contains(&index))
	}

	/// Adopts material metadata and its compiled `pipeline` into the canonical table and wires its texture dependencies.
	///
	/// Next, call [`Self::rebuild_material_lists`] once the frame's events are adopted.
	fn adopt_material(&mut self, material: ResidentMaterial, pipeline: ghi::PipelineHandle) {
		let ResidentMaterial {
			id,
			index,
			alpha_mode,
			double_sided,
			coverage,
			texture_slots: textures,
			..
		} = material;
		let material_data = &mut self.materials[index as usize];
		if material_data.set_textures(textures.iter().copied()) {
			warn!(
				"Visibility material {id} has too many texture slots. The most likely cause is that the material shader expects more textures than the visibility material data supports."
			);
		}
		material_data.coverage_factor = coverage.factor;
		material_data.coverage_texture_slot = coverage.texture_slot.unwrap_or(u32::MAX);
		material_data.alpha_cutoff = match alpha_mode {
			AlphaMode::Mask(cutoff) => cutoff,
			AlphaMode::Opaque | AlphaMode::Blend => 0.0,
		};

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
		self.availability.set_available(material, true);
		let slot = index as usize;
		if slot >= self.loaded_materials.len() {
			self.loaded_materials.resize_with(slot + 1, || None);
		}
		self.loaded_materials[slot] = Some(LoadedMaterial {
			index,
			pipeline,
			name: id.into(),
			alpha_mode,
			double_sided,
			texture_indices,
		});
		self.materials_copies_current = [false; ghi::MAX_FRAMES_IN_FLIGHT];
	}

	/// Rebuilds the opaque and transparent material evaluation lists and gives every ready material its evaluation slot.
	///
	/// Materials whose programs differ only in the textures they bind request equal pipelines, which the pipeline
	/// manager compiles once, so grouping ready materials by pipeline finds them without comparing programs. Each
	/// group gets one slot in the list of its phase, and each phase records one dispatch per slot it draws instead of
	/// one per material. A
	/// material that isn't ready keeps [`NO_EVALUATION`]; no admitted instance uses it.
	fn rebuild_material_lists(&mut self) {
		let render_info = &mut self.scene.render_info;
		render_info.opaque_evaluations.clear();
		render_info.transparent_evaluations.clear();
		self.material_evaluations.fill(NO_EVALUATION);
		// The availability graph combines pipeline and texture readiness before a material reaches an evaluation.
		let mut ready = self
			.loaded_materials
			.iter()
			.flatten()
			.filter(|material| self.availability.is_key_ready(&Availability::Material(material.index)))
			.collect::<Vec<_>>();
		ready.sort_unstable_by_key(|material| (material.pipeline, material.index));
		for (evaluation_index, materials) in ready.chunk_by(|left, right| left.pipeline == right.pipeline).enumerate() {
			let evaluation_index = evaluation_index as u32;
			for material in materials {
				self.material_evaluations[material.index as usize] = evaluation_index;
			}
			// The loader specializes every pipeline for its material's phase, so a slot belongs to one phase.
			let first = &materials[0];
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
				pipeline: first.pipeline,
			});
		}
		self.materials_copies_current = [false; ghi::MAX_FRAMES_IN_FLIGHT];
	}

	/// Creates scene instances for every pending renderable whose mesh is now resident.
	fn resolve_pending_renderables(&mut self, key: MeshKey, mesh: &MeshData) {
		let mut pending = std::mem::take(&mut self.pending_renderables);
		pending.retain(|renderable| {
			if renderable.mesh_key != key {
				return true;
			}
			self.add_renderable(renderable.handle, mesh);
			false
		});
		self.pending_renderables = pending;
	}

	fn add_renderable(&mut self, handle: Handle, mesh: &MeshData) {
		let model = self
			.renderable_transforms
			.get(&handle)
			.map_or_else(Transform::default, |samples| samples.current.clone())
			.get_matrix()
			.into();
		let availability = self.availability.get_or_insert(Availability::Renderable(handle), true);
		for primitive in &mesh.primitives {
			let material = self
				.availability
				.get_or_insert(Availability::Material(primitive.material_index), false);
			self.availability
				.add_dependency(availability, material)
				.expect("Visibility renderable dependency could not be registered. The most likely cause is a cyclic or stale resource relationship.");
			self.scene.add_render_entity(RenderEntity {
				handle,
				availability,
				shader_mesh: ShaderMesh {
					model,
					material_index: primitive.material_index,
					base_vertex_index: mesh.vertex_offset + primitive.vertex_offset,
					base_primitive_index: mesh.primitive_offset + primitive.primitive_offset,
					base_triangle_index: mesh.triangle_offset + primitive.triangle_offset,
					base_meshlet_index: mesh.meshlet_offset + primitive.meshlet_offset,
					meshlet_count: primitive.meshlet_count,
					skinned_base_vertex_index: u32::MAX,
					flags: 0,
					bounding_sphere: primitive.bounding_sphere,
				},
				skinning: primitive.skin.as_ref().map(|binding| RenderSkin {
					binding: binding.clone(),
					source_vertex_offset: primitive
						.skinning_source_vertex_offset
						.expect("Skinned primitive has no GPU source range. The most likely cause is that skin streams were not uploaded with the mesh resource."),
					vertex_count: primitive.skinning_vertex_count,
					skeleton_node_count: mesh.skeleton_node_count,
				}),
			});
		}
	}

	/* Frame preparation */

	/// Applies queued GTAO, SSGI, and contact-shadow controls before any sink records this frame's commands.
	fn apply_runtime_settings(&mut self) {
		drain_settings(
			&self.gtao_configuration,
			GTAO_CONFIGURATION_PREFIX,
			"GTAO parameter was not set. The most likely cause is that the parameter is outside the `render.gtao.` namespace.",
			&mut self.gtao_settings,
			GtaoSettings::with_parameter,
		);
		drain_settings(
			&self.ssgi_configuration,
			SSGI_CONFIGURATION_PREFIX,
			"SSGI parameter was not set. The most likely cause is that the parameter is outside the `render.ssgi.` namespace.",
			&mut self.ssgi_settings,
			SsgiSettings::with_parameter,
		);
		drain_settings(
			&self.contact_shadow_configuration,
			CONTACT_SHADOWS_CONFIGURATION_PREFIX,
			"Contact shadow parameter was not set. The most likely cause is that the parameter is outside the `render.contact-shadows.` namespace.",
			&mut self.contact_shadow_settings,
			ContactShadowSettings::with_parameter,
		);
	}

	/// Rebuilds the frame's instance lists from whole renderables whose dependencies are ready, and uploads skin palettes.
	fn rebuild_active_instances(&mut self, frame: &mut ghi::implementation::Frame) {
		let render_info = &mut self.scene.render_info;
		render_info.clear_active_instances();
		self.skinning_frame.clear();
		let mesh_data = frame.get_mut_dynamic_buffer_slice(self.scene.meshes_buffer);
		let mut deformed_vertex_count = 0;
		// Every admitted entity pushes exactly one instance, so this counts the instances so far.
		let mut active_index = 0;

		for entity in self.scene.render_entities.iter() {
			// A renderable enters a frame as one object; never expose the subset whose materials loaded first.
			if !self.availability.is_ready(entity.availability) {
				continue;
			}
			let Some(material) = self
				.loaded_materials
				.get(entity.shader_mesh.material_index as usize)
				.and_then(Option::as_ref)
			else {
				continue;
			};
			assert!(
				active_index < MAX_INSTANCES,
				"Visibility active instance limit exceeded. The most likely cause is that the scene contains more visible mesh primitives than the visibility pipeline supports."
			);

			let mut shader_mesh = entity.shader_mesh;
			shader_mesh.skinned_base_vertex_index = u32::MAX;
			// Sidedness comes from the currently loaded material, so a reloaded material takes effect next frame.
			shader_mesh.flags = if material.double_sided { MESH_FLAG_DOUBLE_SIDED } else { 0 };
			if let Some(skin) = &entity.skinning
				&& let Some(pose) = self.scene.skinning_poses.get(&entity.handle)
			{
				assert_eq!(
					pose.len(),
					skin.skeleton_node_count as usize,
					"Visibility skin pose has the wrong matrix count. The most likely cause is that the pose was written for a different skeleton."
				);
				if skin.vertex_count > 0
					&& let Some((palette_base, kind)) = self.skinning_frame.palette(entity.handle, &skin.binding, pose)
				{
					// Output is dense per active primitive, so shared meshes never overwrite another instance's pose.
					shader_mesh.skinned_base_vertex_index = deformed_vertex_count as u32;
					deformed_vertex_count += skin.vertex_count as usize;
					assert!(
						deformed_vertex_count <= MAX_SKINNED_VERTICES,
						"Visibility deformed vertex limit exceeded. The most likely cause is that active animated instances require more frame-local vertex storage than the visibility pipeline supports."
					);
					render_info.skinning_dispatches.push(SkinningDispatch {
						source_vertex_base: skin.source_vertex_offset,
						destination_vertex_base: shader_mesh.skinned_base_vertex_index,
						palette_base,
						palette_count: skin.binding.entries.len() as u32,
						vertex_count: skin.vertex_count,
						palette_kind: kind as u32,
					});
				}
			}
			mesh_data[active_index] = shader_mesh;
			render_info.push_active_instance(
				Instance {
					shader_mesh_index: active_index as u32,
					meshlet_count: shader_mesh.meshlet_count,
				},
				self.material_evaluations[shader_mesh.material_index as usize],
				&material.alpha_mode,
				material.double_sided,
			);
			active_index += 1;
		}
		frame.sync_buffer(self.scene.meshes_buffer);
		self.skinning_pass
			.write_palettes(frame, &self.skinning_frame.matrices, &self.skinning_frame.dual_quaternions);
	}

	/// Writes the camera view and every shadow view selected this frame. Returns each shadowed sun's cascades, fitted
	/// to the camera frustum, by sun slot.
	///
	/// `ies_scales` holds each light's IES intensity scale by light index.
	fn write_views(
		&self,
		frame: &mut ghi::implementation::Frame,
		main_view: View,
		shadows: &ShadowLightSelection<'_>,
		ies_scales: &[(f32, Option<IesProfileTexture>)],
	) -> SunCascades {
		let views = frame.get_mut_dynamic_buffer_slice(self.scene.views_buffer);
		views.fill(ShaderViewData::from(main_view));
		let cascades = shadows
			.suns
			.iter()
			.enumerate()
			.map(|(slot, sun)| {
				let cascades = csm::make_cascade_frames(
					main_view,
					sun.direction,
					SHADOW_CASCADE_COUNT,
					self.settings.directional_shadow_map_resolution,
					self.settings.cascade_splits,
				)
				.collect::<SmallVec<[_; SHADOW_CASCADE_COUNT]>>()
				.into_inner()
				.expect("Cascade count does not match the shadow views. The most likely cause is that make_cascade_frames was called with another cascade count.");
				for (cascade, frame) in cascades.iter().enumerate() {
					let mut data = ShaderViewData::from(frame.view);
					data.far = frame.slice_far;
					views[sun_cascade_view(slot) + cascade] = data;
				}
				cascades
			})
			.collect();
		for (layer, (index, light, transform)) in shadows.cones.iter().enumerate() {
			views[CONE_SHADOW_VIEW_OFFSET + layer] =
				make_cone_shadow_view(light, transform, SHADOW_DEFAULT_EXPOSURE_SCALE, ies_scales[*index].0).into();
		}
		for (cube, (index, light, transform)) in shadows.points.iter().enumerate() {
			let scale = ies_scales[*index].0;
			for face in 0..POINT_SHADOW_FACE_COUNT {
				views[POINT_SHADOW_VIEW_OFFSET + cube * POINT_SHADOW_FACE_COUNT + face] =
					make_point_shadow_view(light, transform, face, SHADOW_DEFAULT_EXPOSURE_SCALE, scale).into();
			}
		}
		frame.sync_buffer(self.scene.views_buffer);
		cascades
	}
}

impl PipelineManager for VisibilityPipelineManager {
	/// Drains creation messages in dependency order: resources first so their loads start before any entity
	/// needs them, then lights, meshes, and environments.
	fn update(&mut self) {
		while let Some(message) = self.resource_listener.read() {
			self.loader.request_resource(message.into_data());
		}
		// Concrete routes keep application-defined creation strongly typed. The scene erases each light only at its
		// storage boundary.
		while let Some(message) = self.cone_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into());
		}
		while let Some(message) = self.directional_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into());
		}
		while let Some(message) = self.point_light_listener.read() {
			self.create_light(message.handle(), message.into_data().into());
		}
		while let Some(message) = self.mesh_listener.read() {
			self.request_mesh(message.handle(), message.into_data());
		}
		while let Some(message) = self.environment_listener.read() {
			self.create_environment(message.into_data());
		}
	}

	/// Marks the end of one simulation step and keeps the transforms it published as the samples frames move toward.
	fn step(&mut self) {
		self.step_count += 1;
		while let Some(message) = self.transforms_listener.read() {
			self.record_transform(message.handle(), message.transform(), Some(self.step_count));
		}
	}

	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
		alpha: f32,
		_time: crate::time::MediaTime,
	) -> SmallVec<[(usize, RenderPassReturn<'a>); 16]> {
		self.adopt_scene_updates(alpha);
		self.apply_runtime_settings();
		self.adopt_resource_completions(frame);
		// Each frame sequence has its own buffers, so changed tables are copied into each one on its own frame.
		let sequence = frame.key().sequence_index() as usize;
		if !self.materials_copies_current[sequence] {
			self.materials_copies_current[sequence] = true;
			frame
				.get_mut_dynamic_buffer_slice(self.materials_buffer)
				.copy_from_slice(&*self.materials);
			frame.sync_buffer(self.materials_buffer);
			frame
				.get_mut_dynamic_buffer_slice(self.material_evaluations_buffer)
				.copy_from_slice(&*self.material_evaluations);
			frame.sync_buffer(self.material_evaluations_buffer);
		}
		self.rebuild_active_instances(frame);
		let dispatches = self.mesh_dispatch_work.write_phases(frame, &self.scene.render_info);

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
		let shadows = select_shadow_lights(
			self.scene.lights.iter().map(|(_, light, transform)| (light, transform)),
			sinks,
			&budget,
			|index| ies_scales[index].0,
		);
		crate::rendering::warn_once(&mut self.reported_limits[1], shadows.unshadowed_count > 0, || {
			format!(
				"Shadow map budget exceeded. The most likely cause is that more visible lights need shadows than the {} MiB set by `{SHADOW_MAP_BUDGET_PARAMETER}` holds, or that the directional cascades' resolution leaves too little of it. {} lights remain lit without shadows.",
				self.settings.shadow_map_budget_mib, shadows.unshadowed_count,
			)
		});
		self.shadow_layout = retain_layout(self.shadow_layout, shadows.layout(), &budget);
		let cascades = sinks
			.first()
			.map(|sink| self.write_views(frame, sink.view(), &shadows, &ies_scales))
			.unwrap_or_default();
		// Like the views above, exposure comes from the first sink; every sink shares one lighting upload.
		let exposure = sinks.first().map_or(1.0, Sink::exposure_scale);
		self.scene
			.write_lighting(frame, &shadows, exposure, self.environment.intensity(), &ies_scales);
		let shadow_work = ShadowWork {
			suns: shadows.suns.clone(),
			cascade_resolution: self.settings.directional_shadow_map_resolution,
			receiver_fit: (self.settings.cascade_fitting == CascadeFitting::Receivers)
				.then_some(cascades)
				.unwrap_or_default(),
			layout: self.shadow_layout,
			cone_count: shadows.cones.len(),
			point_count: shadows.points.len(),
		};

		let frame_work = (&self.skinning_pass, &self.shadow_maps);
		let render_info = &self.scene.render_info;
		let previously_recorded_sinks = &self.recorded_sinks;
		let recorded_exposure = self.recorded_exposure;
		let recorded_ssgi = self.recorded_ssgi;
		let mut recorded_sinks = SmallVec::<[Sink; 4]>::new();
		let commands = sinks
			.iter()
			.filter_map(|sink| {
				let state = self.scene.sink_states.iter().find(|state| state.id == sink.index())?;
				Some((sink, &state.render_pass, state.background.as_ref()))
			})
			.enumerate()
			.filter_map(|(command_index, (sink, render_pass, background))| {
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
						ssgi: recorded_ssgi,
					});
				let command = render_pass.prepare(
					frame,
					sink,
					frame_work,
					dispatches,
					render_info,
					&shadow_work,
					history,
					exposure,
					self.gtao_settings,
					self.ssgi_settings,
					self.contact_shadow_settings,
					background,
					frame_allocator,
				)?;
				recorded_sinks.push(*sink);
				Some((sink.index(), allocate_render_command(frame_allocator, command)))
			})
			.collect();
		self.recorded_sinks = recorded_sinks;
		self.recorded_exposure = exposure;
		self.recorded_ssgi = self.ssgi_settings.enabled;
		commands
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		let lit = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				crate::rendering::SCENE_COLOR_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Lit"),
		);
		let depth = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::Depth32, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Depth")
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);
		let primitive_index = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("primitive index"),
		);
		let instance_id = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::U32, ghi::Uses::RenderTarget | ghi::Uses::Storage).name("instance_id"),
		);
		render_pass_builder.alias("Depth", "depth");
		render_pass_builder.alias("Lit", "main");
		let background = render_pass_builder.create_scene_background(crate::rendering::render_pass::SceneBackgroundTargets {
			color: lit.into(),
			depth: depth.into(),
		});
		let ssgi = create_ssgi_targets(render_pass_builder);
		let sun_visibility = create_sun_visibility_targets(render_pass_builder);
		let radiance_history = create_radiance_history_target(render_pass_builder);
		let stage_counters = super::render_pass::StageCounters::new(render_pass_builder);

		let context = render_pass_builder.context();
		let render_pass = VisibilityRenderPass::new(
			context,
			self.pipeline_manager.clone(),
			self.scene.descriptor_set,
			self.scene.lighting_buffer,
			SinkTargets {
				lit: lit.into(),
				depth: depth.into(),
				primitive_index: primitive_index.into(),
				instance_id: instance_id.into(),
				ssgi,
				sun_visibility,
				radiance_history,
			},
			&self.shadow_maps,
			stage_counters,
		);
		context.write(
			&self
				.environment
				.bound
				.descriptor_writes(render_pass.material_evaluation_descriptor_set()),
		);
		self.scene.sink_states.push(SinkState {
			id: sink_id,
			render_pass,
			background,
		});
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	use super::*;
	use crate::rendering::lights::{LightColor, PhotometricIntensity, PointLight};

	fn at(x: f32) -> Transform {
		Transform::from_position(math::Point::new(x, 0.0, 0.0))
	}

	fn shown_x(samples: &mut TransformSamples, step: u64, alpha: f32) -> Option<f32> {
		samples.shown(step, alpha).map(|transform| transform.get_position().x())
	}

	#[test]
	fn samples_move_from_the_previous_step_to_the_latest_then_rest() {
		let mut samples = TransformSamples::snapped(&at(0.0));
		samples.sample(&at(2.0), 1);

		assert_eq!(shown_x(&mut samples, 1, 0.0), Some(0.0));
		assert_eq!(shown_x(&mut samples, 1, 0.5), Some(1.0));
		// No sample in step 2: the renderable rests at its latest state and reports once.
		assert_eq!(shown_x(&mut samples, 2, 0.3), Some(2.0));
		assert_eq!(shown_x(&mut samples, 2, 0.9), None);
	}

	#[test]
	fn a_second_sample_in_one_step_replaces_the_first() {
		let mut samples = TransformSamples::snapped(&at(0.0));
		samples.sample(&at(1.0), 1);
		samples.sample(&at(4.0), 1);

		assert_eq!(shown_x(&mut samples, 1, 0.0), Some(0.0));
		assert_eq!(shown_x(&mut samples, 1, 1.0), Some(4.0));
	}

	#[test]
	fn a_renderable_that_paused_starts_its_next_segment_where_it_rested() {
		let mut samples = TransformSamples::snapped(&at(0.0));
		samples.sample(&at(2.0), 1);
		assert_eq!(shown_x(&mut samples, 2, 0.5), Some(2.0));
		samples.sample(&at(6.0), 3);

		assert_eq!(shown_x(&mut samples, 3, 0.0), Some(2.0));
		assert_eq!(shown_x(&mut samples, 3, 0.5), Some(4.0));
	}

	#[test]
	fn a_snap_shows_at_once_and_ends_the_motion_in_progress() {
		let mut samples = TransformSamples::snapped(&at(0.0));
		samples.sample(&at(2.0), 1);
		samples.snap(&at(10.0));

		assert_eq!(shown_x(&mut samples, 1, 0.5), Some(10.0));
	}

	#[test]
	fn resolved_ies_profile_texture_applies_the_per_light_dimmer() {
		let profile_light = Lights::Point(
			PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.5, "lights/office.ies")
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
			texture_index: 19,
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
					texture_index: 19,
					intensity_scale_candela: 90.0,
				})
			)
		);
		assert_eq!(resolve_ies_profile(&analytic_light, &profiles), (1.0, None));
	}
}
