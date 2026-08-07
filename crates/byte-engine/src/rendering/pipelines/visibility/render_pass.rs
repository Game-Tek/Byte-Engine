use ghi::command_buffer::{
	BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
	CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
};
use ghi::context::{Context as _, ContextCreate as _};
use ghi::frame::Frame as _;
use ghi::implementation::Frame;
use resource_management::{resource::resource_manager::ResourceManager, types::ShaderTypes as ResourceShaderTypes};
use utils::{Box, Extent, RGBA};

use crate::configuration::ConfigurationValue;
use crate::rendering::pipelines::visibility::mesh_dispatch::MeshDispatch;
use crate::rendering::pipelines::visibility::pipeline_manager::Instance;
use crate::rendering::pipelines::visibility::skinning::{SkinningDispatch, SkinningPass};
use crate::rendering::pipelines::visibility::{
	ActiveMaterialMask, CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET,
	DIRECTIONAL_SHADOW_MAP_FORMAT, INSTANCE_ID_BINDING, MATERIAL_COUNT_BINDING, MATERIAL_EVALUATION_DISPATCHES_BINDING,
	MATERIAL_OFFSET_BINDING, MATERIAL_OFFSET_SCRATCH_BINDING, MATERIAL_XY_BINDING, MAX_CONE_SHADOWS, MAX_INSTANCES, MAX_LIGHTS,
	MAX_MATERIALS, MAX_MESHLETS, MAX_PRIMITIVE_TRIANGLES, MAX_TRIANGLES, MAX_VERTICES, MESHLET_DATA_BINDING, MESH_DATA_BINDING,
	PRIMITIVE_INDICES_BINDING, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION, TEXTURES_BINDING, TRIANGLE_INDEX_BINDING,
	VERTEX_INDICES_BINDING, VERTEX_NORMALS_BINDING, VERTEX_POSITIONS_BINDING, VERTEX_UV_BINDING, VIEWS_DATA_BINDING,
};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{render_pass::RenderPassReturn, RenderPass, Sink};

const GTAO_VIEW_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(0),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const GTAO_PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_BLUR_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_BLUR_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_BLUR_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_UPSCALE_LOW_RESOLUTION_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_1_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_2_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_OUTPUT_3_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const GTAO_DEPTH_PYRAMID_MIP_COUNT: u32 = 4;
const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_OUTPUT_1_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
pub(super) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT: u32 = 1;

/// Returns the directional cascade view indices that receive one batched shadow dispatch.
fn directional_shadow_view_indices(mesh_dispatch: MeshDispatch) -> impl Iterator<Item = u32> {
	let has_work = !mesh_dispatch.is_empty();
	(1..=SHADOW_CASCADE_COUNT as u32).filter(move |_| has_work)
}

/// Returns the packed cone view and target-layer indices that receive shadow dispatches.
fn cone_shadow_view_indices(mesh_dispatch: MeshDispatch, cone_shadow_count: usize) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		cone_shadow_count.min(MAX_CONE_SHADOWS)
	};
	(0..count).map(|layer| ((CONE_SHADOW_VIEW_OFFSET + layer) as u32, layer as u32))
}

pub(crate) const GTAO_CONFIGURATION_PREFIX: &str = "render.gtao.";
const GTAO_MIN_SAMPLES_PER_RAY: u32 = 1;
const GTAO_MAX_SAMPLES_PER_RAY: u32 = 32;
const GTAO_MIN_RADIAL_RAYS: u32 = 2;
const GTAO_MAX_RADIAL_RAYS: u32 = 32;

/// The `GtaoSettings` struct defines the runtime quality and world-space search controls for GTAO.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GtaoSettings {
	pub(crate) radius: f32,
	pub(crate) samples_per_ray: u32,
	pub(crate) radial_rays: u32,
}

impl Default for GtaoSettings {
	fn default() -> Self {
		Self {
			radius: 1.0,
			samples_per_ray: 4,
			radial_rays: 6,
		}
	}
}

impl GtaoSettings {
	/// Applies one runtime parameter without changing the settings when validation fails.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"radius" => {
				let radius = configuration_float(value).ok_or_else(|| {
					"GTAO radius was not set. The most likely cause is that the value is not a finite number.".to_string()
				})?;
				if radius < 0.0 || radius > f32::MAX as f64 {
					return Err(
						"GTAO radius was not set. The most likely cause is that the value is outside the nonnegative f32 range."
							.to_string(),
					);
				}
				let settings = Self {
					radius: radius as f32,
					..self
				};
				Ok((settings, ConfigurationValue::Float(f64::from(settings.radius))))
			}
			"samples-per-ray" => {
				let samples_per_ray = configuration_u32(value).ok_or_else(|| {
					"GTAO samples-per-ray was not set. The most likely cause is that the value is not a whole number."
				})?;
				if !(GTAO_MIN_SAMPLES_PER_RAY..=GTAO_MAX_SAMPLES_PER_RAY).contains(&samples_per_ray) {
					return Err(format!(
						"GTAO samples-per-ray was not set. The most likely cause is that the value is outside the supported range {}..={}.",
						GTAO_MIN_SAMPLES_PER_RAY, GTAO_MAX_SAMPLES_PER_RAY
					));
				}
				let settings = Self { samples_per_ray, ..self };
				Ok((settings, ConfigurationValue::Integer(i64::from(samples_per_ray))))
			}
			"radial-rays" => {
				let radial_rays = configuration_u32(value).ok_or_else(|| {
					"GTAO radial-rays was not set. The most likely cause is that the value is not a whole number."
				})?;
				if !(GTAO_MIN_RADIAL_RAYS..=GTAO_MAX_RADIAL_RAYS).contains(&radial_rays) || radial_rays % 2 != 0 {
					return Err(format!(
						"GTAO radial-rays was not set. The most likely cause is that the value must be an even number in the range {}..={}.",
						GTAO_MIN_RADIAL_RAYS, GTAO_MAX_RADIAL_RAYS
					));
				}
				let settings = Self { radial_rays, ..self };
				Ok((settings, ConfigurationValue::Integer(i64::from(radial_rays))))
			}
			_ => {
				Err("GTAO parameter was not set. The most likely cause is that the parameter name is unsupported.".to_string())
			}
		}
	}

	fn shader_parameters(self) -> GtaoShaderParameters {
		GtaoShaderParameters {
			radius: self.radius,
			samples_per_ray: self.samples_per_ray,
			radial_rays: self.radial_rays,
		}
	}
}

fn configuration_float(value: &ConfigurationValue) -> Option<f64> {
	match value {
		ConfigurationValue::Integer(value) => Some(*value as f64),
		ConfigurationValue::Float(value) if value.is_finite() => Some(*value),
		ConfigurationValue::Text(value) => value.parse().ok().filter(|value: &f64| value.is_finite()),
		ConfigurationValue::Bool(_) | ConfigurationValue::Float(_) => None,
	}
}

fn configuration_u32(value: &ConfigurationValue) -> Option<u32> {
	match value {
		ConfigurationValue::Integer(value) => (*value).try_into().ok(),
		ConfigurationValue::Float(value) if value.is_finite() && value.fract() == 0.0 => (*value as i128).try_into().ok(),
		ConfigurationValue::Text(value) => value.parse().ok(),
		ConfigurationValue::Bool(_) | ConfigurationValue::Float(_) => None,
	}
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct GtaoShaderParameters {
	radius: f32,
	samples_per_ray: u32,
	radial_rays: u32,
}

/// The `FastGtaoViewData` struct provides compact camera reconstruction constants to the GTAO compute passes.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct FastGtaoViewData {
	pixel_to_ray_mul: [f32; 2],
	pixel_to_ray_add: [f32; 2],
	projection_pixels_y: f32,
	view_z_sign: f32,
	depth_unproject_numerator: f32,
	depth_unproject_denominator_offset: f32,
}

