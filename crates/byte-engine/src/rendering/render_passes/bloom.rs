use std::borrow::Borrow;

use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Viewport,
	},
};

use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
};
use resource_management::glsl;
use utils::{Box, Extent};

const EXTRACT_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const EXTRACT_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(1, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const EXTRACT_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

const UPSAMPLE_LOW_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const UPSAMPLE_HIGH_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const UPSAMPLE_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const UPSAMPLE_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

const COMPOSITE_SCENE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const COMPOSITE_BLOOM_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const COMPOSITE_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);
const COMPOSITE_PARAMETERS_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(3, ghi::descriptors::DescriptorType::StorageBuffer, ghi::Stages::COMPUTE);

const MAX_BLOOM_LEVELS: u32 = 6;

/// The `BloomPassSettings` struct defines the intent and shaping controls for a reusable HDR bloom stage.
#[derive(Clone, Copy, Debug)]
pub struct BloomPassSettings {
	pub enabled: bool,
	pub threshold: f32,
	pub soft_knee: f32,
	pub intensity: f32,
	pub radius: f32,
	pub levels: u32,
}

impl Default for BloomPassSettings {
	fn default() -> Self {
		Self {
			enabled: true,
			threshold: 1.0,
			soft_knee: 0.5,
			intensity: 0.08,
			radius: 1.0,
			levels: 5,
		}
	}
}

impl BloomPassSettings {
	fn resolved_level_count(self) -> usize {
		self.levels.clamp(1, MAX_BLOOM_LEVELS) as usize
	}
}

#[repr(C)]
#[derive(Clone, Copy)]
struct BloomShaderData {
	prefilter: [f32; 4],
	filter: [f32; 4],
}

/// The `BloomPass` struct creates a reusable pre-tonemap glow stage that can feed later post-processing.
pub struct BloomPass {
	settings: BloomPassSettings,
	parameters: ghi::DynamicBufferHandle<BloomShaderData>,
	extract_pipeline: ghi::PipelineHandle,
	downsample_pipeline: ghi::PipelineHandle,
	upsample_pipeline: ghi::PipelineHandle,
	composite_pipeline: ghi::PipelineHandle,
	extract_descriptor_set: ghi::DescriptorSetHandle,
	downsample_descriptor_sets: Vec<ghi::DescriptorSetHandle>,
	upsample_descriptor_sets: Vec<ghi::DescriptorSetHandle>,
	composite_descriptor_set: ghi::DescriptorSetHandle,
	downsample_images: Vec<ghi::DynamicImageHandle>,
	upsample_images: Vec<ghi::DynamicImageHandle>,
}

impl Entity for BloomPass {}

