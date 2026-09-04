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
use resource_management::resources::skeleton::{AffineMatrix4x3Columns, SkinBinding, identity_affine_matrix4x3_columns};
use resource_management::types::AlphaMode;
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::{AvailabilityGraph, Extent, StableVec};

use super::geometry::{GeometryHandles, MeshData};
use super::layout::{
	CONE_SHADOW_VIEW_OFFSET, DEFAULT_CONE_SHADOW_POOL_CAPACITY, DEFAULT_POINT_SHADOW_POOL_CAPACITY, ENVIRONMENT_BINDING,
	MATERIALS_DATA_BINDING, MAX_BINDLESS_TEXTURES, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_INSTANCES, MAX_MATERIAL_TEXTURES,
	MAX_MATERIALS, MAX_POINT_SHADOW_POOL_CAPACITY, MESH_DATA_BINDING, MESHLET_DATA_BINDING, POINT_SHADOW_FACE_COUNT,
	POINT_SHADOW_VIEW_OFFSET, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, SKINNED_VERTICES_BINDING,
	SPECULAR_ENVIRONMENT_BINDING, TEXTURES_BINDING, VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING,
	VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use super::loader::{ResidentEnvironment, ResidentMaterial, ResidentTexture, VisibilityLoaderClient, VisibilityLoaderEvent};
use super::mesh_dispatch::MeshDispatchWorkBuffer;
use super::render_pass::{GTAO_CONFIGURATION_PREFIX, GtaoSettings, ShadowWork, SinkTargets, VisibilityRenderPass};
use super::scene::{Instance, RenderEntity, RenderSkin, SinkState, VisibilityScene, ies_profile};
use super::shader_data::{IesProfileTexture, MaterialData, ShaderMesh, ShaderViewData};
use super::shadow_selection::{
	SHADOW_DEFAULT_EXPOSURE_SCALE, ShadowLightSelection, make_cone_shadow_view, make_point_shadow_view, select_shadow_lights,
};
use super::skinning::{
	DualQuaternion, MAX_SKINNED_VERTICES, MAX_SKINNING_MATRICES, SkinningDispatch, SkinningPaletteKind, SkinningPass,
	append_dual_quaternion_palette,
};
use crate::core::factory::Handle;
use crate::core::listener::{DefaultListener, Listener as _};
use crate::gameplay::Transform;
use crate::gameplay::transform::TransformationUpdate;
use crate::rendering::lights::{IesProfile, Lights};
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::render_pass::{RenderPassBuilder, RenderPassReturn, allocate_render_command};
use crate::rendering::renderable::mesh::MeshKey;
use crate::rendering::{Environment, PipelineManagerClient, RenderableMesh, Sink, View, csm};

/// The startup parameters that set the local-light shadow pool capacities.
pub const CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER: &str = "render.cone-shadow-map-pool.capacity";
pub const POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER: &str = "render.point-shadow-map-pool.capacity";

/// The `VisibilityPipelineSettings` struct configures memory limits for the visibility rendering pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VisibilityPipelineSettings {
	cone_shadow_map_pool_capacity: usize,
	point_shadow_map_pool_capacity: usize,
}

impl Default for VisibilityPipelineSettings {
	fn default() -> Self {
		Self {
			cone_shadow_map_pool_capacity: DEFAULT_CONE_SHADOW_POOL_CAPACITY,
			point_shadow_map_pool_capacity: DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		}
	}
}

impl VisibilityPipelineSettings {
	/// Sets the maximum number of reusable cone-light shadow maps per visibility sink.
	pub fn with_cone_shadow_map_pool_capacity(mut self, capacity: usize) -> Result<Self, String> {
		if capacity > MAX_CONE_SHADOW_POOL_CAPACITY {
			return Err(format!(
				"Cone shadow map pool capacity was not set. The most likely cause is that {capacity} exceeds the visibility pipeline limit of {MAX_CONE_SHADOW_POOL_CAPACITY}."
			));
		}
		self.cone_shadow_map_pool_capacity = capacity;
		Ok(self)
	}