/// Builds pixel-ray and reversed-depth reconstruction constants for one perspective sink.
fn fast_gtao_view_data(sink: &Sink, extent: Extent) -> FastGtaoViewData {
	let view = sink.view();
	let projection = view.projection();
	let width = extent.width() as f32;
	let height = extent.height() as f32;
	let projection_x = projection[0];
	let projection_y = projection[5];
	let near = view.near();
	let far = view.far();
	let clip_range = far - near;

	debug_assert!(
		width > 0.0 && height > 0.0 && projection_x > 0.0 && projection_y > 0.0,
		"GTAO camera constants are invalid. The most likely cause is an empty target or a non-perspective sink."
	);
	debug_assert!(
		near > 0.0 && far > near,
		"GTAO clipping planes are invalid. The most likely cause is a perspective view with an empty depth range."
	);

	FastGtaoViewData {
		pixel_to_ray_mul: [2.0 / (width * projection_x), -2.0 / (height * projection_y)],
		pixel_to_ray_add: [(1.0 / width - 1.0) / projection_x, (1.0 - 1.0 / height) / projection_y],
		projection_pixels_y: height * projection_y * 0.5,
		// Byte Engine perspective views look down positive view-space Z.
		view_z_sign: 1.0,
		// projection_matrix() maps z to depth as a + b / z. These constants
		// reconstruct positive z as b / (depth - a).
		depth_unproject_numerator: near * far / clip_range,
		depth_unproject_denominator_offset: near / clip_range,
	}
}

/// Loads one fixed visibility shader from the application resource store and verifies its persisted stage contract.
fn load_visibility_shader(
	context: &mut ghi::implementation::Context,
	resources: &ResourceManager,
	id: &str,
	name: &str,
	expected_stage: ResourceShaderTypes,
) -> ghi::ShaderHandle {
	let loaded = crate::rendering::resource_loading::load_shader(context, resources, id, name)
		.unwrap_or_else(|error| panic!("Failed to load visibility shader '{id}': {error}"));
	assert_eq!(
		loaded.stage, expected_stage,
		"Visibility shader stage mismatch for '{id}'. The most likely cause is incorrect shader sidecar metadata."
	);
	loaded.handle
}

/// The `VisibilityPass` struct owns the depth-writing raster state used to populate visibility buffers.
#[derive(Clone)]
pub(crate) struct VisibilityPass {
	descriptor_set: ghi::DescriptorSetHandle,
	pipeline: ghi::PipelineHandle,
	opaque_attachments: [ghi::AttachmentInformation; 3],
	transparent_attachments: [ghi::AttachmentInformation; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VisibilityPhase {
	Opaque,
	Transparent,
}

impl VisibilityPhase {
	fn label(self) -> &'static str {
		match self {
			Self::Opaque => "Opaque",
			Self::Transparent => "Transparent",
		}
	}

	fn blend_flag(self) -> u32 {
		match self {
			Self::Opaque => 0,
			Self::Transparent => 1,
		}
	}
}

impl VisibilityPass {
	/// Creates the visibility pipeline and phase-specific attachment behavior.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		descriptor_set: ghi::DescriptorSetHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		depth_target: ghi::BaseImageHandle,
	) -> Self {
		let visibility_pass_task_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/visibility-task.besl",
			"Visibility Pass Task Shader",
			ResourceShaderTypes::Task,
		);
		let visibility_pass_mesh_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/visibility-mesh.besl",
			"Visibility Pass Mesh Shader",
			ResourceShaderTypes::Mesh,
		);
		let visibility_pass_fragment_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/visibility-fragment.besl",
			"Visibility Pass Fragment Shader",
			ResourceShaderTypes::Fragment,
		);

		let mut visibility_pass_shaders = Vec::with_capacity(3);
		visibility_pass_shaders.push(ghi::ShaderParameter::new(
			&visibility_pass_task_shader,
			ghi::ShaderTypes::Task,
		));
		visibility_pass_shaders.push(ghi::ShaderParameter::new(
			&visibility_pass_mesh_shader,
			ghi::ShaderTypes::Mesh,
		));
		visibility_pass_shaders.push(ghi::ShaderParameter::new(
			&visibility_pass_fragment_shader,
			ghi::ShaderTypes::Fragment,
		));

		let pipeline_attachments = [
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::U32),
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::U32),
			ghi::pipelines::raster::AttachmentDescriptor::new(ghi::Formats::Depth32),
		];

		let vertex_layout = [
			ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::pipelines::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let pipeline = context.create_raster_pipeline(
			ghi::pipelines::raster::Builder::new(
				&[ghi::pipelines::PushConstantRange::new(0, 12)],
				&vertex_layout,
				&visibility_pass_shaders,
				&pipeline_attachments,
			)
			.name("Visibility Pass Mesh Shader"),
		);

		VisibilityPass {
			descriptor_set,
			pipeline,
			opaque_attachments: [
				ghi::AttachmentInformation::new(
					primitive_index,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				),
			],
			transparent_attachments: [
				ghi::AttachmentInformation::new(
					primitive_index,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					instance_id,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Integer(u32::MAX, 0, 0, 0),
					false,
					true,
				),
				ghi::AttachmentInformation::new(
					depth_target,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					true,
					true,
				),
			],
		}
	}

	/// Records one visibility phase.
	///
	/// The transparent phase loads opaque depth, then writes the nearest transparent
	/// surface into it. This preserves opaque occlusion while resolving overlapping
	/// triangles within the single transparent layer represented by the visibility buffer.
	fn record(
		&self,
		c: &mut ghi::implementation::CommandBufferRecording,
		extent: Extent,
		instances: &[Instance],
		mesh_dispatch: MeshDispatch,
		phase: VisibilityPhase,
	) {
		let attachments: &[ghi::AttachmentInformation] = match phase {
			VisibilityPhase::Opaque => &self.opaque_attachments,
			VisibilityPhase::Transparent => &self.transparent_attachments,
		};
		let drawable_instances = instances.iter().filter(|instance| instance.meshlet_count > 0).count();
		let meshlet_count = instances.iter().map(|instance| instance.meshlet_count).sum::<u32>();

		log::debug!(
			"{} visibility pass executing: extent={}x{}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
			phase.label(),
			extent.width(),
			extent.height(),
			instances.len(),
			drawable_instances,
			meshlet_count,
			mesh_dispatch.workgroup_count(),
		);
		c.start_region(|label| {
			label.write_str(phase.label())?;
			label.write_str(" Visibility Buffer")
		});

		let c = c.start_render_pass(extent, attachments);
		if !mesh_dispatch.is_empty() {
			let c = c.bind_raster_pipeline(self.pipeline);
			c.bind_descriptor_sets(&[self.descriptor_set]);
			c.write_push_constant(0, mesh_dispatch.work_item_base());
			c.write_push_constant(4, 0u32);
			c.write_push_constant(8, 0u32);
			c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
		}

		c.end_render_pass();
		c.end_region();
	}
}

/// Returns the one depth-resolved transparent layer supported by the visibility buffer.
fn transparent_visibility_layer(instances: &[Instance]) -> Option<&[Instance]> {
	instances
		.iter()
		.any(|instance| instance.meshlet_count > 0)
		.then_some(instances)
}

/// The `ShadowPass` struct owns the shared pipeline and depth targets used by directional and cone shadow rendering.
pub struct ShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	directional_shadow_depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	directional_shadow_pass_pipeline: ghi::PipelineHandle,
	directional_shadow_depth_pyramid_pipeline: ghi::PipelineHandle,
	cone_shadow_pass_pipeline: ghi::PipelineHandle,
	directional_shadow_map: ghi::BaseImageHandle,
	cone_shadow_map: ghi::BaseImageHandle,
}