impl BloomPass {
	/// Creates a bloom pass with the default glow shaping parameters.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, BloomPassSettings::default())
	}

	/// Creates a bloom pass with caller-supplied settings and remaps `main` for downstream passes.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: BloomPassSettings) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("Bloom Output"),
		);
		render_pass_builder.alias("Bloom Output", "main");

		let device = render_pass_builder.device();
		let level_count = settings.resolved_level_count();
		let downsample_images = (0..level_count)
			.map(|index| {
				device.build_dynamic_image(
					ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image)
						.name(match index {
							0 => "Bloom Downsample 0",
							1 => "Bloom Downsample 1",
							2 => "Bloom Downsample 2",
							3 => "Bloom Downsample 3",
							4 => "Bloom Downsample 4",
							_ => "Bloom Downsample 5",
						})
						.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				)
			})
			.collect::<Vec<_>>();
		let upsample_images = (0..level_count.saturating_sub(1))
			.map(|index| {
				device.build_dynamic_image(
					ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image)
						.name(match index {
							0 => "Bloom Upsample 0",
							1 => "Bloom Upsample 1",
							2 => "Bloom Upsample 2",
							3 => "Bloom Upsample 3",
							_ => "Bloom Upsample 4",
						})
						.device_accesses(ghi::DeviceAccesses::DeviceOnly),
				)
			})
			.collect::<Vec<_>>();

		let parameters = device.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Bloom Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let extract_descriptor_set_layout = device.create_descriptor_set_template(
			Some("Bloom Extract Descriptor Set Layout"),
			&[EXTRACT_SOURCE_BINDING, EXTRACT_OUTPUT_BINDING, EXTRACT_PARAMETERS_BINDING],
		);
		let upsample_descriptor_set_layout = device.create_descriptor_set_template(
			Some("Bloom Upsample Descriptor Set Layout"),
			&[
				UPSAMPLE_LOW_BINDING,
				UPSAMPLE_HIGH_BINDING,
				UPSAMPLE_OUTPUT_BINDING,
				UPSAMPLE_PARAMETERS_BINDING,
			],
		);
		let composite_descriptor_set_layout = device.create_descriptor_set_template(
			Some("Bloom Composite Descriptor Set Layout"),
			&[
				COMPOSITE_SCENE_BINDING,
				COMPOSITE_BLOOM_BINDING,
				COMPOSITE_OUTPUT_BINDING,
				COMPOSITE_PARAMETERS_BINDING,
			],
		);

		let extract_pipeline = create_extract_pipeline(device, extract_descriptor_set_layout);
		let downsample_pipeline = create_downsample_pipeline(device, extract_descriptor_set_layout);
		let upsample_pipeline = create_upsample_pipeline(device, upsample_descriptor_set_layout);
		let composite_pipeline = create_composite_pipeline(device, composite_descriptor_set_layout);

		let extract_descriptor_set =
			device.create_descriptor_set(Some("Bloom Extract Descriptor Set"), &extract_descriptor_set_layout);
		let _ = device.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&EXTRACT_SOURCE_BINDING,
				source,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::image(&EXTRACT_OUTPUT_BINDING, downsample_images[0]),
		);
		let _ = device.create_descriptor_binding(
			extract_descriptor_set,
			ghi::BindingConstructor::buffer(&EXTRACT_PARAMETERS_BINDING, parameters.into()),
		);

		let downsample_descriptor_sets = (1..level_count)
			.map(|index| {
				let descriptor_set =
					device.create_descriptor_set(Some("Bloom Downsample Descriptor Set"), &extract_descriptor_set_layout);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&EXTRACT_SOURCE_BINDING,
						downsample_images[index - 1],
						sampler.clone(),
						ghi::Layouts::Read,
					),
				);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::image(&EXTRACT_OUTPUT_BINDING, downsample_images[index]),
				);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::buffer(&EXTRACT_PARAMETERS_BINDING, parameters.into()),
				);
				descriptor_set
			})
			.collect::<Vec<_>>();

		let upsample_descriptor_sets = (0..level_count.saturating_sub(1))
			.rev()
			.map(|level| {
				let descriptor_set =
					device.create_descriptor_set(Some("Bloom Upsample Descriptor Set"), &upsample_descriptor_set_layout);
				let low_resolution_source: ghi::BaseImageHandle = if level == level_count - 2 {
					downsample_images[level + 1].into()
				} else {
					upsample_images[level + 1].into()
				};
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&UPSAMPLE_LOW_BINDING,
						low_resolution_source,
						sampler.clone(),
						ghi::Layouts::Read,
					),
				);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::combined_image_sampler(
						&UPSAMPLE_HIGH_BINDING,
						downsample_images[level],
						sampler.clone(),
						ghi::Layouts::Read,
					),
				);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::image(&UPSAMPLE_OUTPUT_BINDING, upsample_images[level]),
				);
				let _ = device.create_descriptor_binding(
					descriptor_set,
					ghi::BindingConstructor::buffer(&UPSAMPLE_PARAMETERS_BINDING, parameters.into()),
				);
				descriptor_set
			})
			.collect::<Vec<_>>();

		let bloom_source: ghi::BaseImageHandle = if level_count == 1 {
			downsample_images[0].into()
		} else {
			upsample_images[0].into()
		};
		let composite_descriptor_set =
			device.create_descriptor_set(Some("Bloom Composite Descriptor Set"), &composite_descriptor_set_layout);
		let _ = device.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&COMPOSITE_SCENE_BINDING,
				bloom_source,
				sampler.clone(),
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(
				&COMPOSITE_BLOOM_BINDING,
				bloom_source,
				sampler,
				ghi::Layouts::Read,
			),
		);
		let _ = device.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::image(&COMPOSITE_OUTPUT_BINDING, output),
		);
		let _ = device.create_descriptor_binding(
			composite_descriptor_set,
			ghi::BindingConstructor::buffer(&COMPOSITE_PARAMETERS_BINDING, parameters.into()),
		);

		Self {
			settings,
			parameters,
			extract_pipeline,
			downsample_pipeline,
			upsample_pipeline,
			composite_pipeline,
			extract_descriptor_set,
			downsample_descriptor_sets,
			upsample_descriptor_sets,
			composite_descriptor_set,
			downsample_images,
			upsample_images,
		}
	}

	/// Writes the static bloom controls into the per-frame parameter buffer before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, intensity_multiplier: f32) {
		let parameters = frame.get_mut_dynamic_buffer_slice(self.parameters);

		parameters.prefilter = [
			self.settings.threshold.max(0.0),
			self.settings.soft_knee.clamp(0.0, 1.0),
			self.settings.intensity.max(0.0) * intensity_multiplier,
			0.0,
		];
		parameters.filter = [self.settings.radius.max(0.5), 0.0, 0.0, 0.0];
	}

	/// Resizes every bloom pyramid image to match the current viewport-dependent chain resolution.
	fn resize_images(&self, frame: &mut ghi::implementation::Frame, extent: Extent) {
		for (level, image) in self.downsample_images.iter().enumerate() {
			frame.resize_image((*image).into(), bloom_extent(extent, level));
		}

		for (level, image) in self.upsample_images.iter().enumerate() {
			frame.resize_image((*image).into(), bloom_extent(extent, level));
		}
	}
}

