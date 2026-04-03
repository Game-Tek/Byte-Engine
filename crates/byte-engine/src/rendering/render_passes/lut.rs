use std::borrow::Borrow;
use std::boxed::Box as StdBox;

use ghi::{
	command_buffer::{BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommonCommandBufferMode as _},
	device::{Device as _, DeviceCreate as _},
	frame::Frame as _,
	types::Size as _,
};
use half::f16;
use resource_management::{
	glsl,
	resource::ReadTargetsMut,
	resources::lut::{Lut, LutKind},
	Reference,
};
use utils::{Box, Extent};

use crate::{
	core::Entity,
	rendering::{
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn},
		Viewport,
	},
};

const LUT_SOURCE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	0,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const LUT_TEXTURE_BINDING: ghi::DescriptorSetBindingTemplate = ghi::DescriptorSetBindingTemplate::new(
	1,
	ghi::descriptors::DescriptorType::CombinedImageSampler,
	ghi::Stages::COMPUTE,
);
const LUT_OUTPUT_BINDING: ghi::DescriptorSetBindingTemplate =
	ghi::DescriptorSetBindingTemplate::new(2, ghi::descriptors::DescriptorType::StorageImage, ghi::Stages::COMPUTE);

/// The `LutRenderPassSettings` struct carries the LUT resource used by the LUT grading pass.
pub struct LutRenderPassSettings {
	pub lut: Reference<Lut>,
}

/// The `LutRenderPass` struct applies a baked 3D LUT to the current `main` render target.
pub struct LutRenderPass {
	pipeline: ghi::PipelineHandle,
	descriptor_set: ghi::DescriptorSetHandle,
	lut: Lut,
	lut_reference: Option<Reference<Lut>>,
	lut_image: ghi::ImageHandle,
	lut_uploaded: bool,
}

impl Entity for LutRenderPass {}

impl LutRenderPass {
	/// Creates a LUT grading pass from an injected LUT reference.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, lut: Reference<Lut>) -> Self {
		Self::with_settings(render_pass_builder, LutRenderPassSettings { lut })
	}

	/// Creates a LUT grading pass with caller-supplied settings.
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: LutRenderPassSettings) -> Self {
		let LutRenderPassSettings { lut } = settings;
		let lut_metadata = lut.resource().clone();

		assert!(
			matches!(lut_metadata.kind, LutKind::ThreeDimensional),
			"Unsupported LUT kind for LUT render pass. The most likely cause is that the injected LUT resource is not a 3D LUT."
		);

		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("LUT Output"),
		);
		render_pass_builder.alias("LUT Output", "main");

		let device = render_pass_builder.device();

		let descriptor_set_layout = device.create_descriptor_set_template(
			Some("LUT Render Pass Descriptor Set"),
			&[LUT_SOURCE_BINDING, LUT_TEXTURE_BINDING, LUT_OUTPUT_BINDING],
		);

		let shader_source = create_lut_shader_source(&lut_metadata);
		let shader = create_lut_shader(device, &shader_source);
		let pipeline = device.create_compute_pipeline(ghi::pipelines::compute::Builder::new(
			&[descriptor_set_layout],
			&[],
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		));

		let source_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let lut_sampler = device.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);
		let lut_image = device.build_image(
			ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Image | ghi::Uses::TransferDestination)
				.name("LUT Texture")
				.extent(Extent::cube(lut_metadata.size, lut_metadata.size, lut_metadata.size))
				.device_accesses(ghi::DeviceAccesses::HostToDevice)
				.use_case(ghi::UseCases::STATIC),
		);

		let descriptor_set = device.create_descriptor_set(Some("LUT Render Pass Descriptor Set"), &descriptor_set_layout);
		let _ = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&LUT_SOURCE_BINDING, source, source_sampler, ghi::Layouts::Read),
		);
		let _ = device.create_descriptor_binding(
			descriptor_set,
			ghi::BindingConstructor::combined_image_sampler(&LUT_TEXTURE_BINDING, lut_image, lut_sampler, ghi::Layouts::Read),
		);
		let _ = device.create_descriptor_binding(descriptor_set, ghi::BindingConstructor::image(&LUT_OUTPUT_BINDING, output));

		Self {
			pipeline,
			descriptor_set,
			lut: lut_metadata,
			lut_reference: Some(lut),
			lut_image,
			lut_uploaded: false,
		}
	}

	/// Uploads the baked LUT payload into the cached GPU 3D texture the first time the pass is used.
	fn ensure_lut_uploaded(&mut self, frame: &mut ghi::implementation::Frame) {
		if self.lut_uploaded {
			return;
		}

		let lut_reference = self.lut_reference.as_mut().expect(
			"LUT reference missing during LUT upload. The most likely cause is that the LUT render pass lost its source resource before the first frame.",
		);
		let lut_bytes = load_lut_bytes(lut_reference);
		let upload_bytes = convert_lut_bytes_to_rgba16f_upload(&self.lut, &lut_bytes);
		let target = frame.get_texture_slice_mut(self.lut_image.into());

		assert_eq!(
			target.len(),
			upload_bytes.len(),
			"Unexpected LUT texture upload size. The most likely cause is that the GPU image extent or format does not match the LUT resource metadata."
		);
		target.copy_from_slice(&upload_bytes);
		frame.sync_texture(self.lut_image.into());

		self.lut_uploaded = true;
		self.lut_reference = None;
	}
}