impl ShadowPass {
	/// Creates raster pipelines that match the directional and cone shadow-map depth formats.
	fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		descriptor_set: ghi::DescriptorSetHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		directional_shadow_depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
	) -> Self {
		let directional_shadow_depth_pyramid_descriptor_set =
			context.create_descriptor_set(Some("Directional Shadow Depth Pyramid Descriptor Set"));
		let shadow_depth_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod(0.0),
		);
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				directional_shadow_depth_pyramid_descriptor_set,
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				directional_shadow_map,
				shadow_depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				directional_shadow_depth_pyramid_descriptor_set,
				DIRECTIONAL_SHADOW_DEPTH_PYRAMID_OUTPUT_1_BINDING.slot(),
				directional_shadow_depth_pyramid,
				ghi::Layouts::General,
				0,
			),
		]);
		let shadow_pass_task_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/shadow-task.besl",
			"Shadow Pass Task Shader",
			ResourceShaderTypes::Task,
		);
		let shadow_pass_mesh_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/shadow-mesh.besl",
			"Shadow Pass Mesh Shader",
			ResourceShaderTypes::Mesh,
		);
		let directional_shadow_depth_pyramid_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/directional-shadow-depth-pyramid.besl",
			"Directional Shadow Depth Pyramid Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let directional_attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(
			DIRECTIONAL_SHADOW_MAP_FORMAT,
		)];
		let cone_attachments = [ghi::pipelines::raster::AttachmentDescriptor::new(CONE_SHADOW_MAP_FORMAT)];
		let vertex_layout = [
			ghi::pipelines::VertexElement::new("POSITION", ghi::DataTypes::Float3, 0),
			ghi::pipelines::VertexElement::new("NORMAL", ghi::DataTypes::Float3, 1),
		];

		let mut shadow_pass_shaders = Vec::with_capacity(2);
		shadow_pass_shaders.push(ghi::ShaderParameter::new(&shadow_pass_task_shader, ghi::ShaderTypes::Task));
		shadow_pass_shaders.push(ghi::ShaderParameter::new(&shadow_pass_mesh_shader, ghi::ShaderTypes::Mesh));

		let directional_shadow_pass_pipeline = context.create_raster_pipeline(
			ghi::pipelines::raster::Builder::new(
				&[ghi::pipelines::PushConstantRange::new(0, 12)],
				&vertex_layout,
				&shadow_pass_shaders,
				&directional_attachments,
			)
			.name("Shadow Pass Mesh Shader (Directional)"),
		);
		let cone_shadow_pass_pipeline = context.create_raster_pipeline(
			ghi::pipelines::raster::Builder::new(
				&[ghi::pipelines::PushConstantRange::new(0, 12)],
				&vertex_layout,
				&shadow_pass_shaders,
				&cone_attachments,
			)
			.name("Shadow Pass Mesh Shader (Cone)"),
		);
		let directional_shadow_depth_pyramid_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&directional_shadow_depth_pyramid_shader, ghi::ShaderTypes::Compute),
			)
			.name("Directional Shadow Depth Pyramid Compute Shader"),
		);

		Self {
			descriptor_set,
			directional_shadow_depth_pyramid_descriptor_set,
			directional_shadow_pass_pipeline,
			directional_shadow_depth_pyramid_pipeline,
			cone_shadow_pass_pipeline,
			directional_shadow_map,
			cone_shadow_map,
		}
	}

	/// Prepares directional cascades and packed cone layers for the current scene geometry.
	fn prepare<'a>(
		&self,
		frame: &mut ghi::implementation::Frame,
		instances: &'a [Instance],
		mesh_dispatch: MeshDispatch,
		directional_shadow_enabled: bool,
		cone_shadow_count: usize,
	) -> impl RenderPassFunction + use<'a> {
		let descriptor_set = self.descriptor_set;
		let directional_shadow_depth_pyramid_descriptor_set = self.directional_shadow_depth_pyramid_descriptor_set;
		let directional_pipeline = self.directional_shadow_pass_pipeline;
		let directional_shadow_depth_pyramid_pipeline = self.directional_shadow_depth_pyramid_pipeline;
		let cone_pipeline = self.cone_shadow_pass_pipeline;
		let directional_shadow_map = self.directional_shadow_map;
		let cone_shadow_map = self.cone_shadow_map;
		let directional_extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let directional_shadow_depth_pyramid_extent = Extent::rectangle(
			SHADOW_MAP_RESOLUTION / 2,
			SHADOW_MAP_RESOLUTION / 2 * SHADOW_CASCADE_COUNT as u32,
		);
		let cone_extent = Extent::square(CONE_SHADOW_MAP_RESOLUTION);
		let drawable_instances = instances.iter().filter(|instance| instance.meshlet_count > 0).count();
		let meshlet_count = instances.iter().map(|instance| instance.meshlet_count).sum::<u32>();

		if directional_shadow_enabled {
			frame.resize_image(directional_shadow_map, directional_extent);
		}
		if cone_shadow_count > 0 {
			frame.resize_image(cone_shadow_map, cone_extent);
		}

		move |c, _| {
			if directional_shadow_enabled {
				log::debug!(
					"Directional shadow pass executing: cascades={}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
					SHADOW_CASCADE_COUNT,
					instances.len(),
					drawable_instances,
					meshlet_count,
					mesh_dispatch.workgroup_count(),
				);
				c.start_region(|label| label.write_str("Directional Shadow Map"));
				let attachments = [ghi::AttachmentInformation::new(
					directional_shadow_map,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layers(SHADOW_CASCADE_COUNT as u32)];
				let c = c.start_render_pass(directional_extent, &attachments);
				let c = c.bind_raster_pipeline(directional_pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				for view_index in directional_shadow_view_indices(mesh_dispatch) {
					c.start_region(|label| label.write_str("Cascade"));
					c.write_push_constant(0, mesh_dispatch.work_item_base());
					c.write_push_constant(4, view_index);
					c.write_push_constant(8, view_index - 1);
					c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
					c.end_region();
				}
				c.end_render_pass();
				c.end_region();

				// Each SIMD-width workgroup reduces two adjacent source tiles into 4x4 cells.
				c.start_region(|label| label.write_str("Directional Shadow Depth Pyramid"));
				let c = c.bind_compute_pipeline(directional_shadow_depth_pyramid_pipeline);
				c.bind_descriptor_sets(&[directional_shadow_depth_pyramid_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(
					directional_shadow_depth_pyramid_extent,
					Extent::new(8, 4, 1),
				));
				c.end_region();
			}

			if cone_shadow_count > 0 {
				log::debug!(
					"Cone shadow pass executing: lights={}, active_primitives={}, drawable_primitives={}, meshlets={}, task_workgroups={}",
					cone_shadow_count,
					instances.len(),
					drawable_instances,
					meshlet_count,
					mesh_dispatch.workgroup_count(),
				);
				c.start_region(|label| label.write_str("Cone Shadow Map"));
				let attachments = [ghi::AttachmentInformation::new(
					cone_shadow_map,
					ghi::Layouts::RenderTarget,
					ghi::ClearValue::Depth(0.0),
					false,
					true,
				)
				.layers(MAX_CONE_SHADOWS as u32)];
				let c = c.start_render_pass(cone_extent, &attachments);
				let c = c.bind_raster_pipeline(cone_pipeline);
				c.bind_descriptor_sets(&[descriptor_set]);
				for (view_index, layer) in cone_shadow_view_indices(mesh_dispatch, cone_shadow_count) {
					c.start_region(|label| label.write_str("Cone"));
					c.write_push_constant(0, mesh_dispatch.work_item_base());
					c.write_push_constant(4, view_index);
					c.write_push_constant(8, layer);
					c.dispatch_meshes(mesh_dispatch.workgroup_count(), 1, 1);
					c.end_region();
				}
				c.end_render_pass();
				c.end_region();
			}
		}
	}
}

pub struct MaterialCountPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	pipeline: ghi::PipelineHandle,
}