	pub fn cone_shadow_map_pool_capacity(&self) -> usize {
		self.cone_shadow_map_pool_capacity
	}

	/// Sets the maximum number of reusable point-light cube shadow maps per visibility sink.
	pub fn with_point_shadow_map_pool_capacity(mut self, capacity: usize) -> Result<Self, String> {
		if capacity > MAX_POINT_SHADOW_POOL_CAPACITY {
			return Err(format!(
				"Point shadow map pool capacity was not set. The most likely cause is that {capacity} exceeds the visibility pipeline limit of {MAX_POINT_SHADOW_POOL_CAPACITY}."
			));
		}
		self.point_shadow_map_pool_capacity = capacity;
		Ok(self)
	}

	pub fn point_shadow_map_pool_capacity(&self) -> usize {
		self.point_shadow_map_pool_capacity
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

/// One material's render-thread pipeline and authored alpha contract.
struct LoadedMaterial {
	index: u32,
	pipeline: ghi::PipelineHandle,
	name: String,
	alpha_mode: AlphaMode,
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
fn create_fallback_environment(context: &mut ghi::implementation::Context) -> ResidentEnvironment {
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
	let sampler = context.build_sampler(
		ghi::sampler::Builder::new()
			.filtering_mode(ghi::FilteringModes::Linear)
			.reduction_mode(ghi::SamplingReductionModes::WeightedAverage)
			.mip_map_mode(ghi::FilteringModes::Linear)
			.addressing_mode(ghi::SamplerAddressingModes::Repeat)
			.min_lod(0.0)
			.max_lod(0.0),
	);
	ResidentEnvironment {
		diffuse_image: image.into(),
		specular_image: image.into(),
		sampler,
	}
}

/// Environment selection: the requested resource, every environment that finished uploading, and what is bound.
struct EnvironmentState {
	requested: Option<String>,
	bound: ResidentEnvironment,
	/// The bound environment changed and existing sinks must be rewritten.
	descriptors_dirty: bool,
}

impl EnvironmentState {
	fn bind(&mut self, environment: ResidentEnvironment) {
		self.bound = environment;
		self.descriptors_dirty = true;
	}
}

/// One uploaded palette shared by every primitive of a renderable that uses the same skin binding.
#[derive(Clone, Copy)]
struct PaletteCacheEntry {
	handle: Handle,
	binding: *const SkinBinding,
	palette_base: u32,
	kind: SkinningPaletteKind,
}

/// The `SkinningFrame` struct accumulates this frame's palettes without allocating after the scene's high-water mark.
#[derive(Default)]
struct SkinningFrame {
	matrices: Vec<AffineMatrix4x3Columns>,
	dual_quaternions: Vec<DualQuaternion>,
	cache: Vec<PaletteCacheEntry>,
}

impl SkinningFrame {
	/// Frame caches retain capacity but never retain entity or resource pointers beyond one rebuild.
	fn clear(&mut self) {
		self.matrices.clear();
		self.dual_quaternions.clear();
		self.cache.clear();
	}

	fn cached(&self, handle: Handle, binding: *const SkinBinding) -> Option<(u32, SkinningPaletteKind)> {
		self.cache
			.iter()
			.find(|entry| entry.handle == handle && entry.binding == binding)
			.map(|entry| (entry.palette_base, entry.kind))
	}

	/// Returns the palette range for one renderable's binding, uploading it once per frame.
	///
	/// Rigid poses are converted to dual quaternions; everything else keeps the matrix palette.
	fn palette(
		&mut self,
		handle: Handle,
		binding: &Arc<SkinBinding>,
		pose: &[AffineMatrix4x3Columns],
	) -> Option<(u32, SkinningPaletteKind)> {
		let binding_ptr = Arc::as_ptr(binding);
		if let Some(palette) = self.cached(handle, binding_ptr) {
			return Some(palette);
		}
		let matrix_base = self.matrices.len();
		let matrix_end = matrix_base + binding.len();
		self.matrices.resize(matrix_end, identity_affine_matrix4x3_columns());
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
		self.cache.push(PaletteCacheEntry {
			handle,
			binding: binding_ptr,
			palette_base: palette_base as u32,
			kind,
		});
		Some((palette_base as u32, kind))
	}
}

/// Reserves a non-overlapping frame-local vertex range for one active skinned primitive.
fn reserve_deformed_vertex_range(cursor: &mut usize, vertex_count: u32) -> u32 {
	let base = *cursor;
	*cursor += vertex_count as usize;
	assert!(
		*cursor <= MAX_SKINNED_VERTICES,
		"Visibility deformed vertex limit exceeded. The most likely cause is that active animated instances require more frame-local vertex storage than the visibility pipeline supports."
	);
	base as u32
}

/// Returns the dimmer while a profile texture is pending, then its dimmed calibrated scale after residency.
fn profile_intensity_scale(profile: Option<&IesProfile>, profiles: &HashMap<String, IesProfileTexture>) -> f32 {
	let Some(profile) = profile else {
		return 1.0;
	};
	profiles
		.get(profile.resource_id())
		.map_or(profile.dimmer(), |texture| texture.intensity_scale_candela * profile.dimmer())
}

/// Returns one light's analytic, fallback-profile, or calibrated-profile intensity scale.
fn ies_intensity_scale(light: &Lights, profiles: &HashMap<String, IesProfileTexture>) -> f32 {
	profile_intensity_scale(ies_profile(light), profiles)
}

/// Resolves a profile light to its resident texture and dimmed calibrated candela scale.
fn resolved_ies_profile_texture(light: &Lights, profiles: &HashMap<String, IesProfileTexture>) -> Option<IesProfileTexture> {
	let profile = ies_profile(light)?;
	let mut texture = profiles.get(profile.resource_id()).copied()?;
	texture.intensity_scale_candela *= profile.dimmer();
	Some(texture)
}

/// The `VisibilityPipelineManager` struct provides the visibility-buffer implementation of the world render domain.
///
/// Register it through [`crate::rendering::Renderer::add_pipeline_manager`]. Scene changes arrive through the
/// creation, deletion, transform, and pose methods; frames flow through the [`PipelineManager`] callbacks.
pub struct VisibilityPipelineManager {
	/// Domain façade for requesting resources and consuming readiness changes.
	loader: VisibilityLoaderClient,
	pipeline_manager: PipelineManagerClient,
	/// Transform updates consumed after resource completions and before instance rebuilds.
	transforms_listener: DefaultListener<TransformationUpdate>,
	/// Canonical material table, copied into the frame-local buffer every frame.
	materials: Box<[MaterialData; MAX_MATERIALS]>,
	materials_buffer: ghi::DynamicBufferHandle<[MaterialData; MAX_MATERIALS]>,
	mesh_dispatch_work: MeshDispatchWorkBuffer,
	skinning_pass: SkinningPass,
	skinning_frame: SkinningFrame,
	pending_renderables: Vec<PendingRenderable>,
	/// Latest transform per renderable, retained so a mesh that loads later starts in the right place.
	renderable_transforms: HashMap<Handle, Transform>,
	loaded_materials: HashMap<u32, LoadedMaterial>,
	/// Calibrated IES profile textures that completed their GPU upload, keyed by resource ID.
	loaded_ies_profiles: HashMap<String, IesProfileTexture>,
	availability: AvailabilityGraph<Availability>,
	environment: EnvironmentState,
	cone_shadow_pool_capacity: usize,
	point_shadow_pool_capacity: usize,
	gtao_configuration: crate::configuration::ConfigurationPort,
	gtao_settings: GtaoSettings,
	pub(crate) scene: VisibilityScene,
}

impl VisibilityPipelineManager {
	/// Creates the scene buffers and base descriptor set around an already running resource client.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		geometry: GeometryHandles,
		loader: VisibilityLoaderClient,
		pipeline_manager: PipelineManagerClient,
		transforms_listener: DefaultListener<TransformationUpdate>,
		gtao_configuration: crate::configuration::ConfigurationPort,
		settings: VisibilityPipelineSettings,
	) -> Self {
		let environment = create_fallback_environment(context);
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
		let views_buffer = context.build_dynamic_buffer(host_buffer("Visibility Views Data", ghi::Uses::Storage));
		let meshes_buffer = context.build_dynamic_buffer(host_buffer("Visibility Meshes Data", ghi::Uses::Storage));
		let lighting_buffer =
			context.build_dynamic_buffer(host_buffer("Light Data", ghi::Uses::Storage | ghi::Uses::TransferDestination));
		let descriptor_set = context.create_descriptor_set(Some("Base Descriptor Set"));
		let mesh_dispatch_work = MeshDispatchWorkBuffer::new(context, descriptor_set);
		let write = |binding: ghi::ShaderResourceDescriptor, buffer| {
			ghi::DescriptorWrite::buffer(descriptor_set, binding.slot(), buffer)
		};
		context.write(&[
			write(VIEWS_DATA_BINDING, views_buffer.into()),
			write(MESH_DATA_BINDING, meshes_buffer.into()),
			write(VERTEX_POSITIONS_BINDING, geometry.vertex_positions.into()),
			write(VERTEX_NORMALS_BINDING, geometry.vertex_normals.into()),
			write(SKINNED_VERTICES_BINDING, skinning_pass.skinned_vertices_buffer().into()),
			write(VERTEX_UV_BINDING, geometry.vertex_uvs.into()),
			write(VERTEX_INDICES_BINDING, geometry.vertex_indices.into()),
			write(PRIMITIVE_INDICES_BINDING, geometry.primitive_indices.into()),
			write(MESHLET_DATA_BINDING, geometry.meshlets.into()),
			write(MATERIALS_DATA_BINDING, materials_buffer.into()),
		]);
		Self {
			loader,
			pipeline_manager,
			transforms_listener,
			materials: Box::new([MaterialData::default(); MAX_MATERIALS]),
			materials_buffer,
			mesh_dispatch_work,
			skinning_pass,
			skinning_frame: SkinningFrame::default(),
			pending_renderables: Vec::new(),
			renderable_transforms: HashMap::new(),
			loaded_materials: HashMap::new(),
			loaded_ies_profiles: HashMap::new(),
			availability: AvailabilityGraph::with_capacity(
				MAX_INSTANCES + MAX_MATERIALS + MAX_BINDLESS_TEXTURES,
				MAX_INSTANCES + MAX_MATERIALS * MAX_MATERIAL_TEXTURES,
			),
			environment: EnvironmentState {
				requested: None,
				bound: environment,
				descriptors_dirty: false,
			},
			cone_shadow_pool_capacity: settings.cone_shadow_map_pool_capacity,
			point_shadow_pool_capacity: settings.point_shadow_map_pool_capacity,
			gtao_configuration,
			gtao_settings: GtaoSettings::default(),
			scene: VisibilityScene {
				render_entities: StableVec::new(),
				skinning_poses: HashMap::new(),
				render_entity_handles: HashMap::new(),
				lights: StableVec::new(),
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

	/// Retains a renderable's latest world transform and applies it to every registered primitive and light.
	pub(crate) fn update_transform(&mut self, handle: Handle, transform: &Transform) {
		self.renderable_transforms.insert(handle, transform.clone());
		self.scene.update_transform(handle, transform);
	}

	/// Applies queued transforms before resource adoption so late-loading meshes start in place.
	pub(crate) fn process_transform_updates(&mut self) {
		while let Some(message) = self.transforms_listener.read() {
			self.update_transform(message.handle(), message.transform());
		}
	}

	/// Retains a renderable's global skeleton pose for palette generation during frame preparation.
	pub fn update_pose(&mut self, handle: Handle, global_matrices: &[math::Matrix]) {
		self.scene.write_skinned_pose(handle, global_matrices);
	}

	/// Adds one light and requests its optional photometric texture dependency.
	pub(crate) fn create_light(&mut self, handle: Handle, light: Lights) {
		if let Some(profile) = ies_profile(&light) {
			self.loader.request_texture(profile.resource_id().to_owned());
		}
		self.scene.lights.push((handle, light, Transform::default()));
	}

	pub(crate) fn remove_light(&mut self, handle: Handle) {
		self.scene.remove_light(handle);
	}

	/// Selects an environment and requests its baked lighting resources.
	pub(crate) fn create_environment(&mut self, environment: Environment) {
		let id = environment.resource_id().to_owned();
		self.environment.requested = Some(id.clone());
		if let Some(resident) = self.loader.request_environment(id) {
			self.environment.bind(resident);
		}
	}

	/// Requests the renderable's mesh and keeps the scene instance pending until the mesh is resident.
	pub(crate) fn request_mesh(&mut self, handle: Handle, renderable: RenderableMesh) {
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

	/// Removes a renderable and any transform retained for asynchronous creation.
	pub(crate) fn remove_mesh(&mut self, handle: Handle) {
		self.remove_mesh_instance(handle);
		self.renderable_transforms.remove(&handle);
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
		while let Some(event) = self.loader.poll() {
			match event {
				VisibilityLoaderEvent::MeshReady { key, mesh } => self.resolve_pending_renderables(key, &mesh),
				VisibilityLoaderEvent::MaterialReady(material) => self.adopt_material(material),
				VisibilityLoaderEvent::MaterialUnavailable { index } => {
					self.availability.set_key_available(&Availability::Material(index), false);
					self.rebuild_material_lists();
				}
				VisibilityLoaderEvent::TextureReady(texture) => self.adopt_texture(frame, texture),
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

	/// Publishes one texture the loader already transferred.
	fn adopt_texture(&mut self, frame: &mut ghi::implementation::Frame, texture: ResidentTexture) {
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
			_ if self
				.scene
				.lights
				.iter()
				.any(|(_, light, _)| ies_profile(light).is_some_and(|profile| profile.resource_id() == id.as_str())) =>
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
		if self
			.loaded_materials
			.values()
			.any(|material| material.texture_indices.contains(&index))
		{
			self.rebuild_material_lists();
		}
	}

	/// Adopts material metadata into the canonical table and wires its texture dependencies.
	fn adopt_material(&mut self, material: ResidentMaterial) {
		let ResidentMaterial {
			id,
			index,
			pipeline,
			alpha_mode,
			coverage,
			texture_slots: textures,
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
		self.loaded_materials.insert(
			index,
			LoadedMaterial {
				index,
				pipeline,
				name: id,
				alpha_mode,
				texture_indices,
			},
		);
		self.rebuild_material_lists();
	}

	/// Rebuilds the opaque and transparent material lists consumed by material evaluation.
	fn rebuild_material_lists(&mut self) {
		let render_info = &mut self.scene.render_info;
		render_info.opaque_materials.clear();
		render_info.transparent_materials.clear();
		for material in self.loaded_materials.values() {
			// The availability graph combines pipeline and texture readiness before a material reaches a draw list.
			if !self.availability.is_key_ready(&Availability::Material(material.index)) {
				continue;
			}
			let entry = (material.name.clone(), material.index, material.pipeline);
			match material.alpha_mode {
				AlphaMode::Blend => render_info.transparent_materials.push(entry),
				AlphaMode::Opaque | AlphaMode::Mask(_) => render_info.opaque_materials.push(entry),
			}
		}
		// Materials with the same generated shader share one pipeline. Keep them adjacent so recording can retain
		// the native pipeline binding across consecutive indirect dispatches.
		for materials in [&mut render_info.opaque_materials, &mut render_info.transparent_materials] {
			materials.sort_unstable_by(|left, right| left.2.cmp(&right.2).then(left.1.cmp(&right.1)));
		}
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
			.cloned()
			.unwrap_or_default()
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
					_padding: 0,
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

	/// Applies queued GTAO controls before any sink records this frame's commands.
	fn apply_gtao_configuration(&mut self) {
		while let Some(update) = self.gtao_configuration.read() {
			let Some(parameter) = update.parameter().strip_prefix(GTAO_CONFIGURATION_PREFIX) else {
				self.gtao_configuration.not_set(
					update.id(),
					"GTAO parameter was not set. The most likely cause is that the parameter is outside the `render.gtao.` namespace.",
				);
				continue;
			};
			match self.gtao_settings.with_parameter(parameter, update.value()) {
				Ok((settings, effective_value)) => {
					self.gtao_settings = settings;
					for sink_state in &mut self.scene.sink_states {
						sink_state.render_pass.set_gtao_settings(settings);
					}
					self.gtao_configuration.set(update.id(), effective_value);
				}
				Err(reason) => self.gtao_configuration.not_set(update.id(), reason),
			}
		}
	}

	/// Rebuilds the frame's instance lists from whole renderables whose dependencies are ready, and uploads skin palettes.
	fn rebuild_active_instances(&mut self, frame: &mut ghi::implementation::Frame) {
		let render_info = &mut self.scene.render_info;
		render_info.clear_active_instances();
		self.skinning_frame.clear();
		let mesh_data = frame.get_mut_dynamic_buffer_slice(self.scene.meshes_buffer);
		let mut deformed_vertex_count = 0;

		for entity in self.scene.render_entities.iter() {
			// A renderable enters a frame as one object; never expose the subset whose materials loaded first.
			if !self.availability.is_ready(entity.availability) {
				continue;
			}
			let Some(material) = self.loaded_materials.get(&entity.shader_mesh.material_index) else {
				continue;
			};
			let active_index = render_info.active_instance_count();
			assert!(
				active_index < MAX_INSTANCES,
				"Visibility active instance limit exceeded. The most likely cause is that the scene contains more visible mesh primitives than the visibility pipeline supports."
			);

			let mut shader_mesh = entity.shader_mesh;
			shader_mesh.skinned_base_vertex_index = u32::MAX;
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
					shader_mesh.skinned_base_vertex_index =
						reserve_deformed_vertex_range(&mut deformed_vertex_count, skin.vertex_count);
					render_info.skinning_dispatches.push(SkinningDispatch {
						source_vertex_base: skin.source_vertex_offset,
						destination_vertex_base: shader_mesh.skinned_base_vertex_index,
						palette_base,
						palette_count: skin.binding.len() as u32,
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
				shader_mesh.material_index,
				&material.alpha_mode,
			);
		}
		frame.sync_buffer(self.scene.meshes_buffer);
		self.skinning_pass
			.write_palettes(frame, &self.skinning_frame.matrices, &self.skinning_frame.dual_quaternions);
	}

	/// Writes the camera view and every shadow view selected this frame.
	fn write_views(&self, frame: &mut ghi::implementation::Frame, main_view: View, shadows: &ShadowLightSelection<'_>) {
		let profiles = &self.loaded_ies_profiles;
		let views = frame.get_mut_dynamic_buffer_slice(self.scene.views_buffer);
		views.fill(ShaderViewData::from(main_view));
		if let Some((_, light_direction)) = shadows.directional {
			let cascade_views = csm::make_csm_views(main_view, light_direction, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION);
			let cascade_far = csm::make_cascade_split_ranges(main_view, SHADOW_CASCADE_COUNT).map(|(_, far)| far);
			for (cascade, (view, far)) in cascade_views.zip(cascade_far).enumerate() {
				let mut data = ShaderViewData::from(view);
				data.far = far;
				views[1 + cascade] = data;
			}
		}
		for (layer, (_, light, transform)) in shadows
			.cones
			.iter()
			.enumerate()
			.filter_map(|(layer, cone)| cone.map(|cone| (layer, cone)))
		{
			let scale = profile_intensity_scale(light.ies_profile(), profiles);
			views[CONE_SHADOW_VIEW_OFFSET + layer] =
				make_cone_shadow_view(light, transform, SHADOW_DEFAULT_EXPOSURE_SCALE, scale).into();
		}
		for (cube, (_, light, transform)) in shadows
			.points
			.iter()
			.enumerate()
			.filter_map(|(cube, point)| point.map(|point| (cube, point)))
		{
			let scale = profile_intensity_scale(light.ies_profile(), profiles);
			for face in 0..POINT_SHADOW_FACE_COUNT {
				views[POINT_SHADOW_VIEW_OFFSET + cube * POINT_SHADOW_FACE_COUNT + face] =
					make_point_shadow_view(light, transform, face, SHADOW_DEFAULT_EXPOSURE_SCALE, scale).into();
			}
		}
		frame.sync_buffer(self.scene.views_buffer);
	}
}

impl PipelineManager for VisibilityPipelineManager {
	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>> {
		self.apply_gtao_configuration();
		self.adopt_resource_completions(frame);
		frame
			.get_mut_dynamic_buffer_slice(self.materials_buffer)
			.copy_from_slice(&*self.materials);
		frame.sync_buffer(self.materials_buffer);
		self.rebuild_active_instances(frame);
		let dispatches = self.mesh_dispatch_work.write_phases(frame, &self.scene.render_info);

		let profiles = &self.loaded_ies_profiles;
		let shadows = select_shadow_lights(
			self.scene.lights.iter().map(|(_, light, transform)| (light, transform)),
			sinks,
			self.cone_shadow_pool_capacity,
			self.point_shadow_pool_capacity,
			|light| ies_intensity_scale(light, profiles),
		);
		for (kind, eligible, capacity) in [
			("Cone", shadows.eligible_cone_count, self.cone_shadow_pool_capacity),
			("Point", shadows.eligible_point_count, self.point_shadow_pool_capacity),
		] {
			if eligible > capacity {
				warn!(
					"{kind}-light shadow pool capacity exceeded. The most likely cause is that more than {capacity} visible {} lights require shadows. Extra lights remain lit without shadows.",
					kind.to_lowercase()
				);
			}
		}
		if let Some(sink) = sinks.first() {
			self.write_views(frame, sink.view(), &shadows);
		}
		self.scene
			.write_lighting(frame, &shadows, |light| resolved_ies_profile_texture(light, profiles));
		let shadow_work = ShadowWork {
			directional: shadows.directional.is_some(),
			cone_count: shadows.cone_count(),
			point_count: shadows.point_count(),
		};

		let skinning_pass = &self.skinning_pass;
		let render_info = &self.scene.render_info;
		let commands = sinks
			.iter()
			.filter_map(|sink| {
				let state = self.scene.sink_states.iter().find(|state| state.id == sink.index())?;
				Some((sink, &state.render_pass))
			})
			.enumerate()
			.filter_map(|(command_index, (sink, render_pass))| {
				// Skinning runs once per frame, with the first sink.
				let skinning = (command_index == 0).then_some(skinning_pass);
				render_pass
					.prepare(frame, sink, skinning, dispatches, render_info, shadow_work)
					.map(|command| allocate_render_command(frame_allocator, command))
			})
			.collect();
		Some(commands)
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
			},
			self.cone_shadow_pool_capacity,
			self.point_shadow_pool_capacity,
			self.gtao_settings,
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
		});
	}
}

#[cfg(test)]
mod tests {
	use maths_rs::Vec3f;

	use super::*;
	use crate::core::factory::Factory;
	use crate::rendering::lights::{LightColor, PhotometricIntensity, PointLight};

	#[test]
	fn visibility_pipeline_settings_bound_local_shadow_pool_capacities() {
		let defaults = VisibilityPipelineSettings::default();
		let settings = defaults
			.with_cone_shadow_map_pool_capacity(1)
			.expect("supported cone shadow map pool capacity")
			.with_point_shadow_map_pool_capacity(2)
			.expect("supported point shadow map pool capacity");

		assert_eq!(defaults.cone_shadow_map_pool_capacity(), DEFAULT_CONE_SHADOW_POOL_CAPACITY);
		assert_eq!(settings.cone_shadow_map_pool_capacity(), 1);
		assert_eq!(defaults.point_shadow_map_pool_capacity(), DEFAULT_POINT_SHADOW_POOL_CAPACITY);
		assert_eq!(settings.point_shadow_map_pool_capacity(), 2);
		assert!(
			defaults
				.with_cone_shadow_map_pool_capacity(MAX_CONE_SHADOW_POOL_CAPACITY + 1)
				.is_err()
		);
		assert!(
			defaults
				.with_point_shadow_map_pool_capacity(MAX_POINT_SHADOW_POOL_CAPACITY + 1)
				.is_err()
		);
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

		assert_eq!(ies_intensity_scale(&profile_light, &profiles), 0.5);
		assert_eq!(ies_intensity_scale(&analytic_light, &profiles), 1.0);
		profiles.insert("lights/office.ies".to_string(), profile);

		assert_eq!(ies_intensity_scale(&profile_light, &profiles), 90.0);
		assert_eq!(
			resolved_ies_profile_texture(&profile_light, &profiles),
			Some(IesProfileTexture {
				texture_index: 19,
				intensity_scale_candela: 90.0,
			})
		);
		assert_eq!(resolved_ies_profile_texture(&analytic_light, &profiles), None);
	}

	/// Ensures instances that share immutable source vertices cannot overwrite each other's deformation output.
	#[test]
	fn active_skin_instances_receive_non_overlapping_vertex_ranges() {
		let mut cursor = 0;
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 3), 0);
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 3), 3);
		assert_eq!(reserve_deformed_vertex_range(&mut cursor, 5), 6);
		assert_eq!(cursor, 11);
	}

	/// Ensures interleaved handles keep their palettes instance-local.
	#[test]
	fn noncontiguous_primitives_reuse_their_frame_skinning_palette() {
		let factory = Factory::new();
		let first_handle = factory.create(());
		let second_handle = factory.create(());
		let first_binding = Arc::new(SkinBinding { entries: Vec::new() });
		let second_binding = Arc::new(SkinBinding { entries: Vec::new() });
		let mut frame = SkinningFrame::default();
		for (handle, binding, palette_base, kind) in [
			(first_handle, &first_binding, 7, SkinningPaletteKind::DualQuaternion),
			(first_handle, &second_binding, 11, SkinningPaletteKind::Matrix),
			(second_handle, &first_binding, 17, SkinningPaletteKind::Matrix),
		] {
			frame.cache.push(PaletteCacheEntry {
				handle,
				binding: Arc::as_ptr(binding),
				palette_base,
				kind,
			});
		}

		assert_eq!(
			frame.cached(first_handle, Arc::as_ptr(&first_binding)),
			Some((7, SkinningPaletteKind::DualQuaternion))
		);
		assert_eq!(
			frame.cached(first_handle, Arc::as_ptr(&second_binding)),
			Some((11, SkinningPaletteKind::Matrix))
		);
		assert_eq!(
			frame.cached(second_handle, Arc::as_ptr(&first_binding)),
			Some((17, SkinningPaletteKind::Matrix))
		);
		assert_eq!(frame.cached(second_handle, Arc::as_ptr(&second_binding)), None);
	}
}
