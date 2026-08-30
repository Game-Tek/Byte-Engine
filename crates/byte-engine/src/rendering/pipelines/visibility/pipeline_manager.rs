//! Visibility scene ownership and the final adoption half of asynchronous resource loading.
//!
//! The resource-manager client stops at renderer-oriented completion values.
//! [`VisibilityPipelineManager`] consumes them during frame preparation,
//! interns detached factory objects, starts or polls native GPU I/O, updates
//! material and texture tables, resolves dependency-complete renderables, and
//! only then builds draws. This final layer exists because loading completion is
//! not always the same as scene readiness.
//!
//! To trace the complete flow, start with
//! `VisibilityResourcePreparer::spawn`, follow the client's `begin_frame` and
//! `record_frame_uploads` callbacks, then read
//! `VisibilityPipelineManager::adopt_resource_completions`. Application wiring
//! is in [`crate::application::graphics::setup_pbr_visibility_shading_render_pipeline`].

mod data;
mod environment;
mod manager;
mod shadow;

pub use data::*;
use environment::*;
use shadow::*;

/// The `SkinningPaletteCacheEntry` struct shares one uploaded binding palette across a renderable's primitives.
#[derive(Clone, Copy)]

struct SkinningPaletteCacheEntry {
	handle: Handle,
	binding: *const SkinBinding,
	palette_base: u32,
	palette_kind: SkinningPaletteKind,
}

/// The `EnvironmentTexture` struct retains the image and sampler currently used for visibility reflections.
#[derive(Clone, Copy)]

struct EnvironmentTexture {
	diffuse_image: ghi::BaseImageHandle,
	specular_image: ghi::BaseImageHandle,
	sampler: ghi::SamplerHandle,
}

/// The `PendingTextureIo` struct retains one texture until native resource I/O completes.
#[cfg(target_os = "macos")]
struct PendingTextureIo {
	token: crate::rendering::resource_loading::ResourceToken,
	key: VisibilityTextureKey,
	index: u32,
	image: ghi::BaseImageHandle,
	sampler: ghi::SamplerHandle,
	photometry: Option<resource_management::resources::image::ImagePhotometry>,
	ticket: ghi::implementation::ResourceIoTicket,
}

/// The `VisibilityPipelineSettings` struct configures memory limits for the visibility rendering pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]

pub struct VisibilityPipelineSettings {
	cone_shadow_map_pool_capacity: usize,
	point_shadow_map_pool_capacity: usize,
}

/// The startup parameter that sets [`VisibilityPipelineSettings::cone_shadow_map_pool_capacity`].
pub const CONE_SHADOW_MAP_POOL_CAPACITY_PARAMETER: &str = "render.cone-shadow-map-pool.capacity";

/// The startup parameter that sets [`VisibilityPipelineSettings::point_shadow_map_pool_capacity`].
pub const POINT_SHADOW_MAP_POOL_CAPACITY_PARAMETER: &str = "render.point-shadow-map-pool.capacity";

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

	/// Returns the maximum number of cone-light shadow maps reused by each visibility sink.
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

	/// Returns the maximum number of point-light cube shadow maps reused by each visibility sink.
	pub fn point_shadow_map_pool_capacity(&self) -> usize {
		self.point_shadow_map_pool_capacity
	}
}

/// The `VisibilityPipelineManager` struct provides the visibility buffer implementation for the world render domain.
///
/// It owns scene-visible residency and the renderer-specific second-stage
/// adoption that cannot live in the shared loader: object interning, native
/// texture I/O, shader-table updates, dependency closure, and sink-local draw
/// preparation. Its resource-manager client owns request and transfer state.
///
pub struct VisibilityPipelineManager {
	/// Canonical CPU material metadata retained across every frame sequence.
	materials_data: std::boxed::Box<[MaterialData; MAX_MATERIALS]>,
	/// Frame-local material metadata buffer shared across all scenes.
	materials_data_buffer_handle: ghi::DynamicBufferHandle<[MaterialData; MAX_MATERIALS]>,
	/// Compact single-view mesh work shared by shadow cascades and visibility phases.
	mesh_dispatch_work: crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatchWorkBuffer,
	/// Compute resources shared by every sink for frame-local mesh deformation.
	skinning_pass: SkinningPass,
	/// Fixed visibility pipelines requested during sink creation and resolved during frame preparation.
	pipeline_manager: crate::rendering::PipelineManagerClient,
	/// Application-owned baked resources used by the fixed visibility shader set.
	shader_resources: EntityHandle<ResourceManager>,
	/// Transform updates consumed after asynchronous resource completions and before instance rebuilds.
	transforms_listener: DefaultListener<crate::gameplay::transform::TransformationUpdate>,
	/// Reused palette upload storage prevents per-frame matrix allocations.
	skinning_palette_scratch: Vec<AffineMatrix4x3Columns>,
	/// Reused rigid palette upload storage prevents per-frame dual-quaternion allocations.
	skinning_dual_quaternion_palette_scratch: Vec<DualQuaternion>,
	/// Reused per-instance palette lookup avoids duplicate uploads when primitive order is noncontiguous.
	skinning_palette_cache: Vec<SkinningPaletteCacheEntry>,
	resource_manager: VisibilityPipelineResourceManagerClient,
	#[cfg(target_os = "macos")]
	resource_io_queue: ghi::implementation::ResourceIoQueue,
	#[cfg(target_os = "macos")]
	pending_texture_io: Vec<PendingTextureIo>,
	pending_renderables: Vec<PendingRenderableInstance>,
	// TODO: Replace this temporary map with proper retained component storage.
	renderable_transforms: HashMap<Handle, Transform>,
	loaded_meshes: HashMap<VisibilityMeshKey, MeshData>,
	loaded_materials: HashMap<u32, RenderDescription>,
	loaded_textures: HashSet<u32>,
	/// Calibrated IES profile textures that completed their GPU upload, keyed by resource ID.
	loaded_ies_profiles: HashMap<String, IesProfileTexture>,
	/// Eager readiness state for renderables, materials, and their textures.
	availability: AvailabilityGraph<VisibilityAvailability>,
	/// Requested environment resource retained until its asynchronous upload completes.
	environment_resource_id: Option<String>,
	/// Completed environment residents remain selectable without repeating GPU work.
	loaded_environments: HashMap<String, EnvironmentTexture>,
	/// A cached environment selection needs publication to existing sink descriptors.
	environment_descriptors_dirty: bool,
	/// Texture bound to material evaluation; starts as a transparent analytical-fallback marker.
	environment_texture: EnvironmentTexture,
	/// Maximum number of local-light maps reused by each sink during a frame.
	cone_shadow_map_pool_capacity: usize,
	/// Maximum number of point-light cube maps reused by each sink during a frame.
	point_shadow_map_pool_capacity: usize,
	gtao_configuration: crate::configuration::ConfigurationPort,
	gtao_settings: crate::rendering::pipelines::visibility::render_pass::GtaoSettings,
	pub(crate) scene: crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager,
}