impl MaterialCountPass {
	fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	) -> Self {
		let material_count_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/material-count.besl",
			"Material Count Pass Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let material_count_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&material_count_shader, ghi::ShaderTypes::Compute),
			)
			.name("Material Count Pass Compute Shader"),
		);

		MaterialCountPass {
			descriptor_set,
			material_count_buffer,
			visibility_pass_descriptor_set,
			pipeline: material_count_pipeline,
		}
	}

	fn prepare(&self, sink: &Sink) -> impl RenderPassFunction + use<'_> {
		let descriptor_set = self.descriptor_set;
		let visibility_pass_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.pipeline;
		let material_count_buffer = self.material_count_buffer;

		let extent = sink.extent();

		move |c, _| {
			log::debug!(
				"Visibility material count pass executing: extent={}x{}",
				extent.width(),
				extent.height()
			);
			c.start_region(|label| label.write_str("Material Count"));

			// The offset pass reads these counts without resetting them, so clear before every dispatch.
			c.clear_buffers(&[material_count_buffer.into()]);

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_pass_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(8)));

			c.end_region();
		}
	}

	fn get_material_count_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_count_buffer.into()
	}
}

pub struct MaterialOffsetPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
	material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
	material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	material_offset_pipeline: ghi::PipelineHandle,
}

impl MaterialOffsetPass {
	fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_pass_descriptor_set: ghi::DescriptorSetHandle,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		let material_offset_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/material-offset.besl",
			"Material Offset Pass Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let material_offset_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&material_offset_shader, ghi::ShaderTypes::Compute),
			)
			.name("Material Offset Pass Compute Shader"),
		);

		MaterialOffsetPass {
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
			descriptor_set,
			visibility_pass_descriptor_set,
			material_offset_pipeline,
		}
	}

	fn prepare(&self) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let visibility_passes_descriptor_set = self.visibility_pass_descriptor_set;
		let pipeline = self.material_offset_pipeline;

		move |c, _| {
			log::debug!("Visibility material offset pass executing");
			c.start_region(|label| label.write_str("Material Offset"));

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(Extent::line(1), Extent::line(1)));
			c.end_region();
		}
	}

	fn get_material_offset_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_buffer.into()
	}

	fn get_material_offset_scratch_buffer(&self) -> ghi::BaseBufferHandle {
		self.material_offset_scratch_buffer.into()
	}
}

pub struct PixelMappingPass {
	descriptor_set: ghi::DescriptorSetHandle,
	visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	pixel_mapping_pipeline: ghi::PipelineHandle,
}

impl PixelMappingPass {
	fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		descriptor_set: ghi::DescriptorSetHandle,
		visibility_passes_descriptor_set: ghi::DescriptorSetHandle,
	) -> Self {
		let pixel_mapping_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/pixel-mapping.besl",
			"Pixel Mapping Pass Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let pixel_mapping_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&pixel_mapping_shader, ghi::ShaderTypes::Compute),
			)
			.name("Pixel Mapping Pass Compute Shader"),
		);

		PixelMappingPass {
			descriptor_set,
			visibility_passes_descriptor_set,
			pixel_mapping_pipeline,
		}
	}

	pub(super) fn prepare(&self, sink: &Sink) -> impl RenderPassFunction {
		let descriptor_set = self.descriptor_set;
		let pipeline = self.pixel_mapping_pipeline;
		let visibility_passes_descriptor_set = self.visibility_passes_descriptor_set;

		let extent = sink.extent();

		move |c, _| {
			log::debug!(
				"Visibility pixel mapping pass executing: extent={}x{}",
				extent.width(),
				extent.height()
			);
			c.start_region(|label| label.write_str("Pixel Mapping"));

			let compute_pipeline_command = c.bind_compute_pipeline(pipeline);
			compute_pipeline_command.bind_descriptor_sets(&[descriptor_set, visibility_passes_descriptor_set]);
			compute_pipeline_command.dispatch(ghi::DispatchExtent::new(extent, Extent::square(16)));

			c.end_region();
		}
	}
}

/// The `GtaoPass` struct builds a depth-based ambient occlusion term before material evaluation shades the frame.
pub struct GtaoPass {
	settings: GtaoSettings,
	gtao_descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	blur_descriptor_set_x: ghi::DescriptorSetHandle,
	upscale_descriptor_set: ghi::DescriptorSetHandle,
	gtao_pipeline: ghi::PipelineHandle,
	depth_pyramid_pipeline: ghi::PipelineHandle,
	blur_pipeline_x: ghi::PipelineHandle,
	upscale_pipeline: ghi::PipelineHandle,
	ao_map: ghi::BaseImageHandle,
	view_data: ghi::DynamicBufferHandle<FastGtaoViewData>,
	gtao_parameters: ghi::DynamicBufferHandle<GtaoShaderParameters>,
	depth_pyramid: ghi::DynamicImageHandle,
	raw_ao_map: ghi::DynamicImageHandle,
	temp_ao_map: ghi::DynamicImageHandle,
}