impl RenderPass for LutRenderPass {
	fn prepare(&mut self, frame: &mut ghi::implementation::Frame, viewport: &Viewport) -> Option<RenderPassReturn> {
		self.ensure_lut_uploaded(frame);

		let pipeline = self.pipeline;
		let descriptor_set = self.descriptor_set;
		let extent = viewport.extent();

		Some(Box::new(move |command_buffer, _| {
			command_buffer.region("LUT", |command_buffer| {
				let pipeline = command_buffer.bind_compute_pipeline(pipeline);
				pipeline.bind_descriptor_sets(&[descriptor_set]);
				pipeline.dispatch(ghi::DispatchExtent::new(extent, Extent::new(8, 8, 1)));
			});
		}))
	}
}

fn create_lut_shader(device: &mut ghi::implementation::Device, shader_source: &str) -> ghi::ShaderHandle {
	if ghi::implementation::USES_METAL {
		return device
			.create_shader(
				Some("LUT Render Pass Compute Shader"),
				ghi::shader::Sources::MTL {
					source: shader_source,
					entry_point: "lut_apply",
				},
				ghi::ShaderTypes::Compute,
				[
					LUT_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					LUT_TEXTURE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
					LUT_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
				],
			)
			.expect("Failed to create LUT render shader. The most likely cause is an incompatible Metal shader interface.");
	}

	let shader_artifact = glsl::compile(shader_source, "LUT Render Pass")
		.expect("Failed to compile LUT render shader. The most likely cause is invalid GLSL syntax in the LUT render pass.");

	device
		.create_shader(
			Some("LUT Render Pass Compute Shader"),
			ghi::shader::Sources::SPIRV(shader_artifact.borrow().into()),
			ghi::ShaderTypes::Compute,
			[
				LUT_SOURCE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				LUT_TEXTURE_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::READ),
				LUT_OUTPUT_BINDING.into_shader_binding_descriptor(0, ghi::AccessPolicies::WRITE),
			],
		)
		.expect("Failed to create LUT render shader. The most likely cause is an incompatible shader interface.")
}