impl RenderPass for BloomPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		let extent = viewport.extent();
		let bloom_enabled = self.settings.enabled;

		self.resize_images(frame, extent);
		self.write_parameters(frame, if bloom_enabled { 1.0 } else { 0.0 });

		let extract_pipeline = self.extract_pipeline;
		let downsample_pipeline = self.downsample_pipeline;
		let upsample_pipeline = self.upsample_pipeline;
		let composite_pipeline = self.composite_pipeline;
		let extract_descriptor_set = self.extract_descriptor_set;
		let downsample_descriptor_sets = self.downsample_descriptor_sets.clone();
		let upsample_descriptor_sets = self.upsample_descriptor_sets.clone();
		let composite_descriptor_set = self.composite_descriptor_set;
		let level_count = self.downsample_images.len();

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("Bloom", |command_buffer| {
				if bloom_enabled {
					let extract = command_buffer.bind_compute_pipeline(extract_pipeline);
					extract.bind_descriptor_sets(&[extract_descriptor_set]);
					extract.dispatch(ghi::DispatchExtent::new(bloom_extent(extent, 0), bloom_dispatch_extent()));

					for (index, descriptor_set) in downsample_descriptor_sets.iter().enumerate() {
						let downsample = command_buffer.bind_compute_pipeline(downsample_pipeline);
						downsample.bind_descriptor_sets(&[*descriptor_set]);
						downsample.dispatch(ghi::DispatchExtent::new(
							bloom_extent(extent, index + 1),
							bloom_dispatch_extent(),
						));
					}

					if level_count > 1 {
						for (level, descriptor_set) in (0..level_count - 1).rev().zip(upsample_descriptor_sets.iter()) {
							let upsample = command_buffer.bind_compute_pipeline(upsample_pipeline);
							upsample.bind_descriptor_sets(&[*descriptor_set]);
							upsample.dispatch(ghi::DispatchExtent::new(bloom_extent(extent, level), bloom_dispatch_extent()));
						}
					}
				}

				let composite = command_buffer.bind_compute_pipeline(composite_pipeline);
				composite.bind_descriptor_sets(&[composite_descriptor_set]);
				composite.dispatch(ghi::DispatchExtent::new(extent, bloom_dispatch_extent()));
			});
		}))
	}
}

fn bloom_dispatch_extent() -> Extent {
	Extent::new(8, 8, 1)
}

fn bloom_extent(extent: Extent, level: usize) -> Extent {
	let divisor = 1u32 << (level as u32 + 1);
	Extent::rectangle(
		extent.width().div_ceil(divisor).max(1),
		extent.height().div_ceil(divisor).max(1),
	)
}