impl GtaoPass {
	fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		depth: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		settings: GtaoSettings,
	) -> Self {
		let gtao_descriptor_set = context.create_descriptor_set(Some("GTAO Descriptor Set"));
		let depth_pyramid_descriptor_set = context.create_descriptor_set(Some("GTAO Depth Pyramid Descriptor Set"));
		let blur_descriptor_set_x = context.create_descriptor_set(Some("GTAO Blur X Descriptor Set"));
		let upscale_descriptor_set = context.create_descriptor_set(Some("GTAO Depth-Aware Upscale Descriptor Set"));
		let view_data = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("GTAO View Data")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let gtao_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("GTAO Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// Metal applies min/max reduction only when every sampler filter is linear.
		// Centered samples then conservatively collapse each reversed-depth 2x2 footprint.
		let depth_pyramid_source_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
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
				.max_lod((GTAO_DEPTH_PYRAMID_MIP_COUNT - 1) as f32),
		);
		let ao_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Border {})
				.min_lod(0f32)
				.max_lod(0f32),
		);
		let temp_ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Half-Resolution Blur Intermediate")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let raw_ao_map = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Half-Resolution Raw")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		// Mip zero retains full sink resolution so levels one through three match the depth reductions.
		// The initial 8x8 allocation keeps all declared mips valid before the first sink resize.
		let depth_pyramid = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
				.name("GTAO Depth Pyramid")
				.extent(Extent::square(8))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(GTAO_DEPTH_PYRAMID_MIP_COUNT),
		);
		context.write(&[
			ghi::DescriptorWrite::buffer(depth_pyramid_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				depth,
				depth_pyramid_source_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_1_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				1,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_2_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				2,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				GTAO_DEPTH_PYRAMID_OUTPUT_3_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				3,
			),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::buffer(gtao_descriptor_set, GTAO_PARAMETERS_BINDING.slot(), gtao_parameters.into()),
			ghi::DescriptorWrite::image(
				gtao_descriptor_set,
				GTAO_OUTPUT_BINDING.slot(),
				raw_ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				gtao_descriptor_set,
				GTAO_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_descriptor_set_x,
				GTAO_BLUR_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				blur_descriptor_set_x,
				GTAO_BLUR_SOURCE_BINDING.slot(),
				raw_ao_map,
				ao_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				blur_descriptor_set_x,
				GTAO_BLUR_OUTPUT_BINDING.slot(),
				temp_ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::buffer(upscale_descriptor_set, GTAO_VIEW_BINDING.slot(), view_data.into()),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_BLUR_DEPTH_BINDING.slot(),
				depth,
				depth_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_BLUR_SOURCE_BINDING.slot(),
				temp_ao_map,
				ao_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image(
				upscale_descriptor_set,
				GTAO_BLUR_OUTPUT_BINDING.slot(),
				ao_map,
				ghi::Layouts::General,
			),
			ghi::DescriptorWrite::combined_image_sampler(
				upscale_descriptor_set,
				GTAO_UPSCALE_LOW_RESOLUTION_DEPTH_BINDING.slot(),
				depth_pyramid,
				depth_sampler,
				ghi::Layouts::Read,
			),
		]);
		let depth_pyramid_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/gtao-depth-pyramid.besl",
			"GTAO Depth Pyramid Compute Shader",
			ResourceShaderTypes::Compute,
		);
		let gtao_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/gtao.besl",
			"GTAO Pass Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let depth_pyramid_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(
				&[],
				ghi::ShaderParameter::new(&depth_pyramid_shader, ghi::ShaderTypes::Compute),
			)
			.name("GTAO Depth Pyramid Compute Shader"),
		);
		let gtao_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(&[], ghi::ShaderParameter::new(&gtao_shader, ghi::ShaderTypes::Compute))
				.name("GTAO Pass Compute Shader"),
		);

		let blur_x_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/gtao-blur-x.besl",
			"GTAO Blur X Compute Shader",
			ResourceShaderTypes::Compute,
		);
		let upscale_shader = load_visibility_shader(
			context,
			shader_resources,
			"byte-engine/rendering/visibility/gtao-upscale.besl",
			"GTAO Depth-Aware Upscale Compute Shader",
			ResourceShaderTypes::Compute,
		);

		let blur_pipeline_x = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(&[], ghi::ShaderParameter::new(&blur_x_shader, ghi::ShaderTypes::Compute))
				.name("GTAO Blur X Compute Shader"),
		);
		let upscale_pipeline = context.create_compute_pipeline(
			ghi::pipelines::compute::Builder::new(&[], ghi::ShaderParameter::new(&upscale_shader, ghi::ShaderTypes::Compute))
				.name("GTAO Depth-Aware Upscale Compute Shader"),
		);

		Self {
			settings,
			gtao_descriptor_set,
			depth_pyramid_descriptor_set,
			blur_descriptor_set_x,
			upscale_descriptor_set,
			gtao_pipeline,
			depth_pyramid_pipeline,
			blur_pipeline_x,
			upscale_pipeline,
			ao_map,
			view_data,
			gtao_parameters,
			depth_pyramid,
			raw_ao_map,
			temp_ao_map,
		}
	}

	fn set_settings(&mut self, settings: GtaoSettings) {
		self.settings = settings;
	}

	fn prepare(&self, frame: &mut ghi::implementation::Frame, sink: &Sink) -> impl RenderPassFunction {
		let gtao_descriptor_set = self.gtao_descriptor_set;
		let depth_pyramid_descriptor_set = self.depth_pyramid_descriptor_set;
		let blur_descriptor_set_x = self.blur_descriptor_set_x;
		let upscale_descriptor_set = self.upscale_descriptor_set;
		let gtao_pipeline = self.gtao_pipeline;
		let depth_pyramid_pipeline = self.depth_pyramid_pipeline;
		let blur_pipeline_x = self.blur_pipeline_x;
		let upscale_pipeline = self.upscale_pipeline;
		let ao_map = self.ao_map;
		let view_data = self.view_data;
		let gtao_parameters = self.gtao_parameters;
		let depth_pyramid = self.depth_pyramid;
		let raw_ao_map = self.raw_ao_map;
		let temp_ao_map = self.temp_ao_map;
		let extent = sink.extent();
		let gtao_extent = gtao_half_resolution_extent(extent);

		*frame.get_mut_dynamic_buffer_slice(view_data) = fast_gtao_view_data(sink, gtao_extent);
		frame.sync_buffer(view_data);
		*frame.get_mut_dynamic_buffer_slice(gtao_parameters) = self.settings.shader_parameters();
		frame.sync_buffer(gtao_parameters);
		frame.resize_image(ao_map, extent);
		frame.resize_image(raw_ao_map.into(), gtao_extent);
		frame.resize_image(temp_ao_map.into(), gtao_extent);
		frame.resize_image(depth_pyramid.into(), extent);

		move |c, _| {
			c.start_region(|label| label.write_str("GTAO"));

			c.start_region(|label| label.write_str("GTAO Depth Pyramid"));
			let c = c.bind_compute_pipeline(depth_pyramid_pipeline);
			c.bind_descriptor_sets(&[depth_pyramid_descriptor_set]);
			c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(8, 4, 1)));
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Evaluate"));
			{
				let c = c.bind_compute_pipeline(gtao_pipeline);
				c.bind_descriptor_sets(&[gtao_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(16, 8, 1)));
			}
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Denoise"));
			c.start_region(|label| label.write_str("GTAO Denoise Horizontal"));
			{
				let c = c.bind_compute_pipeline(blur_pipeline_x);
				c.bind_descriptor_sets(&[blur_descriptor_set_x]);
				c.dispatch(ghi::DispatchExtent::new(gtao_extent, Extent::new(8, 8, 1)));
			}
			c.end_region();

			c.start_region(|label| label.write_str("GTAO Denoise and Depth-Aware Upscale"));
			{
				let c = c.bind_compute_pipeline(upscale_pipeline);
				c.bind_descriptor_sets(&[upscale_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			}
			c.end_region();
			c.end_region();

			c.end_region();
		}
	}
}

/// Returns the nonzero extent of the first physical mip in the GTAO depth pyramid.
fn gtao_half_resolution_extent(extent: Extent) -> Extent {
	Extent::rectangle((extent.width() / 2).max(1), (extent.height() / 2).max(1))
}

/// Returns whether this frame contains geometry that uses one material in the requested visibility phase.
fn material_is_active(active_materials: &ActiveMaterialMask, material_index: u32) -> bool {
	let material_index = material_index as usize;
	active_materials
		.get(material_index / u64::BITS as usize)
		.is_some_and(|word| word & (1u64 << (material_index % u64::BITS as usize)) != 0)
}

/// The `MaterialEvaluationPass` struct owns material dispatch state shared by opaque writes and transparent composition.
pub struct MaterialEvaluationPass {
	lit: ghi::BaseImageHandle,
	ao_map: ghi::BaseImageHandle,
	/// Base descriptor set shared by material layouts.
	base_descriptor_set: ghi::DescriptorSetHandle,
	/// Visibility passes descriptor set
	visibility_descriptor_set: ghi::DescriptorSetHandle,
	/// Material evaluation descriptor set
	descriptor_set: ghi::DescriptorSetHandle,
	material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
}

impl MaterialEvaluationPass {
	fn new(
		lit: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		_directional_shadow_map: ghi::BaseImageHandle,
		_cone_shadow_map: ghi::BaseImageHandle,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
	) -> Self {
		MaterialEvaluationPass {
			lit,
			ao_map,
			base_descriptor_set,
			visibility_descriptor_set,
			descriptor_set,
			material_evaluation_dispatches,
		}
	}