/// Generates the platform-specific LUT shader source with the injected LUT domain constants baked in.
fn create_lut_shader_source(lut: &Lut) -> String {
	let domain_scale = lut
		.domain_min
		.into_iter()
		.zip(lut.domain_max)
		.map(|(minimum, maximum)| 1.0 / (maximum - minimum))
		.collect::<Vec<_>>();
	let lut_size = lut.size as f32;
	let lut_texel_scale = (lut_size - 1.0) / lut_size;
	let lut_texel_offset = 0.5 / lut_size;

	if ghi::implementation::USES_METAL {
		return format!(
			r#"
			#include <metal_stdlib>
			using namespace metal;

			struct LutSet0 {{
				texture2d<float> source [[id(0)]];
				sampler source_sampler [[id(1)]];
				texture3d<float> lut_texture [[id(2)]];
				sampler lut_texture_sampler [[id(3)]];
				texture2d<float, access::write> result [[id(4)]];
			}};

			constant float3 LUT_DOMAIN_MIN = float3({:.9}, {:.9}, {:.9});
			constant float3 LUT_DOMAIN_SCALE = float3({:.9}, {:.9}, {:.9});
			constant float LUT_TEXEL_SCALE = {:.9};
			constant float LUT_TEXEL_OFFSET = {:.9};

			float3 apply_lut(float3 color, const constant LutSet0& set0) {{
				float3 normalized = clamp((color - LUT_DOMAIN_MIN) * LUT_DOMAIN_SCALE, float3(0.0), float3(1.0));
				float3 lut_uv = normalized * LUT_TEXEL_SCALE + LUT_TEXEL_OFFSET;
				return set0.lut_texture.sample(set0.lut_texture_sampler, lut_uv).rgb;
			}}

			kernel void lut_apply(
				uint2 gid [[thread_position_in_grid]],
				constant LutSet0& set0 [[buffer(16)]]
			) {{
				if (gid.x >= set0.result.get_width() || gid.y >= set0.result.get_height()) {{
					return;
				}}

				float2 uv = (float2(gid) + 0.5) / float2(set0.result.get_width(), set0.result.get_height());
				float4 source_color = set0.source.sample(set0.source_sampler, uv);
				float3 result_color = apply_lut(source_color.rgb, set0);
				set0.result.write(float4(result_color, source_color.a), gid);
			}}
		"#,
			lut.domain_min[0],
			lut.domain_min[1],
			lut.domain_min[2],
			domain_scale[0],
			domain_scale[1],
			domain_scale[2],
			lut_texel_scale,
			lut_texel_offset
		);
	}

	format!(
		r#"
#version 460 core
#pragma shader_stage(compute)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_shader_image_load_formatted: enable

layout(row_major) uniform;
layout(row_major) buffer;

layout(set=0, binding=0) uniform sampler2D source_texture;
layout(set=0, binding=1) uniform sampler3D lut_texture;
layout(set=0, binding=2) uniform image2D result_texture;

const vec3 LUT_DOMAIN_MIN = vec3({:.9}, {:.9}, {:.9});
const vec3 LUT_DOMAIN_SCALE = vec3({:.9}, {:.9}, {:.9});
const float LUT_TEXEL_SCALE = {:.9};
const float LUT_TEXEL_OFFSET = {:.9};

layout(local_size_x=8, local_size_y=8, local_size_z=1) in;

vec3 apply_lut(vec3 color) {{
	vec3 normalized = clamp((color - LUT_DOMAIN_MIN) * LUT_DOMAIN_SCALE, vec3(0.0), vec3(1.0));
	vec3 lut_uv = normalized * LUT_TEXEL_SCALE + LUT_TEXEL_OFFSET;
	return textureLod(lut_texture, lut_uv, 0.0).rgb;
}}

void main() {{
	ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
	ivec2 extent = imageSize(result_texture);

	if (pixel.x >= extent.x || pixel.y >= extent.y) {{
		return;
	}}

	vec2 uv = (vec2(pixel) + 0.5) / vec2(extent);
	vec4 source_color = textureLod(source_texture, uv, 0.0);
	vec3 result_color = apply_lut(source_color.rgb);
	imageStore(result_texture, pixel, vec4(result_color, source_color.a));
}}
"#,
		lut.domain_min[0],
		lut.domain_min[1],
		lut.domain_min[2],
		domain_scale[0],
		domain_scale[1],
		domain_scale[2],
		lut_texel_scale,
		lut_texel_offset
	)
}

/// Reads the baked LUT payload from the resource reference into owned bytes.
fn load_lut_bytes(reference: &mut Reference<Lut>) -> StdBox<[u8]> {
	let read_target = ReadTargetsMut::Box(vec![0_u8; reference.size].into_boxed_slice());
	let read_result = reference.load(read_target).expect(
		"Failed to read LUT resource data. The most likely cause is that the cached LUT payload is missing or unreadable.",
	);

	match read_result {
		resource_management::resource::ReadTargets::Box(bytes) => bytes,
		resource_management::resource::ReadTargets::Buffer(bytes) => bytes.into(),
		resource_management::resource::ReadTargets::Streams(_) => {
			panic!(
				"Unexpected LUT stream layout. The most likely cause is that the LUT resource was stored as streams instead of a flat payload."
			);
		}
	}
}