#[cfg(target_os = "macos")]
impl Drop for VisibilityPipelineManager {
	fn drop(&mut self) {
		// Renderer drops pipeline managers before its GHI context, so every native I/O destination remains alive here.
		for pending in self.pending_texture_io.drain(..) {
			if let Err(error) = pending.ticket.wait() {
				log::error!("Visibility texture I/O shutdown failed: {error}");
			}
		}
	}
}

impl PipelineManager for VisibilityPipelineManager {
	fn begin_frame(&mut self, completed_frame: Option<ghi::FrameKey>) -> bool {
		self.resource_manager.begin_frame(completed_frame)
	}

	fn record_frame_uploads(&mut self, frame: ghi::FrameKey, recording: &mut ghi::implementation::CommandBufferRecording<'_>) {
		self.resource_manager.record_frame_uploads(frame, recording);
	}

	fn prepare<'a>(
		&'a mut self,
		frame: &mut ghi::implementation::Frame,
		sinks: &[Sink],
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<SmallVec<[RenderPassReturn<'a>; 16]>> {
		self.apply_gtao_configuration();

		self.adopt_resource_completions(frame);

		self.refresh_material_pipelines();

		self.write_material_data(frame);

		self.rebuild_active_instances(frame);

		let loaded_ies_profiles = &self.loaded_ies_profiles;

		let shadow_lights = select_shadow_lights_with_intensity_scale(
			self.scene.lights.iter().map(|(_, light, transform)| (light, transform)),
			sinks,
			self.cone_shadow_map_pool_capacity,
			self.point_shadow_map_pool_capacity,
			|light| manager::ies_intensity_scale(light, loaded_ies_profiles),
		);

		if shadow_lights.eligible_cone_count > self.cone_shadow_map_pool_capacity {
			warn!(
				"Cone-light shadow pool capacity exceeded. The most likely cause is that more than {} visible cone lights require shadows. Extra cone lights remain lit without shadows.",
				self.cone_shadow_map_pool_capacity,
			);
		}

		if shadow_lights.eligible_point_count > self.point_shadow_map_pool_capacity {
			warn!(
				"Point-light shadow pool capacity exceeded. The most likely cause is that more than {} visible point lights require shadows. Extra point lights remain lit without shadows.",
				self.point_shadow_map_pool_capacity,
			);
		}

		let [opaque_mesh_dispatch, masked_mesh_dispatch, transparent_mesh_dispatch] = self.mesh_dispatch_work.write_phases(
			frame,
			[
				self.scene.render_info.opaque_instances.as_slice(),
				self.scene.render_info.masked_instances.as_slice(),
				self.scene.render_info.transparent_instances.as_slice(),
			],
		);

		if let Some(sink) = sinks.first() {
			let main_view = sink.view();

			let main_view_data = Self::make_shader_view_data(main_view);

			let views_data_buffer = frame.get_mut_dynamic_buffer_slice(self.scene.views_data_buffer_handle);

			for view_data in views_data_buffer.iter_mut() {
				*view_data = main_view_data;
			}

			if let Some((_, light_direction)) = shadow_lights.directional {
				for (cascade_index, (cascade_view, cascade_far)) in
					csm::make_csm_views(main_view, light_direction, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION)
						.zip(csm::make_cascade_split_ranges(main_view, SHADOW_CASCADE_COUNT).map(|(_, far)| far))
						.enumerate()
				{
					let mut cascade_view_data = Self::make_shader_view_data(cascade_view);

					cascade_view_data.far = cascade_far;

					views_data_buffer[cascade_index + 1] = cascade_view_data;
				}
			}

			for (layer, shadow_light) in shadow_lights.cones.iter().enumerate() {
				if let Some((_, light, transform)) = shadow_light {
					let intensity_scale_candela =
						manager::ies_intensity_scale_for_profile(light.ies_profile(), loaded_ies_profiles);

					views_data_buffer[CONE_SHADOW_VIEW_OFFSET + layer] = Self::make_shader_view_data(make_cone_shadow_view(
						*light,
						transform,
						CONE_SHADOW_DEFAULT_EXPOSURE_SCALE,
						intensity_scale_candela,
					));
				}
			}

			for (cube_index, shadow_light) in shadow_lights.points.iter().enumerate() {
				if let Some((_, light, transform)) = shadow_light {
					let intensity_scale_candela =
						manager::ies_intensity_scale_for_profile(light.ies_profile(), loaded_ies_profiles);

					for face in 0..POINT_SHADOW_FACE_COUNT {
						views_data_buffer[POINT_SHADOW_VIEW_OFFSET + cube_index * POINT_SHADOW_FACE_COUNT + face] =
							Self::make_shader_view_data(make_point_shadow_view(
								*light,
								transform,
								face,
								POINT_SHADOW_DEFAULT_EXPOSURE_SCALE,
								intensity_scale_candela,
							));
					}
				}
			}

			frame.sync_buffer(self.scene.views_data_buffer_handle);
		}

		let directional_shadow_light_index = shadow_lights.directional.map(|(index, _)| index);

		let cone_shadow_light_indices = shadow_lights.cones.map(|light| light.map(|(index, ..)| index));

		let point_shadow_light_indices = shadow_lights.points.map(|light| light.map(|(index, ..)| index));

		self.scene.write_light_data(
			frame,
			directional_shadow_light_index,
			&cone_shadow_light_indices,
			&point_shadow_light_indices,
			|light| manager::resolved_ies_profile_texture(light, loaded_ies_profiles),
		);

		let sink_x_rp = sinks.iter().filter_map(|sink| {
			self.scene
				.sink_states
				.iter()
				.find(|sink_state| sink_state.id == sink.index())
				.map(|sink_state| (sink, &sink_state.render_pass))
		});

		let skinning_pass = &self.skinning_pass;

		let skinning_dispatches = self.scene.render_info.skinning_dispatches.as_slice();

		let commands: SmallVec<[RenderPassReturn<'a>; 16]> = sink_x_rp
			.enumerate()
			.filter_map(|(command_index, (v, r))| {
				r.prepare(
					frame,
					v,
					(command_index == 0).then_some(skinning_pass),
					opaque_mesh_dispatch,
					masked_mesh_dispatch,
					transparent_mesh_dispatch,
					skinning_dispatches,
					&self.scene.render_info.opaque_instances,
					&self.scene.render_info.masked_instances,
					&self.scene.render_info.transparent_instances,
					&self.scene.render_info.opaque_materials,
					&self.scene.render_info.transparent_materials,
					&self.scene.render_info.opaque_material_mask,
					&self.scene.render_info.transparent_material_mask,
					directional_shadow_light_index.is_some(),
					cone_shadow_light_indices.iter().flatten().count(),
					point_shadow_light_indices.iter().flatten().count(),
				)
				.map(|command| crate::rendering::render_pass::allocate_render_command(frame_allocator, command))
			})
			.collect::<SmallVec<[_; 16]>>();

		log::debug!(
			"Visibility prepare summary: sinks={}, sink_states={}, commands={}, loaded_meshes={}, pending_renderables={}, render_entities={}, active_primitives={}, opaque_primitives={}, transparent_primitives={}, opaque_materials={}, transparent_materials={}, directional_shadow_enabled={}, cone_shadow_count={}, cone_shadow_pool_capacity={}, point_shadow_count={}, point_shadow_pool_capacity={}",
			sinks.len(),
			self.scene.sink_states.len(),
			commands.len(),
			self.loaded_meshes.len(),
			self.pending_renderables.len(),
			self.scene.render_entities.len(),
			self.scene.render_info.active_instance_count(),
			self.scene.render_info.opaque_instances.len(),
			self.scene.render_info.transparent_instances.len(),
			self.scene.render_info.opaque_materials.len(),
			self.scene.render_info.transparent_materials.len(),
			directional_shadow_light_index.is_some(),
			cone_shadow_light_indices.iter().flatten().count(),
			self.cone_shadow_map_pool_capacity,
			point_shadow_light_indices.iter().flatten().count(),
			self.point_shadow_map_pool_capacity,
		);

		Some(commands)
	}

	fn create_sink(&mut self, sink_id: usize, render_pass_builder: &mut RenderPassBuilder) {
		log::debug!("Visibility sink created: sink_id={}", sink_id);

		let lit_target = render_pass_builder.create_render_target(
			ghi::image::Builder::new(
				crate::rendering::SCENE_COLOR_FORMAT,
				ghi::Uses::RenderTarget | ghi::Uses::Image | ghi::Uses::Storage | ghi::Uses::TransferDestination,
			)
			.name("Lit"),
		);

		let depth_target = render_pass_builder.create_render_target(
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

		let context = render_pass_builder.context();

		let visibility_passes_descriptor_set = context.create_descriptor_set(Some("Visibility Descriptor Set"));

		let material_evaluation_descriptor_set = context.create_descriptor_set(Some("Material Evaluation Descriptor Set"));

		let material_count_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Count")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_xy: ghi::BufferHandle<[(u16, u16); MAX_PIXEL_MAPPING_ENTRIES]> = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material XY")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_evaluation_dispatches = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination | ghi::Uses::Indirect)
				.name("Material Evaluation Dipatches")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let material_offset_scratch_buffer = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage | ghi::Uses::TransferDestination)
				.name("Material Offset Scratch")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(
				ghi::Formats::R8UNORM,
				ghi::Uses::RenderTarget | ghi::Uses::Storage | ghi::Uses::Image | ghi::Uses::TransferDestination,
			)
			.name("Occlusion Map")
			.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);

		let directional_shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(DIRECTIONAL_SHADOW_MAP_FORMAT, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Directional Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32))
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
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

		// Dynamic images start at zero extent, so this pool has no backing shadow maps until a visible cone uses it.
		// Metal requires two layers to create the array texture that material evaluation always binds.
		let cone_shadow_map_layers = self.cone_shadow_map_pool_capacity.max(2) as u32;

		let cone_shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(CONE_SHADOW_MAP_FORMAT, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Cone Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.array_layers(NonZeroU32::new(cone_shadow_map_layers))
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);

		// Dynamic images start at zero extent, so this cube array has no backing maps until a visible point light uses it.
		let point_shadow_map = context.build_dynamic_image(
			ghi::image::Builder::new(POINT_SHADOW_MAP_FORMAT, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name("Point Shadow Map")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.cube_array_compatible(
					NonZeroU32::new(self.point_shadow_map_pool_capacity.max(1) as u32)
						.expect("Point shadow map pool has a nonzero fallback cube."),
				)
				.optimized_clear_value(ghi::ClearValue::Depth(0.0)),
		);

		let sampler = context.build_sampler(
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

		let directional_shadow_depth_pyramid_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod((DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);

		context.write(&[
			ghi::DescriptorWrite::image(
				material_evaluation_descriptor_set,
				LIT_BINDING.slot(),
				ghi::BaseImageHandle::from(lit_target),
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(
				material_evaluation_descriptor_set,
				LIGHTING_DATA_BINDING.slot(),
				self.scene.light_data_buffer.into(),
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				AO_MAP_BINDING.slot(),
				ao_map,
				sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				SHADOW_MAP_BINDING.slot(),
				directional_shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING.slot(),
				directional_shadow_depth_pyramid,
				directional_shadow_depth_pyramid_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				CONE_SHADOW_MAP_BINDING.slot(),
				cone_shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				material_evaluation_descriptor_set,
				POINT_SHADOW_MAP_BINDING.slot(),
				point_shadow_map,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_COUNT_BINDING.slot(),
				material_count_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_OFFSET_BINDING.slot(),
				material_offset_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_OFFSET_SCRATCH_BINDING.slot(),
				material_offset_scratch_buffer.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_EVALUATION_DISPATCHES_BINDING.slot(),
				material_evaluation_dispatches.into(),
			),
			ghi::DescriptorWrite::buffer(
				visibility_passes_descriptor_set,
				MATERIAL_XY_BINDING.slot(),
				material_xy.into(),
			),
			ghi::DescriptorWrite::image(
				visibility_passes_descriptor_set,
				TRIANGLE_INDEX_BINDING.slot(),
				ghi::BaseImageHandle::from(primitive_index),
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::image(
				visibility_passes_descriptor_set,
				INSTANCE_ID_BINDING.slot(),
				ghi::BaseImageHandle::from(instance_id),
				ghi::Layouts::General,
			),
		]);

		context.write(&[diffuse_environment_descriptor_write(
			material_evaluation_descriptor_set,
			self.environment_texture,
		)]);

		context.write(&[specular_environment_descriptor_write(
			material_evaluation_descriptor_set,
			self.environment_texture,
		)]);

		render_pass_builder.alias("Depth", "depth");

		render_pass_builder.alias("Lit", "main");

		let render_pass = VisibilityPipelineRenderPass::new(
			render_pass_builder.context(),
			self.pipeline_manager.clone(),
			self.scene.descriptor_set,
			visibility_passes_descriptor_set,
			material_evaluation_descriptor_set,
			material_count_buffer,
			ghi::BaseImageHandle::from(lit_target),
			ao_map.into(),
			directional_shadow_map.into(),
			directional_shadow_depth_pyramid.into(),
			cone_shadow_map.into(),
			point_shadow_map.into(),
			ghi::BaseImageHandle::from(depth_target),
			ghi::BaseImageHandle::from(primitive_index),
			ghi::BaseImageHandle::from(instance_id),
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			self.gtao_settings,
		);

		self.scene.sink_states.push(SinkState {
			id: sink_id,
			render_pass,
		});
	}
}

#[cfg(test)]

mod tests {

	use std::sync::Arc;

	use math::{Orientation, Point, UnitVector};
	use maths_rs::{Vec3f, Vec4f};
	use resource_management::resources::skeleton::SkinBinding;
	use resource_management::types::AlphaMode;
	use utils::AvailabilityGraph;
	use utils::{Extent, hash::HashMap};

	use super::manager::{
		cached_skin_palette, ies_intensity_scale, reserve_deformed_vertex_range, resolved_ies_profile_texture,
		retained_renderable_transform,
	};
	use super::{
		AO_MAP_BINDING, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE, CONE_SHADOW_EXPOSURE_THRESHOLD_LUX, CONE_SHADOW_MAP_BINDING,
		CONE_SHADOW_NEAR_M, DEFAULT_CONE_SHADOW_POOL_CAPACITY, DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING, ENVIRONMENT_BINDING, IesProfileTexture, Instance, LIGHTING_DATA_BINDING,
		LIT_BINDING, LightData, LightingData, MATERIALS_DATA_BINDING, MAX_CONE_SHADOW_POOL_CAPACITY,
		MAX_POINT_SHADOW_POOL_CAPACITY, MaterialData, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, POINT_SHADOW_EXPOSURE_THRESHOLD_LUX,
		POINT_SHADOW_NEAR_M, RenderInfo, SHADOW_MAP_BINDING, SPECULAR_ENVIRONMENT_BINDING, ShaderMesh, ShaderViewData,
		SkinningPaletteCacheEntry, VisibilityAvailability, VisibilityPipelineSettings, cone_light_has_brightness,
		cone_shadow_importance, make_cone_shadow_view, make_point_shadow_view, point_light_has_brightness, point_shadow_bounds,
		point_shadow_importance, resolve_cone_shadow_range, resolve_point_shadow_range, select_shadow_lights,
		select_shadow_lights_with_intensity_scale, write_material_texture_indices,
	};
	use crate::core::factory::Factory;
	use crate::gameplay::Transform;
	use crate::rendering::lights::{ConeLight, DirectionalLight, LightColor, Lights, PhotometricIntensity, PointLight};
	use crate::rendering::pipelines::visibility::resource_manager::IBL_SPECULAR_LEVEL_COUNT;
	use crate::rendering::pipelines::visibility::skinning::SkinningPaletteKind;
	use crate::rendering::pipelines::visibility::{
		INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING,
		MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_MATERIAL_TEXTURES, MAX_MATERIALS, MESH_DATA_BINDING,
		MESH_DATA_BUFFER_STRIDE, MESH_DISPATCH_WORK_BINDING, MESHLET_DATA_BINDING, POINT_SHADOW_FACE_COUNT,
		PRIMITIVE_INDICES_BINDING, SKINNED_VERTICES_BINDING, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING,
		VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEW_DATA_BUFFER_STRIDE, VIEWS_DATA_BINDING,
	};
	use crate::rendering::{Sink, View};

	#[test]
	fn early_renderable_transform_is_available_when_mesh_becomes_resident() {
		let mut handles = Factory::new();
		let handle = handles.create(());
		let transform = Transform::from_position(Point::new(4.0, 5.0, 6.0));
		let mut retained_transforms = HashMap::default();

		retained_transforms.insert(handle, transform.clone());

		assert_eq!(
			retained_renderable_transform(&retained_transforms, handle).get_matrix(),
			transform.get_matrix()
		);
	}

	#[test]
	fn renderable_transform_upsert_replaces_the_value_used_at_residency() {
		let mut handles = Factory::new();
		let handle = handles.create(());
		let first = Transform::from_position(Point::new(1.0, 2.0, 3.0));
		let replacement = Transform::from_position(Point::new(7.0, 8.0, 9.0));
		let mut retained_transforms = HashMap::default();

		retained_transforms.insert(handle, first);
		retained_transforms.insert(handle, replacement.clone());

		assert_eq!(
			retained_renderable_transform(&retained_transforms, handle).get_matrix(),
			replacement.get_matrix()
		);
	}

	#[test]
	fn renderable_admission_waits_for_every_primitive_dependency() {
		let mut handles = Factory::new();
		let incomplete = handles.create("incomplete");
		let independent = handles.create("independent");
		let independent = handles.create("independent");
		let mut availability = AvailabilityGraph::new();
		let ready_material = availability.get_or_insert(VisibilityAvailability::Material(0), true);
		let pending_material = availability.get_or_insert(VisibilityAvailability::Material(1), false);
		let incomplete = availability.get_or_insert(VisibilityAvailability::Renderable(incomplete), true);
		let independent = availability.get_or_insert(VisibilityAvailability::Renderable(independent), true);
		availability.add_dependency(incomplete, ready_material).unwrap();
		availability.add_dependency(incomplete, pending_material).unwrap();
		availability.add_dependency(independent, ready_material).unwrap();

		assert!(!availability.is_ready(incomplete));
		assert!(availability.is_ready(independent));

		availability.set_available(pending_material, true);

		assert!(availability.is_ready(incomplete));
		assert!(availability.is_ready(independent));
	}

	/// Creates one compact shadow-capable cone for selection and projection tests.
	fn cone(_position_x: f32) -> ConeLight {
		ConeLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
			math::Degrees::new(15.0).to_radians(),
			math::Degrees::new(30.0).to_radians(),
		)
		.expect("physical cone light")
	}

	/// Creates one compact shadow-capable point light for selection and projection tests.
	fn point(_position_x: f32) -> PointLight {
		PointLight::new(
			LightColor::Kelvin(4_500.0),
			PhotometricIntensity::LuminousIntensity {
				candela: 100.0,
				reference_distance_m: 1.0,
			},
		)
		.expect("physical point light")
	}

	/// Creates the retained transform paired with a local-light test payload.
	fn light_transform(position_x: f32) -> Transform {
		Transform::from_position(Point::new(position_x, 2.0, 3.0))
	}

	/// Creates a visibility sink for shadow-selection tests.
	fn sink(position: Point) -> Sink {
		Sink::new(
			View::new_perspective(math::Degrees::new(90.0), 1.0, 0.1, 100.0, position, UnitVector::z_axis()),
			Extent::square(1),
			0,
		)
	}

	#[test]
	fn shadow_selection_keeps_one_directional_light_and_four_highest_priority_cones() {
		let lights = [
			Lights::Cone(
				ConeLight::new(
					LightColor::Kelvin(4_500.0),
					PhotometricIntensity::LuminousIntensity {
						candela: 100.0,
						reference_distance_m: 1.0,
					},
					math::Radians::new(0.25),
					math::Radians::new(std::f32::consts::PI),
				)
				.expect("physical cone light"),
			),
			Lights::Cone(cone(0.0)),
			Lights::Point(
				PointLight::new(
					LightColor::Kelvin(4_500.0),
					PhotometricIntensity::LuminousIntensity {
						candela: 100.0,
						reference_distance_m: 1.0,
					},
				)
				.expect("physical point light"),
			),
			Lights::Direction(
				DirectionalLight::new(
					LightColor::Kelvin(6_500.0),
					PhotometricIntensity::Illuminance {
						lux: 100_000.0,
						measurement_distance_m: 1.0,
					},
				)
				.expect("physical directional light"),
			),
			Lights::Cone(cone(1.0)),
			Lights::Cone(cone(2.0)),
			Lights::Cone(cone(3.0)),
			Lights::Cone(cone(4.0)),
		];
		let transforms = [
			light_transform(0.0),
			light_transform(0.0),
			light_transform(0.0),
			Transform::from_rotation(math::orientation_from_direction(-UnitVector::<math::WorldSpace>::y_axis())),
			light_transform(1.0),
			light_transform(2.0),
			light_transform(3.0),
			light_transform(4.0),
		];

		let selection = select_shadow_lights(
			lights.iter().zip(&transforms),
			&[sink(Point::origin())],
			DEFAULT_CONE_SHADOW_POOL_CAPACITY,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(selection.directional.map(|(index, _)| index), Some(3));
		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[1, 4, 5, 6]
		);
		assert_eq!(selection.eligible_cone_count, 5);
		assert_eq!(
			selection
				.points
				.iter()
				.flatten()
				.map(|(index, ..)| *index)
				.collect::<Vec<_>>(),
			[2]
		);
		assert_eq!(selection.eligible_point_count, 1);
	}

	#[test]
	fn shadow_selection_keeps_cones_visible_in_any_sink_and_skips_cones_outside_all_sinks() {
		let visible_in_second_sink = cone(100.0).with_shadow_far(20.0);

		let outside_all_sinks = cone(500.0).with_shadow_far(20.0);

		let lights = [
			Lights::Cone(visible_in_second_sink.clone()),
			Lights::Cone(outside_all_sinks.clone()),
		];
		let transforms = [light_transform(100.0), light_transform(500.0)];

		let sinks = [sink(Point::origin()), sink(Point::new(100.0, 0.0, 0.0))];

		assert!(
			sinks
				.iter()
				.any(|sink| cone_shadow_importance(&visible_in_second_sink, &transforms[0], 1.0, sink).is_some())
		);
		assert!(
			sinks
				.iter()
				.all(|sink| cone_shadow_importance(&outside_all_sinks, &transforms[1], 1.0, sink).is_none())
		);

		let selection = select_shadow_lights(
			lights.iter().zip(&transforms),
			&sinks,
			DEFAULT_CONE_SHADOW_POOL_CAPACITY,
			DEFAULT_POINT_SHADOW_POOL_CAPACITY,
		);

		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[0]
		);
		assert_eq!(selection.eligible_cone_count, 1);
	}

	#[test]
	fn cone_shadow_pool_assigns_its_limited_layers_to_visible_lights() {
		let lights = [Lights::Cone(cone(0.0)), Lights::Cone(cone(1.0))];
		let transforms = [light_transform(0.0), light_transform(1.0)];

		let sinks = [sink(Point::origin())];

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 1, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		let empty_selection =
			select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[0]
		);
		assert_eq!(selection.eligible_cone_count, 2);
		assert!(empty_selection.cones.iter().all(Option::is_none));
		assert_eq!(empty_selection.eligible_cone_count, 2);
	}

	#[test]
	fn cone_shadow_pool_orders_lights_by_projected_sink_coverage() {
		let lights = [
			Lights::Cone(cone(8.0).with_shadow_far(5.0)),
			Lights::Cone(cone(0.0).with_shadow_far(5.0)),
		];
		let transforms = [light_transform(8.0), light_transform(0.0)];

		let sinks = [sink(Point::origin())];

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 1, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[1]
		);
	}

	#[test]
	fn cone_shadow_pool_continues_in_sink_order_after_assigning_each_sink_its_top_light() {
		let lights = [
			Lights::Cone(cone(0.0).with_shadow_far(20.0)),
			Lights::Cone(cone(1.0).with_shadow_far(20.0)),
			Lights::Cone(cone(2.0).with_shadow_far(20.0)),
			Lights::Cone(cone(3.0).with_shadow_far(20.0)),
			Lights::Cone(cone(100.0).with_shadow_far(20.0)),
			Lights::Cone(cone(200.0).with_shadow_far(20.0)),
		];
		let transforms = [
			light_transform(0.0),
			light_transform(1.0),
			light_transform(2.0),
			light_transform(3.0),
			light_transform(100.0),
			light_transform(200.0),
		];

		let sinks = [
			sink(Point::origin()),
			sink(Point::new(100.0, 0.0, 0.0)),
			sink(Point::new(200.0, 0.0, 0.0)),
		];

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 4, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[0, 4, 5, 1]
		);
	}

	#[test]
	fn unlit_cones_yield_pool_layers_to_visible_lit_cones() {
		let mut unlit = cone(0.0);

		unlit.color = Vec3f::new(0.0, 0.0, 0.0);

		let lights = [Lights::Cone(unlit.clone()), Lights::Cone(cone(1.0))];
		let transforms = [light_transform(0.0), light_transform(1.0)];

		let sinks = [sink(Point::origin())];

		assert!(!cone_light_has_brightness(&unlit, 1.0));

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 1, DEFAULT_POINT_SHADOW_POOL_CAPACITY);

		assert_eq!(
			selection.cones.iter().flatten().map(|(index, ..)| *index).collect::<Vec<_>>(),
			[1]
		);
		assert_eq!(selection.eligible_cone_count, 1);
	}

	#[test]
	fn point_shadow_pool_assigns_its_limited_cubes_to_visible_lights() {
		let lights = [
			Lights::Point(point(0.0)),
			Lights::Point(point(1.0)),
			Lights::Point(point(2.0)),
		];
		let transforms = [light_transform(0.0), light_transform(1.0), light_transform(2.0)];

		let sinks = [sink(Point::origin())];

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, 2);

		let empty_selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, 0);

		assert_eq!(
			selection
				.points
				.iter()
				.flatten()
				.map(|(index, ..)| *index)
				.collect::<Vec<_>>(),
			[0, 1]
		);
		assert_eq!(selection.eligible_point_count, 3);
		assert!(empty_selection.points.iter().all(Option::is_none));
		assert_eq!(empty_selection.eligible_point_count, 3);
	}

	#[test]
	fn point_shadow_pool_orders_lights_by_projected_sink_coverage() {
		let lights = [
			Lights::Point(point(3.5).with_shadow_far(1.0)),
			Lights::Point(point(0.0).with_shadow_far(1.0)),
		];
		let transforms = [light_transform(3.5), light_transform(0.0)];

		let sinks = [sink(Point::origin())];

		let selection = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, 1);

		assert_eq!(
			selection
				.points
				.iter()
				.flatten()
				.map(|(index, ..)| *index)
				.collect::<Vec<_>>(),
			[1]
		);
	}

	#[test]
	fn resolved_ies_profile_texture_applies_the_per_light_dimmer() {
		let profile_light = Lights::Point(
			PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.5, "lights/office.ies")
				.expect("physical IES point light"),
		);

		let analytic_light = Lights::Point(point(0.0));

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

	/// Verifies a resident profile's dimmed peak intensity drives both local-shadow range and selection.
	#[test]
	fn ies_profile_scale_expands_point_shadow_coverage() {
		let light = PointLight::new_ies(LightColor::LinearSrgb(Vec3f::new(1.0, 1.0, 1.0)), 0.5, "lights/office.ies")
			.expect("physical IES point light");

		let lights = [Lights::Point(light.clone())];
		let transforms = [light_transform(20.0)];

		let sinks = [sink(Point::origin())];

		let mut profiles = HashMap::default();

		profiles.insert(
			"lights/office.ies".to_string(),
			IesProfileTexture {
				texture_index: 19,
				intensity_scale_candela: 180.0,
			},
		);

		let fallback = select_shadow_lights(lights.iter().zip(&transforms), &sinks, 0, 1);

		let resident = select_shadow_lights_with_intensity_scale(lights.iter().zip(&transforms), &sinks, 0, 1, |light| {
			resolved_ies_profile_texture(light, &profiles).map_or(1.0, |profile| profile.intensity_scale_candela)
		});

		let (_, fallback_far) = resolve_point_shadow_range(&light, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let (_, resident_far) = resolve_point_shadow_range(&light, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, 90.0);

		assert!(fallback.points.iter().all(Option::is_none));
		assert_eq!(fallback.eligible_point_count, 0);
		assert_eq!(resident.points[0].map(|(index, ..)| index), Some(0));
		assert_eq!(resident.eligible_point_count, 1);
		assert!((resident_far / fallback_far - 90.0_f32.sqrt()).abs() < 0.0001);
	}

	#[test]
	fn point_shadow_views_cover_every_cube_direction_and_range() {
		let light = point(1.0).with_shadow_range(0.2, 50.0);
		let transform = light_transform(1.0);

		let directions = [
			UnitVector::x_axis(),
			-UnitVector::x_axis(),
			UnitVector::y_axis(),
			-UnitVector::y_axis(),
			UnitVector::z_axis(),
			-UnitVector::z_axis(),
		];

		for (face, direction) in directions.into_iter().enumerate() {
			let view = make_point_shadow_view(&light, &transform, face, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

			let point = (transform.get_position() + direction * 10.0).into_maths();

			let clip = view.view_projection() * Vec4f::new(point.x, point.y, point.z, 1.0);

			let ndc = clip / clip.w;

			assert!((view.y_fov().value() - 90.0).abs() < 0.0001);
			assert_eq!(view.near(), 0.2);
			assert_eq!(view.far(), 50.0);
			assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001);
			assert!((0.0..=1.0).contains(&ndc.z));
		}

		let positive_y_view = make_point_shadow_view(&light, &transform, 2, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let right_of_positive_y_face =
			(transform.get_position() + UnitVector::y_axis() * 10.0 + UnitVector::x_axis()).into_maths();

		let clip = positive_y_view.view_projection()
			* Vec4f::new(
				right_of_positive_y_face.x,
				right_of_positive_y_face.y,
				right_of_positive_y_face.z,
				1.0,
			);

		assert!((clip.x / clip.w) > 0.0);
	}

	#[test]
	fn point_shadow_range_uses_manual_endpoints_and_visibility() {
		let light = point(500.0).with_shadow_range(-4.0, f32::NAN);
		let transform = light_transform(500.0);

		let (near, far) = resolve_point_shadow_range(&light, POINT_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let automatic_far = (100.0 / POINT_SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert_eq!(near, POINT_SHADOW_NEAR_M);
		assert_eq!(far, automatic_far);
		assert!(
			point_shadow_importance(&light.clone().with_shadow_far(20.0), &transform, 1.0, &sink(Point::origin())).is_none()
		);
		assert!(
			point_shadow_importance(
				&point(100.0).with_shadow_far(20.0),
				&light_transform(100.0),
				1.0,
				&sink(Point::new(100.0, 0.0, 0.0)),
			)
			.is_some()
		);

		let mut unlit = point(0.0);

		unlit.color = Vec3f::new(0.0, 0.0, 0.0);

		assert!(!point_light_has_brightness(&unlit, 1.0));
		assert_eq!(POINT_SHADOW_FACE_COUNT, 6);
	}

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
	fn cone_shadow_view_uses_the_light_projection_and_automatic_clip_range() {
		let light = cone(1.0);
		let transform = light_transform(1.0);

		let view = make_cone_shadow_view(&light, &transform, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let point = transform.get_position() + math::direction_from_orientation(transform.get_orientation()) * 10.0;

		let point = point.into_maths();

		let clip = view.view_projection() * Vec4f::new(point.x, point.y, point.z, 1.0);

		let ndc = clip / clip.w;

		let automatic_far = (100.0 / CONE_SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert!((view.y_fov().value() - 60.0).abs() < 0.0001);
		assert_eq!(view.near(), CONE_SHADOW_NEAR_M);
		assert_eq!(CONE_SHADOW_EXPOSURE_THRESHOLD_LUX, 0.125);
		assert!((view.far() - automatic_far).abs() < 0.0001);
		assert!(ndc.x.abs() < 0.0001 && ndc.y.abs() < 0.0001);
		assert!((0.0..=1.0).contains(&ndc.z));
	}

	#[test]
	fn cone_shadow_range_uses_manual_endpoints_and_clamps_invalid_values() {
		let light = cone(1.0).with_shadow_range(-4.0, f32::NAN);

		let (near, far) = resolve_cone_shadow_range(&light, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let automatic_far = (100.0 / CONE_SHADOW_EXPOSURE_THRESHOLD_LUX).sqrt();

		assert_eq!(near, CONE_SHADOW_NEAR_M);
		assert!((far - automatic_far).abs() < 0.0001);

		let light = cone(1.0).with_shadow_near(50.0).with_shadow_far(20.0);

		assert_eq!(
			resolve_cone_shadow_range(&light, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0),
			(50.0, 50.1)
		);
	}

	#[test]
	fn cone_shadow_range_scales_with_linear_exposure() {
		let light = cone(1.0);

		let (_, neutral_far) = resolve_cone_shadow_range(&light, CONE_SHADOW_DEFAULT_EXPOSURE_SCALE, 1.0);

		let (_, brighter_far) = resolve_cone_shadow_range(&light, 4.0, 1.0);

		let (_, invalid_far) = resolve_cone_shadow_range(&light, f32::NAN, 1.0);

		assert!((brighter_far - neutral_far * 2.0).abs() < 0.0001);
		assert!((invalid_far - neutral_far).abs() < 0.0001);
	}

	#[test]
	fn lit_binding_supports_transparent_read_modify_write() {
		assert_eq!(LIT_BINDING.access(), ghi::AccessPolicies::READ_WRITE);
	}

	#[test]
	fn material_data_defaults_every_texture_slot_to_missing() {
		let material_data = MaterialData::default();

		assert!(material_data.textures.iter().all(|texture_index| *texture_index == u32::MAX));
	}

	#[test]
	fn material_texture_updates_replace_the_complete_canonical_record() {
		let mut material_data = MaterialData {
			textures: [41; MAX_MATERIAL_TEXTURES],
			..MaterialData::default()
		};

		assert!(!write_material_texture_indices(&mut material_data, [Some(7), None, Some(11)]));
		assert_eq!(material_data.textures[..3], [7, u32::MAX, 11]);
		assert!(
			material_data.textures[3..]
				.iter()
				.all(|texture_index| *texture_index == u32::MAX)
		);
		assert!(!write_material_texture_indices(&mut material_data, [Some(3)]));
		assert_eq!(material_data.textures[0], 3);
		assert!(
			material_data.textures[1..]
				.iter()
				.all(|texture_index| *texture_index == u32::MAX)
		);
	}

	#[test]
	fn material_texture_updates_report_truncated_slots() {
		let mut material_data = MaterialData::default();

		assert!(write_material_texture_indices(
			&mut material_data,
			[Some(5); MAX_MATERIAL_TEXTURES + 1]
		));
		assert!(material_data.textures.iter().all(|texture_index| *texture_index == 5));
	}

	/// Verifies authored alpha modes partition solid, clipped, and blended raster work.
	#[test]
	fn active_instances_partition_by_authored_alpha_mode() {
		let mut render_info = RenderInfo {
			opaque_instances: Vec::new(),
			masked_instances: Vec::new(),
			transparent_instances: Vec::new(),
			skinning_dispatches: Vec::new(),
			opaque_materials: Vec::new(),
			transparent_materials: Vec::new(),
			opaque_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
			transparent_material_mask: [0; MAX_MATERIALS / u64::BITS as usize],
		};

		let blended = Instance {
			shader_mesh_index: 3,
			meshlet_count: 1,
		};

		let opaque = Instance {
			shader_mesh_index: 5,
			meshlet_count: 2,
		};

		let masked = Instance {
			shader_mesh_index: 8,
			meshlet_count: 3,
		};

		render_info.push_active_instance(blended, 7, &AlphaMode::Blend);

		render_info.push_active_instance(opaque, 11, &AlphaMode::Opaque);

		render_info.push_active_instance(masked, 68, &AlphaMode::Mask(0.5));

		assert_eq!(render_info.opaque_instances, [opaque]);
		assert_eq!(render_info.masked_instances, [masked]);
		assert_eq!(render_info.transparent_instances, [blended]);
		assert_eq!(render_info.opaque_material_mask[0], 1 << 11);
		assert_eq!(render_info.opaque_material_mask[1], 1 << 4);
		assert_eq!(render_info.transparent_material_mask[0], 1 << 7);
		assert_eq!(render_info.active_instance_count(), 3);
	}

	#[test]
	fn shader_mesh_matches_gpu_buffer_layout() {
		let (expected_size, expected_material_offset) = (80, 48);

		assert_eq!(
			std::mem::size_of::<ShaderMesh>(),
			expected_size,
			"Unexpected Visibility shader mesh size. The most likely cause is that the CPU-side mesh buffer layout drifted from the shader struct array stride."
		);
		assert_eq!(
			std::mem::size_of::<ShaderMesh>() as u32,
			MESH_DATA_BUFFER_STRIDE,
			"Unexpected Visibility shader mesh binding stride. The most likely cause is that the descriptor stride no longer matches the CPU-side mesh buffer layout."
		);
		assert_eq!(
			std::mem::align_of::<ShaderMesh>(),
			16,
			"Unexpected Visibility shader mesh alignment. The most likely cause is that the CPU-side mesh buffer no longer matches the shader struct alignment."
		);
		assert_eq!(
			std::mem::offset_of!(ShaderMesh, material_index),
			expected_material_offset,
			"Unexpected Visibility shader mesh material offset. The most likely cause is that the CPU-side mesh fields no longer match the shader struct."
		);
		assert_eq!(
			std::mem::offset_of!(ShaderMesh, skinned_base_vertex_index),
			expected_material_offset + 24,
			"Unexpected Visibility skinned vertex offset. The most likely cause is that the CPU-side mesh fields no longer match the visibility and material shader structs."
		);
	}

	#[test]
	fn shader_view_data_matches_compact_gpu_buffer_layout() {
		let (expected_size, expected_view_projection_offset, expected_inverse_view_offset, expected_fov_offset) =
			(176, 48, 112, 160);

		assert_eq!(
			std::mem::size_of::<ShaderViewData>(),
			expected_size,
			"Unexpected compact visibility view size. The most likely cause is that the CPU view record drifted from its shader storage layout."
		);
		assert_eq!(std::mem::size_of::<ShaderViewData>() as u32, VIEW_DATA_BUFFER_STRIDE);
		assert_eq!(std::mem::offset_of!(ShaderViewData, view), 0);
		assert_eq!(
			std::mem::offset_of!(ShaderViewData, view_projection),
			expected_view_projection_offset
		);
		assert_eq!(
			std::mem::offset_of!(ShaderViewData, inverse_view),
			expected_inverse_view_offset
		);
		assert_eq!(std::mem::offset_of!(ShaderViewData, fov), expected_fov_offset);
		assert_eq!(std::mem::offset_of!(ShaderViewData, near), expected_fov_offset + 8);
		assert_eq!(std::mem::offset_of!(ShaderViewData, far), expected_fov_offset + 12);
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
		let mut factory = Factory::new();

		let first_handle = factory.create(());

		let second_handle = factory.create(());

		let first_binding = Arc::new(SkinBinding { entries: Vec::new() });

		let second_binding = Arc::new(SkinBinding { entries: Vec::new() });

		let palette_cache = vec![
			SkinningPaletteCacheEntry {
				handle: first_handle,
				binding: Arc::as_ptr(&first_binding),
				palette_base: 7,
				palette_kind: SkinningPaletteKind::DualQuaternion,
			},
			SkinningPaletteCacheEntry {
				handle: first_handle,
				binding: Arc::as_ptr(&second_binding),
				palette_base: 11,
				palette_kind: SkinningPaletteKind::Matrix,
			},
			SkinningPaletteCacheEntry {
				handle: second_handle,
				binding: Arc::as_ptr(&first_binding),
				palette_base: 17,
				palette_kind: SkinningPaletteKind::Matrix,
			},
		];

		assert_eq!(
			cached_skin_palette(&palette_cache, first_handle, Arc::as_ptr(&first_binding)),
			Some((7, SkinningPaletteKind::DualQuaternion))
		);
		assert_eq!(
			cached_skin_palette(&palette_cache, first_handle, Arc::as_ptr(&second_binding)),
			Some((11, SkinningPaletteKind::Matrix))
		);
		assert_eq!(
			cached_skin_palette(&palette_cache, second_handle, Arc::as_ptr(&first_binding)),
			Some((17, SkinningPaletteKind::Matrix))
		);
	}

	#[test]
	fn lighting_data_matches_gpu_buffer_layout() {
		assert_eq!(
			std::mem::size_of::<LightData>(),
			112,
			"Unexpected visibility LightData size. The most likely cause is that the CPU light buffer layout drifted from the generated shader struct."
		);
		assert_eq!(
			std::mem::align_of::<LightData>(),
			16,
			"Unexpected visibility LightData alignment. The most likely cause is that ShaderVec3 padding changed."
		);
		assert_eq!(std::mem::offset_of!(LightData, position), 0);
		assert_eq!(std::mem::offset_of!(LightData, color), 16);
		assert_eq!(std::mem::offset_of!(LightData, direction), 32);
		assert_eq!(std::mem::offset_of!(LightData, cone_cosines), 48);
		assert_eq!(std::mem::offset_of!(LightData, light_type), 56);
		assert_eq!(std::mem::offset_of!(LightData, shadow_views), 60);
		assert_eq!(std::mem::offset_of!(LightData, shadow_layer), 92);
		assert_eq!(std::mem::offset_of!(LightData, ies_profile_texture), 96);
		assert_eq!(std::mem::offset_of!(LightData, ies_c0_tangent), 100);
		assert_eq!(std::mem::offset_of!(LightData, _ies_padding), 104);
		assert_eq!(
			std::mem::size_of::<LightingData>(),
			1808,
			"Unexpected visibility LightingData size. The most likely cause is that the CPU lighting buffer no longer matches the shader struct array stride."
		);
		assert_eq!(std::mem::align_of::<LightingData>(), 16);
		assert_eq!(std::mem::offset_of!(LightingData, count), 0);
		assert_eq!(std::mem::offset_of!(LightingData, _padding), 4);
		assert_eq!(std::mem::offset_of!(LightingData, lights), 16);
	}
}

const LIT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1041),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::READ_WRITE,
);

const LIGHTING_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1045),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(std::mem::size_of::<LightingData>() as u32);

const MATERIALS_DATA_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1046),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
)
.buffer_stride(std::mem::size_of::<MaterialData>() as u32);

const AO_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1051),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);