	/// Prepares one material phase with explicit overwrite or source-over behavior.
	fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		materials: &'a [(String, u32, ghi::PipelineHandle)],
		active_materials: &'a ActiveMaterialMask,
		phase: VisibilityPhase,
	) -> impl RenderPassFunction + 'a {
		let lit = self.lit;
		let ao_map = self.ao_map;
		let base_descriptor_set = self.base_descriptor_set;
		let material_evaluation_dispatches = self.material_evaluation_dispatches;
		let visibility_descriptor_set = self.visibility_descriptor_set;
		let material_evaluation_descriptor_set = self.descriptor_set;
		let extent = sink.extent();
		let active_material_count = materials
			.iter()
			.filter(|(_, index, _)| material_is_active(active_materials, *index))
			.count();

		if phase == VisibilityPhase::Opaque {
			frame.resize_image(ao_map, extent);
		}

		move |c, t| {
			if phase == VisibilityPhase::Opaque {
				c.clear_images(&[(lit, ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0)))]);
			}
			if active_material_count == 0 {
				return;
			}
			log::debug!(
				"{} visibility material evaluation executing: extent={}x{}, materials={}",
				phase.label(),
				extent.width(),
				extent.height(),
				active_material_count,
			);

			c.start_region(|label| label.write_str("Material Evaluation"));
			c.start_region(|label| label.write_str(phase.label()));

			let mut bound_material_pipeline = None;
			for (name, index, pipeline) in materials {
				if !material_is_active(active_materials, *index) {
					continue;
				}
				c.start_region(|label| label.write_str(name));
				if bound_material_pipeline != Some(*pipeline) {
					let c = c.bind_compute_pipeline(*pipeline);
					c.bind_descriptor_sets(&[
						base_descriptor_set,
						visibility_descriptor_set,
						material_evaluation_descriptor_set,
					]);
					bound_material_pipeline = Some(*pipeline);
				}
				c.write_push_constant(0, [*index, phase.blend_flag()]);
				c.indirect_dispatch(material_evaluation_dispatches, *index as usize);
				c.end_region();
			}

			c.end_region();
			c.end_region();
		}
	}
}

/// The `VisibilityPipelineRenderPass` struct sequences visibility-buffer work for one sink and scene frame.
pub(crate) struct VisibilityPipelineRenderPass {
	shadow_pass: ShadowPass,
	visibility_pass: VisibilityPass,
	material_count_pass: MaterialCountPass,
	material_offset_pass: MaterialOffsetPass,
	pixel_mapping_pass: PixelMappingPass,
	gtao_pass: GtaoPass,
	material_evaluation_pass: MaterialEvaluationPass,
}

impl VisibilityPipelineRenderPass {
	pub(crate) fn set_gtao_settings(&mut self, settings: GtaoSettings) {
		self.gtao_pass.set_settings(settings);
	}

	/// Returns the descriptor set that carries material-evaluation-only resources.
	pub(super) fn material_evaluation_descriptor_set(&self) -> ghi::DescriptorSetHandle {
		self.material_evaluation_pass.descriptor_set
	}

	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		shader_resources: &ResourceManager,
		base_descriptor_set: ghi::DescriptorSetHandle,
		visibility_descriptor_set: ghi::DescriptorSetHandle,
		material_evaluation_descriptor_set: ghi::DescriptorSetHandle,
		material_count_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		lit: ghi::BaseImageHandle,
		ao_map: ghi::BaseImageHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		directional_shadow_depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
		depth: ghi::BaseImageHandle,
		primitive_index: ghi::BaseImageHandle,
		instance_id: ghi::BaseImageHandle,
		material_offset_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_offset_scratch_buffer: ghi::BufferHandle<[u32; MAX_MATERIALS]>,
		material_evaluation_dispatches: ghi::BufferHandle<[[u32; 3]; MAX_MATERIALS]>,
		gtao_settings: GtaoSettings,
	) -> Self {
		let shadow_pass = ShadowPass::new(
			context,
			shader_resources,
			base_descriptor_set,
			directional_shadow_map,
			directional_shadow_depth_pyramid,
			cone_shadow_map,
		);
		let visibility_pass = VisibilityPass::new(
			context,
			shader_resources,
			base_descriptor_set,
			primitive_index,
			instance_id,
			depth,
		);
		let material_count_pass = MaterialCountPass::new(
			context,
			shader_resources,
			base_descriptor_set,
			visibility_descriptor_set,
			material_count_buffer,
		);
		let material_offset_pass = MaterialOffsetPass::new(
			context,
			shader_resources,
			base_descriptor_set,
			visibility_descriptor_set,
			material_offset_buffer,
			material_offset_scratch_buffer,
			material_evaluation_dispatches,
		);
		let pixel_mapping_pass =
			PixelMappingPass::new(context, shader_resources, base_descriptor_set, visibility_descriptor_set);
		let gtao_pass = GtaoPass::new(context, shader_resources, depth, ao_map, gtao_settings);

		let material_evaluation_dispatches = material_offset_pass.material_evaluation_dispatches;

		let material_evaluation_pass = MaterialEvaluationPass::new(
			lit,
			ao_map,
			directional_shadow_map,
			cone_shadow_map,
			base_descriptor_set,
			visibility_descriptor_set,
			material_evaluation_descriptor_set,
			material_evaluation_dispatches,
		);

		Self {
			shadow_pass,
			visibility_pass,
			material_count_pass,
			material_offset_pass,
			pixel_mapping_pass,
			gtao_pass,
			material_evaluation_pass,
		}
	}

	/// Prepares one opaque visibility layer and one nearest-surface transparent layer.
	pub(super) fn prepare<'a>(
		&'a self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		skinning_pass: Option<&'a SkinningPass>,
		opaque_mesh_dispatch: MeshDispatch,
		transparent_mesh_dispatch: MeshDispatch,
		skinning_dispatches: &'a [SkinningDispatch],
		opaque_instances: &'a [Instance],
		transparent_instances: &'a [Instance],
		opaque_materials: &'a [(String, u32, ghi::PipelineHandle)],
		transparent_materials: &'a [(String, u32, ghi::PipelineHandle)],
		opaque_material_mask: &'a ActiveMaterialMask,
		transparent_material_mask: &'a ActiveMaterialMask,
		directional_shadow_enabled: bool,
		cone_shadow_count: usize,
	) -> impl RenderPassFunction + 'a {
		// Blend materials have no alpha-aware shadow shader, so only opaque-phase primitives populate the depth map.
		let shadow_pass = self.shadow_pass.prepare(
			frame,
			opaque_instances,
			opaque_mesh_dispatch,
			directional_shadow_enabled,
			cone_shadow_count,
		);
		let visibility_pass = &self.visibility_pass;
		// The offset pass consumes and resets every counter before the optional transparent layer runs.
		let opaque_material_count_pass = self.material_count_pass.prepare(sink);
		let transparent_material_count_pass = self.material_count_pass.prepare(sink);
		let material_offset_pass = self.material_offset_pass.prepare();
		let pixel_mapping_pass = self.pixel_mapping_pass.prepare(sink);
		let gtao_pass = self.gtao_pass.prepare(frame, sink);
		let opaque_material_evaluation_pass =
			self.material_evaluation_pass
				.prepare(frame, sink, opaque_materials, opaque_material_mask, VisibilityPhase::Opaque);
		let transparent_material_evaluation_pass = self.material_evaluation_pass.prepare(
			frame,
			sink,
			transparent_materials,
			transparent_material_mask,
			VisibilityPhase::Transparent,
		);
		let extent = sink.extent();
		let instance_count = opaque_instances.len() + transparent_instances.len();
		let meshlet_count = opaque_instances
			.iter()
			.chain(transparent_instances)
			.map(|instance| instance.meshlet_count)
			.sum::<u32>();
		let opaque_count = opaque_materials
			.iter()
			.filter(|(_, index, _)| material_is_active(opaque_material_mask, *index))
			.count();
		let transparent_count = transparent_materials
			.iter()
			.filter(|(_, index, _)| material_is_active(transparent_material_mask, *index))
			.count();
		move |c, t| {
			log::debug!(
				"Visibility render model executing: primitives={}, opaque_primitives={}, transparent_primitives={}, meshlets={}, opaque_materials={}, transparent_materials={}, shadow_enabled={}",
				instance_count,
				opaque_instances.len(),
				transparent_instances.len(),
				meshlet_count,
				opaque_count,
				transparent_count,
				directional_shadow_enabled || cone_shadow_count > 0,
			);
			c.start_region(|label| label.write_str("Visibility Render Model"));

			if let Some(skinning_pass) = skinning_pass {
				skinning_pass.record(c, skinning_dispatches);
			}
			shadow_pass(c, t);

			// The opaque layer establishes the depth and color retained by every later transparent primitive.
			visibility_pass.record(c, extent, opaque_instances, opaque_mesh_dispatch, VisibilityPhase::Opaque);
			opaque_material_count_pass(c, t);
			material_offset_pass(c, t);
			pixel_mapping_pass(c, t);
			gtao_pass(c, t);
			opaque_material_evaluation_pass(c, t);

			// The visibility buffer represents one transparent layer. Resolve every blend primitive
			// together so normal depth testing selects the nearest surface before source-over evaluation.
			if let Some(transparent_layer) = transparent_visibility_layer(transparent_instances) {
				visibility_pass.record(
					c,
					extent,
					transparent_layer,
					transparent_mesh_dispatch,
					VisibilityPhase::Transparent,
				);
				transparent_material_count_pass(c, t);
				material_offset_pass(c, t);
				pixel_mapping_pass(c, t);
				transparent_material_evaluation_pass(c, t);
			}

			c.end_region();
		}
	}
}