fn create_extract_pipeline(
	device: &mut ghi::implementation::Device,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader_artifact = glsl::compile(BLOOM_EXTRACT_SHADER, "Bloom Extract Shader")
		.expect("Failed to compile bloom extract shader. The most likely cause is invalid GLSL in the bloom extract stage.");
	let shader = device
		.create_shader(
			Some("Bloom Extract Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				EXTRACT_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				EXTRACT_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				EXTRACT_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		)
		.expect(
			"Failed to create bloom extract shader. The most likely cause is an incompatible bloom extract shader interface.",
		);

	device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_downsample_pipeline(
	device: &mut ghi::implementation::Device,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader_artifact = glsl::compile(BLOOM_DOWNSAMPLE_SHADER, "Bloom Downsample Shader").expect(
		"Failed to compile bloom downsample shader. The most likely cause is invalid GLSL in the bloom downsample stage.",
	);
	let shader = device
		.create_shader(
			Some("Bloom Downsample Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				EXTRACT_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				EXTRACT_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				EXTRACT_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		)
		.expect("Failed to create bloom downsample shader. The most likely cause is an incompatible bloom downsample shader interface.");

	device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_upsample_pipeline(
	device: &mut ghi::implementation::Device,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader_artifact = glsl::compile(BLOOM_UPSAMPLE_SHADER, "Bloom Upsample Shader")
		.expect("Failed to compile bloom upsample shader. The most likely cause is invalid GLSL in the bloom upsample stage.");
	let shader = device
		.create_shader(
			Some("Bloom Upsample Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				UPSAMPLE_LOW_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				UPSAMPLE_HIGH_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				UPSAMPLE_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				UPSAMPLE_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		)
		.expect(
			"Failed to create bloom upsample shader. The most likely cause is an incompatible bloom upsample shader interface.",
		);

	device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

fn create_composite_pipeline(
	device: &mut ghi::implementation::Device,
	descriptor_set_layout: ghi::DescriptorSetTemplateHandle,
) -> ghi::PipelineHandle {
	let shader_artifact = glsl::compile(BLOOM_COMPOSITE_SHADER, "Bloom Composite Shader").expect(
		"Failed to compile bloom composite shader. The most likely cause is invalid GLSL in the bloom composite stage.",
	);
	let shader = device
		.create_shader(
			Some("Bloom Composite Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				COMPOSITE_SCENE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				COMPOSITE_BLOOM_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				COMPOSITE_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				COMPOSITE_PARAMETERS_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
			],
		)
		.expect("Failed to create bloom composite shader. The most likely cause is an incompatible bloom composite shader interface.");

	device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
		&[descriptor_set_layout],
		&[],
		ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
	))
}

const BLOOM_EXTRACT_SHADER: &str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_image_load_formatted: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D source_texture;
layout(set=0, binding=1) uniform image2D result_texture;

struct BloomParameters {
	vec4 prefilter;
	vec4 blur_data;
};

layout(set=0, binding=2, scalar) readonly buffer BloomParametersBuffer {
	BloomParameters parameters;
};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

float bloom_brightness(vec3 color) {
	return max(max(color.r, color.g), color.b);
}

vec3 extract_bloom(vec3 color) {
	float threshold = parameters.prefilter.x;
	float soft_knee = parameters.prefilter.y;
	float knee = max(threshold * soft_knee, 1e-5);
	float brightness = bloom_brightness(color);
	float soft = clamp(brightness - threshold + knee, 0.0, 2.0 * knee);
	soft = (soft * soft) / (4.0 * knee + 1e-5);
	float contribution = max(soft, brightness - threshold);
	contribution /= max(brightness, 1e-5);
	return color * contribution;
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(result_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + 0.5) / vec2(extent);
	vec3 color = textureLod(source_texture, uv, 0.0).rgb;
	imageStore(result_texture, pixel, vec4(extract_bloom(color), 1.0));
}
"#;

const BLOOM_DOWNSAMPLE_SHADER: &str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_image_load_formatted: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D source_texture;
layout(set=0, binding=1) uniform image2D result_texture;

struct BloomParameters {
	vec4 prefilter;
	vec4 blur_data;
};

layout(set=0, binding=2, scalar) readonly buffer BloomParametersBuffer {
	BloomParameters parameters;
};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

vec3 downsample_dual_kawase(vec2 uv, vec2 texel_size) {
	float radius = parameters.blur_data.x;
	vec2 offset = texel_size * radius;
	vec3 sum = textureLod(source_texture, uv, 0.0).rgb * 4.0;
	sum += textureLod(source_texture, uv + vec2(-offset.x, -offset.y), 0.0).rgb;
	sum += textureLod(source_texture, uv + vec2(offset.x, -offset.y), 0.0).rgb;
	sum += textureLod(source_texture, uv + vec2(-offset.x, offset.y), 0.0).rgb;
	sum += textureLod(source_texture, uv + vec2(offset.x, offset.y), 0.0).rgb;
	return sum / 8.0;
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(result_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + 0.5) / vec2(extent);
	vec2 texel_size = 1.0 / vec2(textureSize(source_texture, 0));
	imageStore(result_texture, pixel, vec4(downsample_dual_kawase(uv, texel_size), 1.0));
}
"#;

const BLOOM_UPSAMPLE_SHADER: &str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_image_load_formatted: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D low_resolution_texture;
layout(set=0, binding=1) uniform sampler2D high_resolution_texture;
layout(set=0, binding=2) uniform image2D result_texture;

struct BloomParameters {
	vec4 prefilter;
	vec4 blur_data;
};

layout(set=0, binding=3, scalar) readonly buffer BloomParametersBuffer {
	BloomParameters parameters;
};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

vec3 upsample_dual_kawase(vec2 uv, vec2 texel_size) {
	float radius = parameters.blur_data.x;
	vec2 offset = texel_size * radius;
	vec3 sum = vec3(0.0);
	sum += textureLod(low_resolution_texture, uv + vec2(-offset.x, 0.0), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(offset.x, 0.0), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(0.0, -offset.y), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(0.0, offset.y), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(-offset.x, -offset.y), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(offset.x, -offset.y), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(-offset.x, offset.y), 0.0).rgb;
	sum += textureLod(low_resolution_texture, uv + vec2(offset.x, offset.y), 0.0).rgb;
	return sum / 8.0;
}

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(result_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + 0.5) / vec2(extent);
	vec2 texel_size = 1.0 / vec2(textureSize(low_resolution_texture, 0));
	vec3 low_resolution = upsample_dual_kawase(uv, texel_size);
	vec3 high_resolution = textureLod(high_resolution_texture, uv, 0.0).rgb;
	imageStore(result_texture, pixel, vec4(high_resolution + low_resolution, 1.0));
}
"#;

const BLOOM_COMPOSITE_SHADER: &str = r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_image_load_formatted: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D scene_texture;
layout(set=0, binding=1) uniform sampler2D bloom_texture;
layout(set=0, binding=2) uniform image2D result_texture;

struct BloomParameters {
	vec4 prefilter;
	vec4 blur_data;
};

layout(set=0, binding=3, scalar) readonly buffer BloomParametersBuffer {
	BloomParameters parameters;
};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

void main() {
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(result_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {
		return;
	}

	vec2 uv = (vec2(pixel) + 0.5) / vec2(extent);
	vec3 scene = textureLod(scene_texture, uv, 0.0).rgb;
	float intensity = parameters.prefilter.z;
	if (intensity <= 0.0) {
		imageStore(result_texture, pixel, vec4(scene, 1.0));
		return;
	}

	vec3 bloom = textureLod(bloom_texture, uv, 0.0).rgb;
	imageStore(result_texture, pixel, vec4(scene + bloom * intensity, 1.0));
}
"#;

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bloom_extract_shader_compiles() {
		resource_management::glsl::compile(BLOOM_EXTRACT_SHADER, "Bloom Extract Shader").unwrap();
	}

	#[test]
	fn bloom_downsample_shader_compiles() {
		resource_management::glsl::compile(BLOOM_DOWNSAMPLE_SHADER, "Bloom Downsample Shader").unwrap();
	}

	#[test]
	fn bloom_upsample_shader_compiles() {
		resource_management::glsl::compile(BLOOM_UPSAMPLE_SHADER, "Bloom Upsample Shader").unwrap();
	}

	#[test]
	fn bloom_composite_shader_compiles() {
		resource_management::glsl::compile(BLOOM_COMPOSITE_SHADER, "Bloom Composite Shader").unwrap();
	}

	#[test]
	fn bloom_level_count_is_clamped() {
		let settings = BloomPassSettings {
			levels: MAX_BLOOM_LEVELS + 4,
			..Default::default()
		};

		assert_eq!(settings.resolved_level_count(), MAX_BLOOM_LEVELS as usize);
	}

	#[test]
	fn bloom_extent_stays_non_zero() {
		let extent = bloom_extent(Extent::rectangle(1, 1), 4);

		assert_eq!(extent.width(), 1);
		assert_eq!(extent.height(), 1);
	}
}