const SHADOW_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1052),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);

const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1053),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);

const CONE_SHADOW_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1064),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);

const POINT_SHADOW_MAP_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1065),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::TextureCubeArray);

const ENVIRONMENT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1054),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::TextureCube);

const SPECULAR_ENVIRONMENT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1055),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::TextureCube);

use std::borrow::Borrow;
use std::cell::RefCell;
use std::collections::{HashSet, hash_map::Entry};
use std::num::NonZeroU32;
use std::ops::{Deref, DerefMut};
use std::sync::Arc;

use ::core::slice::SlicePattern;
#[cfg(target_os = "macos")]
use ghi::Size as _;
use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
#[cfg(target_os = "macos")]
use ghi::io::{ResourceIoContext as _, ResourceIoQueue as _, ResourceIoTicket as _};
use log::{error, warn};
use math::{AffineShaderMatrix, Matrix, ShaderMatrix, UnitVector};
use maths_rs::Vec4f;
use resource_management::asset::handler::implementations::bema::ProgramGenerator;
use resource_management::resource::resource_manager::ResourceManager;
use resource_management::resources::mesh::{Mesh as ResourceMesh, Primitive};
use resource_management::resources::skeleton::{AffineMatrix4x3Columns, SkinBinding, identity_affine_matrix4x3_columns};
use resource_management::shader::besl::backends::glsl::GLSLTranspiler;
use resource_management::shader::besl::backends::msl::MSLTranspiler;
use resource_management::shader::generator::{ShaderGenerationSettings, ShaderGenerator};
use resource_management::types::{AlphaMode, IndexStreamTypes, IntegralTypes, ShaderTypes};
use smallvec::SmallVec;
use utils::hash::{HashMap, HashMapExt};
use utils::json::{self, object};
use utils::sync::{Rc, RwLock};
use utils::{AvailabilityGraph, AvailabilityHandle, Box, Extent, RGBA, StableVec};