#[cfg(test)]
mod tests {
	use math::{inverse, Point, UnitVector};
	use maths_rs::{cross, dot, length, Vec3f, Vec4f};
	use utils::Extent;

	use super::{
		cone_shadow_view_indices, directional_shadow_view_indices, fast_gtao_view_data, gtao_half_resolution_extent,
		transparent_visibility_layer, GtaoSettings, Instance, MeshDispatch,
	};
	use crate::configuration::ConfigurationValue;
	use crate::rendering::{view::View, Sink};

	#[test]
	fn shadow_dispatches_preserve_directional_cascades_and_packed_cone_layers() {
		let dispatch = MeshDispatch::with_workgroup_count(19);

		assert_eq!(directional_shadow_view_indices(dispatch).collect::<Vec<_>>(), [1, 2, 3, 4]);
		assert_eq!(
			cone_shadow_view_indices(dispatch, 4).collect::<Vec<_>>(),
			[(5, 0), (6, 1), (7, 2), (8, 3)]
		);
		assert_eq!(directional_shadow_view_indices(MeshDispatch::default()).count(), 0);
		assert_eq!(cone_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
	}

	#[test]
	fn gtao_runtime_parameters_update_quality_controls_without_partial_state() {
		let defaults = GtaoSettings::default();
		let (settings, radius) = defaults
			.with_parameter("radius", &ConfigurationValue::Text("2.5".to_string()))
			.expect("radius should parse");
		let (settings, samples) = settings
			.with_parameter("samples-per-ray", &ConfigurationValue::Integer(12))
			.expect("sample count should parse");
		let (settings, rays) = settings
			.with_parameter("radial-rays", &ConfigurationValue::Integer(16))
			.expect("ray count should parse");

		assert_eq!(settings.radius, 2.5);
		assert_eq!(settings.samples_per_ray, 12);
		assert_eq!(settings.radial_rays, 16);
		assert_eq!(radius, ConfigurationValue::Float(2.5));
		assert_eq!(samples, ConfigurationValue::Integer(12));
		assert_eq!(rays, ConfigurationValue::Integer(16));

		assert!(settings
			.with_parameter("radial-rays", &ConfigurationValue::Integer(7))
			.is_err());
		assert_eq!(settings.radial_rays, 16);
	}

	#[test]
	fn transparent_visibility_uses_one_depth_resolved_layer() {
		let instances = [
			Instance {
				shader_mesh_index: 3,
				meshlet_count: 2,
			},
			Instance {
				shader_mesh_index: 8,
				meshlet_count: 5,
			},
		];

		let layer = transparent_visibility_layer(&instances).expect("Non-empty transparent work must produce one layer");

		assert_eq!(layer, instances);
		assert!(transparent_visibility_layer(&[]).is_none());
		assert!(transparent_visibility_layer(&[Instance {
			shader_mesh_index: 13,
			meshlet_count: 0,
		}])
		.is_none());
	}

	#[test]
	fn fast_gtao_view_reconstructs_pixel_rays_and_reversed_depth() {
		let extent = Extent::rectangle(1920, 1080);
		let view = View::new_perspective(
			60.0,
			extent.width() as f32 / extent.height() as f32,
			0.1,
			100.0,
			Point::origin(),
			UnitVector::z_axis(),
		);
		let sink = Sink::new(view, extent, 0);
		let gtao_extent = gtao_half_resolution_extent(extent);
		let constants = fast_gtao_view_data(&sink, gtao_extent);
		let projection = view.projection();
		assert_eq!(std::mem::size_of_val(&constants), 32);

		for z in [0.1f32, 0.5, 1.0, 10.0, 100.0] {
			let clip = projection * Vec4f::new(0.0, 0.0, z, 1.0);
			let depth = clip.z / clip.w;
			let reconstructed = constants.depth_unproject_numerator / (depth + constants.depth_unproject_denominator_offset);
			assert!(
				(reconstructed - z).abs() <= z.max(1.0) * 0.00001,
				"Unexpected GTAO depth reconstruction for z={z}: {reconstructed}"
			);
		}

		for pixel in [[0.0f32, 0.0], [479.0, 269.0], [959.0, 539.0]] {
			let ray = [
				pixel[0] * constants.pixel_to_ray_mul[0] + constants.pixel_to_ray_add[0],
				pixel[1] * constants.pixel_to_ray_mul[1] + constants.pixel_to_ray_add[1],
			];
			let ndc = [
				2.0 * (pixel[0] + 0.5) / gtao_extent.width() as f32 - 1.0,
				1.0 - 2.0 * (pixel[1] + 0.5) / gtao_extent.height() as f32,
			];
			assert!((ray[0] - ndc[0] / projection[0]).abs() < 0.000001);
			assert!((ray[1] - ndc[1] / projection[5]).abs() < 0.000001);
		}
		assert_eq!(constants.view_z_sign, 1.0);
		assert_eq!(gtao_extent, Extent::rectangle(960, 540));
		assert_eq!(
			gtao_half_resolution_extent(Extent::rectangle(1919, 1079)),
			Extent::rectangle(959, 539)
		);
		assert_eq!(gtao_half_resolution_extent(Extent::square(1)), Extent::square(1));
	}

	#[test]
	fn gtao_view_space_reconstruction_z_is_positive() {
		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = inverse(proj);

		// Simulate what the GTAO shader does: reconstruct positions for center + neighbors
		// at various depths, compute the normal, and check its direction

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vec3f {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vec4f::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			let w = view.w;
			Vec3f::new(view.x / w, view.y / w, view.z / w)
		};

		// Project a known view-space point to get its depth
		let project_to_depth = |vx: f32, vy: f32, vz: f32| -> f32 {
			let clip = proj * Vec4f::new(vx, vy, vz, 1.0);
			clip.z / clip.w // ndc depth
		};

		// Test at different distances
		for vz in [0.5f32, 1.0, 2.0, 5.0, 10.0, 20.0, 50.0] {
			let depth = project_to_depth(0.0, 0.0, vz);
			let center_px = extent_x / 2;
			let center_py = extent_y / 2;

			let center = reconstruct(center_px, center_py, depth);
			let right = reconstruct(center_px + 1, center_py, depth);
			let left = reconstruct(center_px - 1, center_py, depth);
			let top = reconstruct(center_px, center_py - 1, depth);
			let bottom = reconstruct(center_px, center_py + 1, depth);

			// min_diff for horizontal: pick shorter of (right - center) or (center - left)
			let ap_h = Vec3f::new(right.x - center.x, right.y - center.y, right.z - center.z);
			let bp_h = Vec3f::new(center.x - left.x, center.y - left.y, center.z - left.z);
			let h_diff = if dot(ap_h, ap_h) < dot(bp_h, bp_h) { ap_h } else { bp_h };

			// min_diff for vertical: pick shorter of (top - center) or (center - bottom)
			let ap_v = Vec3f::new(top.x - center.x, top.y - center.y, top.z - center.z);
			let bp_v = Vec3f::new(center.x - bottom.x, center.y - bottom.y, center.z - bottom.z);
			let v_diff = if dot(ap_v, ap_v) < dot(bp_v, bp_v) { ap_v } else { bp_v };

			let normal = cross(h_diff, v_diff);
			let normal_len = length(normal);
			let normal = if normal_len > 1e-8 {
				Vec3f::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vec3f::new(0.0, 0.0, 1.0)
			};

			// The shader enforces camera-facing: if dot(normal, center_position) > 0, flip.
			// In view space the camera is at origin, so center_position IS the view direction to the point.
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vec3f::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"vz={:.1}: center=({:.4},{:.4},{:.4}), normal=({:.4},{:.4},{:.4}), depth={:.6}",
				vz, center.x, center.y, center.z, normal.x, normal.y, normal.z, depth
			);

			// The normal must face toward the camera, i.e. dot(normal, center_position) <= 0.
			// For a flat surface perpendicular to Z: normal.z should be dominant and negative.
			let dot_check = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			assert!(
				dot_check <= 0.0,
				"Normal should face camera (dot(normal, center_position) <= 0) at vz={}, got dot={}",
				vz,
				dot_check
			);
			assert!(
				normal.z.abs() > 0.99,
				"Normal Z should be dominant for flat surface perpendicular to Z at vz={}, got normal.z={}",
				vz,
				normal.z
			);
		}
	}