/// Converts the baked LUT RGB float payload into an RGBA16F 3D texture upload buffer.
fn convert_lut_bytes_to_rgba16f_upload(lut: &Lut, lut_bytes: &[u8]) -> StdBox<[u8]> {
	assert!(
		matches!(lut.kind, LutKind::ThreeDimensional),
		"Unsupported LUT kind for upload. The most likely cause is that a non-3D LUT resource reached the LUT render pass."
	);

	let expected_size = expected_lut_payload_size(lut);
	assert_eq!(
		lut_bytes.len(),
		expected_size,
		"Invalid LUT payload size. The most likely cause is that the baked LUT binary does not match the LUT metadata."
	);

	let texel_count = lut
		.kind
		.expected_entry_count(lut.size)
		.expect("Invalid LUT dimensions. The most likely cause is that the LUT size overflowed during texture upload.");
	let mut upload_bytes = Vec::with_capacity(texel_count * 4 * std::mem::size_of::<u16>());

	for rgb in lut_bytes.chunks_exact(3 * std::mem::size_of::<f32>()) {
		let r = f32::from_le_bytes(rgb[0..4].try_into().unwrap());
		let g = f32::from_le_bytes(rgb[4..8].try_into().unwrap());
		let b = f32::from_le_bytes(rgb[8..12].try_into().unwrap());

		upload_bytes.extend_from_slice(&f16::from_f32(r).to_bits().to_le_bytes());
		upload_bytes.extend_from_slice(&f16::from_f32(g).to_bits().to_le_bytes());
		upload_bytes.extend_from_slice(&f16::from_f32(b).to_bits().to_le_bytes());
		upload_bytes.extend_from_slice(&f16::from_f32(1.0).to_bits().to_le_bytes());
	}

	upload_bytes.into_boxed_slice()
}

fn expected_lut_payload_size(lut: &Lut) -> usize {
	lut.kind
		.expected_entry_count(lut.size)
		.and_then(|entry_count| entry_count.checked_mul(3 * std::mem::size_of::<f32>()))
		.expect("Invalid LUT payload size calculation. The most likely cause is that the LUT dimensions overflowed.")
}

#[cfg(test)]
mod tests {
	use half::f16;
	use resource_management::resources::lut::{Lut, LutKind};

	use super::{convert_lut_bytes_to_rgba16f_upload, create_lut_shader_source, expected_lut_payload_size};

	#[test]
	fn lut_shader_compiles() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 16,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		let shader_source = create_lut_shader_source(&lut);

		resource_management::glsl::compile(&shader_source, "LUT Render Pass Test").unwrap();
	}

	#[test]
	fn converts_rgb_float_payload_into_rgba16f_upload() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};
		let mut lut_bytes = Vec::new();

		for [r, g, b] in [
			[0.0_f32, 0.0, 0.0],
			[1.0, 0.0, 0.0],
			[0.0, 1.0, 0.0],
			[1.0, 1.0, 0.0],
			[0.0, 0.0, 1.0],
			[1.0, 0.0, 1.0],
			[0.0, 1.0, 1.0],
			[1.0, 1.0, 1.0],
		] {
			lut_bytes.extend_from_slice(&r.to_le_bytes());
			lut_bytes.extend_from_slice(&g.to_le_bytes());
			lut_bytes.extend_from_slice(&b.to_le_bytes());
		}

		let upload = convert_lut_bytes_to_rgba16f_upload(&lut, &lut_bytes);

		assert_eq!(upload.len(), 8 * 4 * std::mem::size_of::<u16>());
		assert_eq!(
			u16::from_le_bytes(upload[0..2].try_into().unwrap()),
			f16::from_f32(0.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[6..8].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[upload.len() - 8..upload.len() - 6].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
		assert_eq!(
			u16::from_le_bytes(upload[upload.len() - 2..].try_into().unwrap()),
			f16::from_f32(1.0).to_bits()
		);
	}

	#[test]
	#[should_panic(expected = "Invalid LUT payload size")]
	fn rejects_invalid_lut_payload_size() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 2,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		let _ = convert_lut_bytes_to_rgba16f_upload(&lut, &[0_u8; 8]);
	}

	#[test]
	fn computes_expected_lut_payload_size() {
		let lut = Lut {
			kind: LutKind::ThreeDimensional,
			size: 4,
			domain_min: [0.0, 0.0, 0.0],
			domain_max: [1.0, 1.0, 1.0],
		};

		assert_eq!(
			expected_lut_payload_size(&lut),
			4_usize.pow(3) * 3 * std::mem::size_of::<f32>()
		);
	}
}