use super::shader_generator::{VisibilityShaderGenerator, VisibilityShaderScope};
use crate::core::{
	Entity, EntityHandle,
	factory::Handle,
	listener::{DefaultListener, Listener as _},
};
use crate::gameplay::Transform;
use crate::ghi;
use crate::rendering::lights::{ConeLight, DirectionalLight, Light, Lights, PointLight};
use crate::rendering::mesh::generator::MeshGenerator;
use crate::rendering::pipeline_manager::PipelineManager;
use crate::rendering::pipelines::visibility::gpu_vertex_data_manager::GPUVertexDataManager;
use crate::rendering::pipelines::visibility::render_pass::{
	DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT, VisibilityPipelineRenderPass,
};
use crate::rendering::pipelines::visibility::resource_manager::{
	IBL_SPECULAR_LEVEL_COUNT, VisibilityMeshKey, VisibilityPipelineResourceManagerClient, VisibilityRenderResource,
	VisibilityResourceCompletion, VisibilityTextureKey, resource_image_format_to_ghi,
};
use crate::rendering::pipelines::visibility::scene_manager::VisibilitySceneManager;
use crate::rendering::pipelines::visibility::skinning::{
	DualQuaternion, MAX_SKINNED_VERTICES, MAX_SKINNING_MATRICES, SkinningDispatch, SkinningPaletteKind, SkinningPass,
	SkinningSourceBuffers, append_dual_quaternion_palette,
};
use crate::rendering::pipelines::visibility::{
	ActiveMaterialMask, CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_VIEW_OFFSET, DEFAULT_CONE_SHADOW_POOL_CAPACITY,
	DEFAULT_POINT_SHADOW_POOL_CAPACITY, DIRECTIONAL_SHADOW_MAP_FORMAT, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING,
	MATERIAL_EVALUATION_DISPATCHES_BINDING, MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING,
	MAX_BINDLESS_TEXTURES, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_INSTANCES, MAX_LIGHTS, MAX_MATERIAL_TEXTURES, MAX_MATERIALS,
	MAX_MESHLETS, MAX_PIXEL_MAPPING_ENTRIES, MAX_POINT_SHADOW_POOL_CAPACITY, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES,
	MAX_VERTICES, MESH_DATA_BINDING, MESHLET_DATA_BINDING, POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_FORMAT,
	POINT_SHADOW_VIEW_OFFSET, PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, SHADOW_VIEW_COUNT,
	SKINNED_VERTICES_BINDING, ShaderMeshletData, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING, VERTEX_INDICES_BINDING,
	VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::{FramePrepare, RenderPass, RenderPassBuilder, RenderPassReturn};
use crate::rendering::renderable::mesh::MeshSource;
use crate::rendering::resource_loading::{NativeTextureUpload, TextureMetadata};
use crate::rendering::view::View;
use crate::rendering::{
	Environment, RenderableMesh, Sink, csm, make_perspective_view_from_camera, map_shader_binding_to_shader_binding_descriptor,
	mesh,
};
use crate::resource_management::{self};
use crate::space::{Orientable as _, Positionable as _};