	/// Simulates the GTAO normal reconstruction on a floor plane (Y=constant)
	/// where depth varies per pixel, and checks for normal sign flips at different distances.
	#[test]
	fn gtao_normal_on_floor_plane() {
		let near = 0.1f32;
		let far = 100.0f32;
		let fov = 45.0f32;
		let aspect = 16.0 / 9.0;
		let extent_x = 1920i32;
		let extent_y = 1080i32;

		let proj = math::projection_matrix(fov, aspect, near, far);
		let inv_proj = inverse(proj);

		let reconstruct = |px: i32, py: i32, depth: f32| -> Vec3f {
			let uv_x = (px as f32 + 0.5) / extent_x as f32;
			let uv_y = (py as f32 + 0.5) / extent_y as f32;
			let ndc_x = uv_x * 2.0 - 1.0;
			let ndc_y = 1.0 - uv_y * 2.0;
			let clip = Vec4f::new(ndc_x, ndc_y, depth, 1.0);
			let view = inv_proj * clip;
			Vec3f::new(view.x / view.w, view.y / view.w, view.z / view.w)
		};

		let project = |vx: f32, vy: f32, vz: f32| -> (f32, f32, f32) {
			let clip = proj * Vec4f::new(vx, vy, vz, 1.0);
			let ndc_x = clip.x / clip.w;
			let ndc_y = clip.y / clip.w;
			let depth = clip.z / clip.w;
			// Inverse of: ndc_x = uv_x * 2 - 1, ndc_y = 1 - uv_y * 2
			let uv_x = (ndc_x + 1.0) / 2.0;
			let uv_y = (1.0 - ndc_y) / 2.0;
			let px = uv_x * extent_x as f32 - 0.5;
			let py = uv_y * extent_y as f32 - 0.5;
			(px, py, depth)
		};

		// Floor plane at Y = -1 (camera looks along +Z, floor is below camera)
		// For a given pixel, we need to find where the ray through that pixel hits Y=-1
		let floor_y = -1.0f32;

		// For a pixel (px, py), reconstruct a ray direction in view space:
		// The ray goes from origin through the point at depth=1 (arbitrary)
		let ray_hit_floor = |px: i32, py: i32| -> Option<(f32, f32)> {
			// Reconstruct view-space direction using depth=0.5 (arbitrary non-zero)
			let p = reconstruct(px, py, 0.5);
			// Ray: origin=(0,0,0), direction=p (normalized doesn't matter, just need ratio)
			// Hit Y=floor_y: t = floor_y / p.y
			if p.y.abs() < 1e-8 {
				return None;
			} // ray parallel to floor
			let t = floor_y / p.y;
			if t <= 0.0 {
				return None;
			} // floor behind camera
			let hit_z = p.z * t;
			if hit_z < near || hit_z > far {
				return None;
			} // outside clip range
	 // Project hit point to get depth
			let hit_x = p.x * t;
			let clip = proj * Vec4f::new(hit_x, floor_y, hit_z, 1.0);
			Some((hit_z, clip.z / clip.w))
		};

		let min_diff = |p: Vec3f, a: Vec3f, b: Vec3f| -> Vec3f {
			let ap = Vec3f::new(a.x - p.x, a.y - p.y, a.z - p.z);
			let bp = Vec3f::new(p.x - b.x, p.y - b.y, p.z - b.z);
			if dot(ap, ap) < dot(bp, bp) {
				ap
			} else {
				bp
			}
		};

		eprintln!("\n--- Floor plane normal reconstruction ---");
		eprintln!("Testing at various screen Y positions (floor at Y={}):", floor_y);

		let mut found_flip = false;

		// Test across different screen rows (different distances to floor)
		for py in (extent_y / 2 + 50..extent_y - 10).step_by(50) {
			let px = extent_x / 2; // screen center X

			let Some((center_vz, center_depth)) = ray_hit_floor(px, py) else {
				continue;
			};
			let Some((_, left_depth)) = ray_hit_floor(px - 1, py) else {
				continue;
			};
			let Some((_, right_depth)) = ray_hit_floor(px + 1, py) else {
				continue;
			};
			let Some((_, top_depth)) = ray_hit_floor(px, py - 1) else {
				continue;
			};
			let Some((_, bottom_depth)) = ray_hit_floor(px, py + 1) else {
				continue;
			};

			let center = reconstruct(px, py, center_depth);
			let left = reconstruct(px - 1, py, left_depth);
			let right = reconstruct(px + 1, py, right_depth);
			let top = reconstruct(px, py - 1, top_depth);
			let bottom = reconstruct(px, py + 1, bottom_depth);

			let h_diff = min_diff(center, right, left);
			let v_diff = min_diff(center, top, bottom);

			let normal = cross(h_diff, v_diff);
			let normal_len = length(normal);
			let normal = if normal_len > 1e-8 {
				Vec3f::new(normal.x / normal_len, normal.y / normal_len, normal.z / normal_len)
			} else {
				Vec3f::new(0.0, 0.0, 1.0)
			};

			// Apply camera-facing check (same as shader)
			let dot_n_p = normal.x * center.x + normal.y * center.y + normal.z * center.z;
			let normal = if dot_n_p > 0.0 {
				Vec3f::new(-normal.x, -normal.y, -normal.z)
			} else {
				normal
			};

			eprintln!(
				"py={:4}, vz={:6.2}: h_diff=({:+.6},{:+.6},{:+.6}), v_diff=({:+.6},{:+.6},{:+.6}), normal=({:+.4},{:+.4},{:+.4})",
				py, center_vz, h_diff.x, h_diff.y, h_diff.z, v_diff.x, v_diff.y, v_diff.z, normal.x, normal.y, normal.z,
			);

			// For a floor plane at Y=-1, the normal should point +Y (up, toward camera if cam is above floor)
			if normal.y < 0.0 {
				found_flip = true;
				eprintln!("  ^^^ FLIPPED! Normal Y is negative (pointing into floor)");
			}
		}

		if found_flip {
			eprintln!("\nWARNING: Normal flipped at some distances! This explains the hard boundary.");
		} else {
			eprintln!("\nAll normals consistent (no flip detected in tested range).");
		}
	}
}
