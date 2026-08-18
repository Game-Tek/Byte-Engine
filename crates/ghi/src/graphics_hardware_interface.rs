//! Defines backend-independent handles and resource descriptions for GPU rendering.
//!
//! These types do not require a specific render-pipeline architecture.

mod handles;
mod queue;
mod resources;

pub use handles::*;
pub use queue::*;
pub use resources::*;
#[cfg(test)]
use utils::{Extent, RGBA};

#[cfg(test)]
use crate::{descriptors, DataTypes, Encodings, Formats, Layouts};

#[cfg(test)]
pub(super) mod tests {

	use super::*;
	use crate::{
		command_buffer::{
			BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
			BoundRayTracingPipelineMode as _, CommandBuffer as _, CommandBufferRecording as _, CommonCommandBufferMode as _,
			RasterizationRenderPassMode as _,
		},
		frame::Frame as _,
		pipelines::{self, raster::AttachmentDescriptor, PushConstantRange, ShaderParameter, VertexElement},
		queue::{FrameRequest, Queue as _, QueueExecution as _},
		rt::{
			BindingTables, BottomLevelAccelerationStructureBuild, BottomLevelAccelerationStructureBuildDescriptions,
			TopLevelAccelerationStructureBuild, TopLevelAccelerationStructureBuildDescriptions,
		},
		shader::{CompiledShaderSource, ShaderSource},
		BufferDescriptor, BufferStridedRange, DeviceAccesses, FilteringModes, SamplerAddressingModes, SamplingReductionModes,
		ShaderTypes, UseCases, Uses, Window,
	};
	use crate::{ChannelBitSize, ChannelLayout, Size as _};

	#[test]
	fn attachment_layer_builders_keep_single_and_layered_rendering_distinct() {
		let single_layer =
			AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layer(3);
		assert_eq!(single_layer.layer, Some(3));
		assert_eq!(single_layer.layer_count, None);

		let layered =
			AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
				.layers(4);
		assert_eq!(layered.layer, None);
		assert_eq!(layered.layer_count.map(std::num::NonZeroU32::get), Some(4));
		assert_eq!(AttachmentInformation::render_pass_layer_count(&[layered, layered]), 4);
	}

	#[test]
	#[should_panic(expected = "Cannot select one attachment layer after enabling layered rendering")]
	fn attachment_rejects_layer_after_one_layer_layered_rendering() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
			.layers(1)
			.layer(0);
	}

	#[test]
	#[should_panic(expected = "Layered rendering requires at least one attachment layer")]
	fn attachment_rejects_empty_layered_rendering() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layers(0);
	}

	#[test]
	#[should_panic(expected = "Cannot enable layered rendering after selecting one attachment layer")]
	fn attachment_rejects_layered_rendering_after_layer() {
		AttachmentInformation::new(BaseImageHandle(1), Layouts::RenderTarget, ClearValue::Depth(0.0), false, true)
			.layer(0)
			.layers(1);
	}

	#[test]
	#[should_panic(expected = "Render-pass attachments use different layer counts")]
	fn render_pass_rejects_mixed_attachment_layer_counts() {
		let target = BaseImageHandle(1);
		let single = AttachmentInformation::new(target, Layouts::RenderTarget, ClearValue::Depth(0.0), false, true);
		let layered = AttachmentInformation::new(target, Layouts::RenderTarget, ClearValue::Depth(0.0), false, true).layers(4);

		AttachmentInformation::render_pass_layer_count(&[single, layered]);
	}

	#[test]
	fn test_formats_encoding() {
		// Test floating point formats
		assert_eq!(Formats::R8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::R32F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RG16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGB16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA8F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::RGBA16F.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(Formats::Depth32.encoding(), Some(Encodings::FloatingPoint));

		// Test unsigned normalized formats
		assert_eq!(Formats::R8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::R32UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RG16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGB16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA8UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBA16UNORM.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::Depth16.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::RGBu11u11u10.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(Formats::BGRAu8.encoding(), Some(Encodings::UnsignedNormalized));

		// Test signed normalized formats
		assert_eq!(Formats::R8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::R32SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RG16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGB16SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA8SNORM.encoding(), Some(Encodings::SignedNormalized));
		assert_eq!(Formats::RGBA16SNORM.encoding(), Some(Encodings::SignedNormalized));

		// Test sRGB formats
		assert_eq!(Formats::R8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::R32sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RG16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGB16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA8sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::RGBA16sRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::BGRAsRGB.encoding(), Some(Encodings::sRGB));
		assert_eq!(Formats::BC7SRGB.encoding(), Some(Encodings::sRGB));

		// Test formats without encoding
		assert_eq!(Formats::U32.encoding(), None);
		assert_eq!(Formats::BC5.encoding(), None);
		assert_eq!(Formats::BC7.encoding(), None);
	}

	#[test]
	fn descriptor_write_constructors_preserve_set_slot_array_and_frame_semantics() {
		let set = DescriptorSetHandle(1);
		let slot = crate::shader::ResourceSlot::new(9);
		let buffer = BaseBufferHandle(2);
		let image = ImageHandle(BaseImageHandle(3));
		let sampler = SamplerHandle(4);
		let acceleration_structure = TopLevelAccelerationStructureHandle(5);

		let buffer_write = descriptors::DescriptorWrite::buffer(set, slot, buffer);
		assert_eq!(buffer_write.descriptor_set, set);
		assert_eq!(buffer_write.slot, slot);
		assert_eq!(buffer_write.array_element, 0);
		assert_eq!(buffer_write.frame_offset, None);
		assert!(matches!(
			buffer_write.descriptor,
			descriptors::WriteData::Buffer {
				handle,
				size: Ranges::Whole
			} if handle == buffer
		));

		let image_write = descriptors::DescriptorWrite::image_with_frame(set, slot, image, Layouts::General, -1);
		assert_eq!(image_write.frame_offset, Some(-1));
		assert!(matches!(
			image_write.descriptor,
			descriptors::WriteData::Image {
				handle,
				layout: Layouts::General,
				mip_level: None,
			} if handle == BaseImageHandle(3)
		));
		let mip_write = descriptors::DescriptorWrite::image_mip(set, slot, image, Layouts::Read, 2);
		assert!(matches!(
			mip_write.descriptor,
			descriptors::WriteData::Image {
				handle,
				layout: Layouts::Read,
				mip_level: Some(2),
			} if handle == BaseImageHandle(3)
		));

		let array_write = descriptors::DescriptorWrite::combined_image_sampler_array_with_frame(
			set,
			slot,
			image,
			sampler,
			Layouts::Read,
			7,
			2,
		);
		assert_eq!(array_write.array_element, 7);
		assert_eq!(array_write.frame_offset, Some(2));
		assert!(matches!(
			array_write.descriptor,
			descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout: Layouts::Read,
				layer: None,
			} if image_handle == BaseImageHandle(3) && sampler_handle == sampler
		));

		let sampler_write = descriptors::DescriptorWrite::sampler(set, slot, sampler);
		assert!(matches!(sampler_write.descriptor, descriptors::WriteData::Sampler(value) if value == sampler));
		let acceleration_write = descriptors::DescriptorWrite::acceleration_structure(set, slot, acceleration_structure);
		assert!(matches!(
			acceleration_write.descriptor,
			descriptors::WriteData::AccelerationStructure { handle } if handle == acceleration_structure
		));
	}

	#[test]
	fn descriptor_write_variants_without_frame_offsets_remain_frame_invariant() {
		let set = DescriptorSetHandle(8);
		let slot = crate::shader::ResourceSlot::new(12);
		let image = ImageHandle(BaseImageHandle(9));
		let sampler = SamplerHandle(10);

		let image_write = descriptors::DescriptorWrite::image(set, slot, image, Layouts::Read);
		let combined = descriptors::DescriptorWrite::combined_image_sampler(set, slot, image, sampler, Layouts::Read);
		let array = descriptors::DescriptorWrite::combined_image_sampler_array(set, slot, image, sampler, Layouts::Read, 3);

		assert_eq!(image_write.frame_offset, None);
		assert_eq!(combined.frame_offset, None);
		assert_eq!(array.frame_offset, None);
		assert_eq!(array.array_element, 3);
	}

	#[test]
	fn test_formats_channel_bit_size() {
		// Test 8-bit formats
		assert_eq!(Formats::R8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::R8sRGB.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RG8F.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGB8UNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::RGBA8SNORM.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAu8.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(Formats::BGRAsRGB.channel_bit_size(), ChannelBitSize::Bits8);

		// Test 16-bit formats
		assert_eq!(Formats::R16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::R16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RG16SNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGB16F.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::RGBA16UNORM.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(Formats::Depth16.channel_bit_size(), ChannelBitSize::Bits16);

		// Test 32-bit formats
		assert_eq!(Formats::R32F.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::R32UNORM.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::Depth32.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(Formats::U32.channel_bit_size(), ChannelBitSize::Bits32);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(Formats::BC5.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(Formats::BC7SRGB.channel_bit_size(), ChannelBitSize::Compressed);
	}

	#[test]
	fn test_formats_channel_layout() {
		// Test single channel formats
		assert_eq!(Formats::R8F.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R16UNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R32SNORM.channel_layout(), ChannelLayout::R);
		assert_eq!(Formats::R8sRGB.channel_layout(), ChannelLayout::R);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG16UNORM.channel_layout(), ChannelLayout::RG);
		assert_eq!(Formats::RG8SNORM.channel_layout(), ChannelLayout::RG);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB16UNORM.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGB8sRGB.channel_layout(), ChannelLayout::RGB);
		assert_eq!(Formats::RGBu11u11u10.channel_layout(), ChannelLayout::RGB);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA16UNORM.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(Formats::RGBA8SNORM.channel_layout(), ChannelLayout::RGBA);

		// Test BGRA format
		assert_eq!(Formats::BGRAu8.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(Formats::BGRAsRGB.channel_layout(), ChannelLayout::BGRA);

		// Test depth format
		assert_eq!(Formats::Depth16.channel_layout(), ChannelLayout::Depth);
		assert_eq!(Formats::Depth32.channel_layout(), ChannelLayout::Depth);

		// Test packed format
		assert_eq!(Formats::U32.channel_layout(), ChannelLayout::Packed);

		// Test block compressed formats
		assert_eq!(Formats::BC5.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7.channel_layout(), ChannelLayout::BC);
		assert_eq!(Formats::BC7SRGB.channel_layout(), ChannelLayout::BC);
	}

	#[test]
	fn test_formats_size() {
		// Test single channel formats
		assert_eq!(Formats::R8F.size(), 1);
		assert_eq!(Formats::R8UNORM.size(), 1);
		assert_eq!(Formats::R16F.size(), 2);
		assert_eq!(Formats::R16UNORM.size(), 2);
		assert_eq!(Formats::R32F.size(), 4);
		assert_eq!(Formats::R32SNORM.size(), 4);

		// Test dual channel formats
		assert_eq!(Formats::RG8F.size(), 2);
		assert_eq!(Formats::RG8UNORM.size(), 2);
		assert_eq!(Formats::RG16F.size(), 4);
		assert_eq!(Formats::RG16SNORM.size(), 4);

		// Test triple channel formats
		assert_eq!(Formats::RGB8F.size(), 3);
		assert_eq!(Formats::RGB8UNORM.size(), 3);
		assert_eq!(Formats::RGB16F.size(), 6);
		assert_eq!(Formats::RGB16SNORM.size(), 6);

		// Test quad channel formats
		assert_eq!(Formats::RGBA8F.size(), 4);
		assert_eq!(Formats::RGBA8UNORM.size(), 4);
		assert_eq!(Formats::RGBA16F.size(), 8);
		assert_eq!(Formats::RGBA16UNORM.size(), 8);

		// Test special formats
		assert_eq!(Formats::RGBu11u11u10.size(), 4);
		assert_eq!(Formats::BGRAu8.size(), 4);
		assert_eq!(Formats::BGRAsRGB.size(), 4);
		assert_eq!(Formats::Depth16.size(), 2);
		assert_eq!(Formats::Depth32.size(), 4);
		assert_eq!(Formats::U32.size(), 4);
		assert_eq!(Formats::BC5.size(), 1);
		assert_eq!(Formats::BC7.size(), 1);
	}

	#[test]
	fn test_formats_comprehensive() {
		// Test that encoding, channel_bit_size, and channel_layout are consistent
		// For R8FloatingPoint
		let format = Formats::R8F;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::R);
		assert_eq!(format.size(), 1);

		// For RGBA16UnsignedNormalized
		let format = Formats::RGBA16UNORM;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(format.channel_layout(), ChannelLayout::RGBA);
		assert_eq!(format.size(), 8);

		// For RGB8sRGB
		let format = Formats::RGB8sRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 3);

		// For special format RGBu11u11u10
		let format = Formats::RGBu11u11u10;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits11_11_10);
		assert_eq!(format.channel_layout(), ChannelLayout::RGB);
		assert_eq!(format.size(), 4);

		// For Depth16
		let format = Formats::Depth16;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits16);
		assert_eq!(format.channel_layout(), ChannelLayout::Depth);
		assert_eq!(format.size(), 2);

		// For BGRAu8
		let format = Formats::BGRAu8;
		assert_eq!(format.encoding(), Some(Encodings::UnsignedNormalized));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(format.size(), 4);

		// For BGRAsRGB
		let format = Formats::BGRAsRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits8);
		assert_eq!(format.channel_layout(), ChannelLayout::BGRA);
		assert_eq!(format.size(), 4);

		// For Depth32
		let format = Formats::Depth32;
		assert_eq!(format.encoding(), Some(Encodings::FloatingPoint));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Bits32);
		assert_eq!(format.channel_layout(), ChannelLayout::Depth);
		assert_eq!(format.size(), 4);

		// For BC7
		let format = Formats::BC7;
		assert_eq!(format.encoding(), None);
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(format.channel_layout(), ChannelLayout::BC);
		assert_eq!(format.size(), 1);

		// For BC7 sRGB
		let format = Formats::BC7SRGB;
		assert_eq!(format.encoding(), Some(Encodings::sRGB));
		assert_eq!(format.channel_bit_size(), ChannelBitSize::Compressed);
		assert_eq!(format.channel_layout(), ChannelLayout::BC);
		assert_eq!(format.size(), 1);
	}

	fn compile_shaders() -> (CompiledShaderSource, CompiledShaderSource) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput vertex_main(VertexInput input [[stage_in]]) {
				return VertexOutput { float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 fragment_main(VertexOutput input [[stage_in]]) {
				return input.color;
			}
		"#;
		let vertex_shader_hlsl = r#"
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = float4(input.position, 1.0);
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 { return input.color; }
		"#;

		let vertex_shader_artifact = crate::shader::compile(
			"GHI test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "vertex_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect("Failed to compile GHI test vertex shader. The most likely cause is invalid native shader source.");
		let fragment_shader_artifact = crate::shader::compile(
			"GHI test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "fragment_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect("Failed to compile GHI test fragment shader. The most likely cause is invalid native shader source.");

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	fn compile_shaders_with_model_matrix() -> (CompiledShaderSource, CompiledShaderSource) {
		let vertex_shader_code = "
			#version 450
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(push_constant) uniform ModelMatrix {
				mat4 model_matrix;
			} push_constants;

			void main() {
				out_color = in_color;
				gl_Position = push_constants.model_matrix * vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			void main() {
				out_color = in_color;
			}
		";
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput vertex_main(
				VertexInput input [[stage_in]],
				constant float4x4& model_matrix [[buffer(15)]]) {
				return VertexOutput { model_matrix * float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 fragment_main(VertexOutput input [[stage_in]]) {
				return input.color;
			}
		"#;
		let vertex_shader_hlsl = r#"
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			struct PushConstant { float4x4 model_matrix; };
			ConstantBuffer<PushConstant> push_constant : register(b0, space0);
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = mul(push_constant.model_matrix, float4(input.position, 1.0));
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 { return input.color; }
		"#;

		let vertex_shader_artifact = crate::shader::compile(
			"GHI model-matrix test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "vertex_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect(
			"Failed to compile GHI model-matrix test vertex shader. The most likely cause is invalid native shader source.",
		);
		let fragment_shader_artifact = crate::shader::compile(
			"GHI model-matrix test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "fragment_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect("Failed to compile GHI test fragment shader. The most likely cause is invalid native shader source.");

		(vertex_shader_artifact, fragment_shader_artifact)
	}

	#[test]
	fn dispatch_extent() {
		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(5, 5, 5));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(2, 2, 2));

		let dispatch_extent = DispatchExtent::new(Extent::new(10, 10, 10), Extent::new(3, 3, 3));
		assert_eq!(dispatch_extent.get_extent(), Extent::new(4, 4, 4));
	}

	fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
		assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

		let pixel = pixels[0]; // top left
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 0,
				a: 255
			}
		);

		if !extent.width().is_multiple_of(2) {
			let pixel = pixels[(extent.width() / 2) as usize]; // middle top center
			assert_eq!(
				pixel,
				RGBAu8 {
					r: 255,
					g: 0,
					b: 0,
					a: 255
				}
			);
		}

		let pixel = pixels[(extent.width() - 1) as usize]; // top right
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 0,
				a: 255
			}
		);

		let pixel = pixels[(extent.width() * (extent.height() - 1)) as usize]; // bottom left
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 0,
				b: 255,
				a: 255
			}
		);

		let pixel = pixels[(extent.width() * extent.height() - (extent.width() / 2)) as usize]; // middle bottom center
		assert!(
			pixel
				== RGBAu8 {
					r: 0,
					g: 127,
					b: 127,
					a: 255
				} || pixel
				== RGBAu8 {
					r: 0,
					g: 128,
					b: 127,
					a: 255
				}
		); // different implementations render slightly differently

		let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
		assert_eq!(
			pixel,
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255
			}
		);
	}

	pub(crate) fn render_triangle(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, false);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::STATIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		device.start_frame_capture();

		let texture_copy_handles = {
			let mut command_buffer = device.command_buffer(command_buffer_handle);
			let mut command_buffer_recording = command_buffer.create_command_buffer_recording();

			let attachments = [AttachmentInformation::new(
				render_target,
				Layouts::RenderTarget,
				ClearValue::Color(RGBA::black()),
				false,
				true,
			)];

			let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

			let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

			raster_pipeline_command.draw_mesh(&mesh);

			render_pass_command.end_render_pass();

			let texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);

			command_buffer_recording.execute(signal);
			texture_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		// Get image data and cast u8 slice to rgbau8
		let pixels = unsafe {
			std::slice::from_raw_parts(
				device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
				(extent.width() * extent.height()) as usize,
			)
		};

		check_triangle(pixels, extent);
	}

	#[cfg(target_os = "macos")]
	/// Uploads one overlapping triangle with a constant depth and color for native Metal depth-state validation.
	fn add_depth_state_test_triangle(
		device: &mut impl crate::context::Context,
		depth: f32,
		color: [f32; 4],
		scale: f32,
		vertex_layout: &[VertexElement],
	) -> MeshHandle {
		let vertices: [f32; 21] = [
			0.0, scale, depth, color[0], color[1], color[2], color[3], scale, -scale, depth, color[0], color[1], color[2],
			color[3], -scale, -scale, depth, color[0], color[1], color[2], color[3],
		];
		let indices = [0u16, 1u16, 2u16];

		// The upload API accepts bytes, and both stack arrays remain alive for the complete synchronous upload call.
		unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(vertices.as_ptr().cast(), std::mem::size_of_val(&vertices)),
				std::slice::from_raw_parts(indices.as_ptr().cast(), std::mem::size_of_val(&indices)),
				vertex_layout,
			)
		}
	}

	#[cfg(target_os = "macos")]
	/// Verifies that a Metal raster pipeline can depth-test without replacing the retained reverse-Z depth.
	pub(crate) fn render_without_depth_writes(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, false);
		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];
		let first = add_depth_state_test_triangle(device, 0.8, [1.0, 0.0, 0.0, 1.0], 1.0, &vertex_layout);
		let no_write = add_depth_state_test_triangle(device, 0.9, [0.0, 1.0, 0.0, 1.0], 1.0, &vertex_layout);
		let last = add_depth_state_test_triangle(device, 0.85, [0.0, 0.0, 1.0, 1.0], 0.5, &vertex_layout);
		let behind = add_depth_state_test_triangle(device, 0.7, [1.0, 1.0, 0.0, 1.0], 1.0, &vertex_layout);

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();
		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect(
				"Failed to create the Metal depth-state test vertex shader. The most likely cause is invalid native shader source.",
			);
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect(
				"Failed to create the Metal depth-state test fragment shader. The most likely cause is invalid native shader source.",
			);
		let shaders = [
			ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
			ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
		];
		let attachment_descriptors = [
			AttachmentDescriptor::new(Formats::RGBA8UNORM),
			AttachmentDescriptor::new(Formats::Depth32),
		];
		let depth_write_pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&shaders,
			&attachment_descriptors,
		));
		let no_depth_write_pipeline = device.create_raster_pipeline(
			pipelines::raster::Builder::new(&[], &vertex_layout, &shaders, &attachment_descriptors).depth_write(false),
		);

		let extent = Extent::rectangle(9, 9);
		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::STATIC),
		);
		let depth_target = device.build_image(
			crate::image::Builder::new(Formats::Depth32, Uses::DepthStencil)
				.extent(extent)
				.use_case(UseCases::STATIC)
				.optimized_clear_value(ClearValue::Depth(0.0)),
		);
		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let texture_copy_handles = {
			let mut command_buffer = device.command_buffer(command_buffer_handle);
			let mut recording = command_buffer.create_command_buffer_recording();
			let attachments = [
				AttachmentInformation::new(
					render_target,
					Layouts::RenderTarget,
					ClearValue::Color(RGBA::black()),
					false,
					true,
				),
				AttachmentInformation::new(depth_target, Layouts::RenderTarget, ClearValue::Depth(0.0), false, true),
			];
			let render_pass = recording.start_render_pass(extent, &attachments);

			// With reverse-Z, the middle draw passes at 0.9 but must leave the first draw's 0.8 depth intact.
			render_pass.bind_raster_pipeline(depth_write_pipeline).draw_mesh(&first);
			render_pass.bind_raster_pipeline(no_depth_write_pipeline).draw_mesh(&no_write);
			render_pass.bind_raster_pipeline(depth_write_pipeline).draw_mesh(&last);
			// A later no-write draw behind retained opaque depth must still be rejected by the depth test.
			render_pass.bind_raster_pipeline(no_depth_write_pipeline).draw_mesh(&behind);
			render_pass.end_render_pass();

			let texture_copy_handles = recording.transfer_textures(&[render_target.into()]);
			recording.execute(signal);
			texture_copy_handles
		};

		device.wait();
		assert!(
			!device.has_errors(),
			"Metal depth-state rendering failed. The most likely cause is an invalid pipeline or render-pass attachment configuration.",
		);
		let copy_handle = *texture_copy_handles.first().expect(
			"Missing Metal depth-state test readback. The most likely cause is that the color target was not created for CPU access.",
		);
		let image_data = device.get_image_data(copy_handle);
		let expected_byte_count = (extent.width() * extent.height()) as usize * std::mem::size_of::<RGBAu8>();
		assert_eq!(
			image_data.len(),
			expected_byte_count,
			"Unexpected Metal depth-state test readback size. The most likely cause is a non-compact RGBA8 staging layout.",
		);
		let center_pixel = (extent.width() * (extent.height() / 2) + extent.width() / 2) as usize;
		let center_byte = center_pixel * std::mem::size_of::<RGBAu8>();
		let full_triangle_pixel = (extent.width() * (extent.height() - 2) + extent.width() / 2) as usize;
		let full_triangle_byte = full_triangle_pixel * std::mem::size_of::<RGBAu8>();

		assert_eq!(
			&image_data[center_byte..center_byte + std::mem::size_of::<RGBAu8>()],
			&[0, 0, 255, 255],
			"Unexpected center color after disabling depth writes. The most likely cause is that Metal replaced retained depth or stopped depth-testing the no-write pipeline.",
		);
		assert_eq!(
			&image_data[full_triangle_byte..full_triangle_byte + std::mem::size_of::<RGBAu8>()],
			&[0, 255, 0, 255],
			"Unexpected color outside the final triangle. The most likely cause is that the visible no-write draw was skipped or the behind draw bypassed depth testing.",
		);
	}

	pub(crate) fn present(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1921, 1080);

		let mut window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent, Uses::RenderTarget);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			renderer.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let attachments = [AttachmentDescriptor::new(Formats::BGRAsRGB)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for _ in window.poll() {}

		renderer.start_frame_capture();

		{
			let mut queue = renderer.queue(queue_handle);
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: render_finished_synchronizer,
				}),
				&[],
				render_finished_synchronizer,
				|execution| {
					let (present_key, _) = execution.frame().unwrap().acquire_swapchain_image(swapchain);
					let present_keys = [present_key];

					execution.record(command_buffer_handle, |command_buffer_recording| {
						let attachments = [AttachmentInformation::new(
							swapchain,
							Layouts::RenderTarget,
							ClearValue::Color(RGBA::black()),
							false,
							true,
						)];

						let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

						let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

						raster_pipeline_command.draw_mesh(&mesh);

						render_pass_command.end_render_pass();
					});

					present_keys
				},
			);
		}

		renderer.end_frame_capture();

		for _ in window.poll() {}

		// TODO: assert rendering results

		assert!(!renderer.has_errors())
	}

	pub(crate) fn multiframe_present(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let window = Window::new("Present Test", extent).expect("Failed to create window");

		let os_handles = window.os_handles();

		let swapchain = renderer.bind_to_window(&os_handles, Default::default(), extent, Uses::RenderTarget);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			renderer.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = renderer
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = renderer
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let attachments = [AttachmentDescriptor::new(Formats::BGRAsRGB)];

		let pipeline = renderer.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		for i in 0..2 * 64 {
			renderer.start_frame_capture();

			{
				let mut queue = renderer.queue(queue_handle);
				queue.execute(
					Some(FrameRequest {
						index: i,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let (present_key, _) = execution.frame().unwrap().acquire_swapchain_image(swapchain);
						let present_keys = [present_key];

						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								swapchain,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA {
									r: 0.0,
									g: 0.0,
									b: 0.0,
									a: 1.0,
								}),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();
						});

						present_keys
					},
				);
			}

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());
		}
	}

	pub(crate) fn multiframe_rendering(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// Use and odd width to make sure there is a middle/center pixel
		let _extent = Extent::rectangle(1920, 1080);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[PushConstantRange::new(0, 16 * 4)],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn change_frames(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while changing the amount of frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			if i == 2 {
				device.set_frames_in_flight(3); // Change from default 2 to 3
			}

			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn resize(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering while resize the render targets.

		const FRAMES_IN_FLIGHT: usize = 3;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		let mut extent = Extent::rectangle(1280, 720);

		let render_target = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();

				queue.execute(
					Some(FrameRequest {
						index: i as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let frame = execution.frame().unwrap();

						if i == 2 {
							extent = Extent::rectangle(1920, 1080);
							frame.resize_image(render_target.into(), extent);
						}

						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

							let raster_pipeline_command = render_pass_command.bind_raster_pipeline(pipeline);

							raster_pipeline_command.draw_mesh(&mesh);

							raster_pipeline_command.end_render_pass();

							texture_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let image_data = device.get_image_data(texture_copy_handles[0]);
			let pixel_count = (extent.width() * extent.height()) as usize;
			assert_eq!(
				image_data.len(),
				pixel_count * std::mem::size_of::<RGBAu8>(),
				"Render-target readback size does not match its resized extent. The most likely cause is that one frame-local image kept its previous extent."
			);
			let pixels = unsafe {
				// RGBA8 readback stores one tightly packed RGBAu8 value per pixel.
				std::slice::from_raw_parts(image_data.as_ptr() as *const RGBAu8, pixel_count)
			};

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			check_triangle(pixels, extent);
		}
	}

	pub(crate) fn dynamic_data(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders_with_model_matrix();

		let vertex_shader = device
			.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
			.expect("Failed to create fragment shader");

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[PushConstantRange::new(0, 16 * 4)],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let _buffer =
			device.build_buffer::<u8>(crate::buffer::Builder::new(Uses::Storage).device_accesses(DeviceAccesses::HostToDevice));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = device.create_synchronizer(None, true);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			device.start_frame_capture();

			let copy_texture_handles = {
				let mut queue = device.queue(queue_handle);
				let mut copy_texture_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(command_buffer_handle, |command_buffer_recording| {
							let attachments = [AttachmentInformation::new(
								render_target,
								Layouts::RenderTarget,
								ClearValue::Color(RGBA::black()),
								false,
								true,
							)];

							let c = command_buffer_recording.start_render_pass(extent, &attachments);

							let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);

							let matrix: [f32; 16] = [
								angle.cos(),
								-angle.sin(),
								0f32,
								0f32,
								angle.sin(),
								angle.cos(),
								0f32,
								0f32,
								0f32,
								0f32,
								1f32,
								0f32,
								0f32,
								0f32,
								0f32,
								1f32,
							];

							let c = c.bind_raster_pipeline(pipeline);

							c.write_push_constant(0, matrix);
							c.draw_mesh(&mesh);

							c.end_render_pass();

							copy_texture_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				copy_texture_handles
			};

			device.end_frame_capture();

			device.wait();

			assert!(!device.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(copy_texture_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

			// Track green corner as it should move through screen

			if i % 4 == 0 {
				let pixel = pixels[(extent.width() * extent.height() - 1) as usize]; // bottom right
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at bottom right corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 1 {
				let pixel = pixels[(extent.width() * (extent.height() - 1)) as usize]; // bottom left
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at bottom left corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 2 {
				let pixel = pixels[0]; // top left
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at top left corner did not match expected green color in frame: {i}"
				);
			} else if i % 4 == 3 {
				let pixel = pixels[(extent.width() - 1) as usize]; // top right
				assert_eq!(
					pixel,
					RGBAu8 {
						r: 0,
						g: 255,
						b: 0,
						a: 255
					},
					"Pixel at top right corner did not match expected green color in frame: {i}"
				);
			}
		}

		assert!(!device.has_errors())
	}

	pub(crate) fn dynamic_textures(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that dynamic textures write to the current frame image instead of always writing to the root image.

		let extent = Extent::square(2);
		let pixel_count = (extent.width() * extent.height()) as usize;

		let upload_image = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferSource)
				.extent(extent)
				.device_accesses(DeviceAccesses::HostToDevice),
		);

		let readback_image = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferDestination)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost),
		);

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);
		let render_finished_synchronizer = device.create_synchronizer(None, true);

		let expected_colors = [
			RGBAu8 {
				r: 255,
				g: 0,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255,
			},
		];

		for (frame_index, expected_color) in expected_colors.into_iter().enumerate() {
			let texture_copy_handles = {
				let mut queue = device.queue(queue_handle);
				let mut texture_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: frame_index as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						let frame = execution.frame().unwrap();

						let texture_slice = frame.get_mut_dynamic_texture_slice(upload_image.into());
						let pixels =
							unsafe { std::slice::from_raw_parts_mut(texture_slice.as_mut_ptr() as *mut RGBAu8, pixel_count) };
						pixels.fill(expected_color);
						frame.sync_texture(upload_image.into());

						execution.record(command_buffer_handle, |command_buffer_recording| {
							command_buffer_recording.blit_image(
								upload_image.into(),
								Layouts::Transfer,
								readback_image.into(),
								Layouts::Transfer,
							);
							texture_copy_handles = command_buffer_recording.transfer_textures(&[readback_image.into()]);
						});
						[]
					},
				);
				texture_copy_handles
			};

			device.wait();

			let pixels = unsafe {
				std::slice::from_raw_parts(
					device.get_image_data(texture_copy_handles[0]).as_ptr() as *const RGBAu8,
					pixel_count,
				)
			};

			assert!(pixels.iter().all(|pixel| *pixel == expected_color));
			assert!(!device.has_errors());
		}
	}

	pub(crate) fn multiframe_resources(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests frame-local image creation, previous-frame bindings, and sequence wraparound.

		// TODO: test multiframe resources for combined image samplers
		let compute_shader_string = "
			#version 450
			#pragma shader_stage(compute)

			layout(set=0,binding=0, rgba8) uniform image2D img;
			layout(set=0,binding=1, rgba8) uniform readonly image2D last_frame_img;

			layout(push_constant) uniform PushConstants {
				float value;
			} push_constants;

			layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
			void main() {
				imageStore(img, ivec2(0, 0), vec4(vec3(push_constants.value), 1));
				imageStore(img, ivec2(1, 0), imageLoad(last_frame_img, ivec2(0, 0)));
			}
		";
		let compute_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct Resources {
				texture2d<float, access::write> image [[id(0)]];
				texture2d<float, access::read> last_frame_image [[id(2)]];
			};
			kernel void compute_main(
				uint2 gid [[thread_position_in_grid]],
				constant Resources& resources [[buffer(16)]],
				constant float& value [[buffer(15)]]) {
				resources.image.write(float4(value, value, value, 1.0), uint2(0, 0));
				resources.image.write(resources.last_frame_image.read(uint2(0, 0)), uint2(1, 0));
			}
		"#;
		let compute_shader_hlsl = r#"
			RWTexture2D<float4> image : register(u0, space0);
			RWTexture2D<float4> last_frame_image : register(u1, space0);
			struct PushConstant { float value; };
			ConstantBuffer<PushConstant> push_constant : register(b0, space0);
			[numthreads(1, 1, 1)]
			void compute_main(uint3 gid : SV_DispatchThreadID) {
				image[uint2(0, 0)] = float4(push_constant.value.xxx, 1.0);
				image[uint2(1, 0)] = last_frame_image[uint2(0, 0)];
			}
		"#;
		let compute_shader_artifact = crate::shader::compile(
			"GHI multiframe resource test compute shader",
			ShaderSource::PlatformNative {
				glsl: compute_shader_string,
				msl: compute_shader_msl,
				msl_entry_point: "compute_main",
				hlsl: compute_shader_hlsl,
				hlsl_entry_point: "compute_main",
			},
		)
		.expect("Failed to compile the multiframe resource shader. The most likely cause is invalid native shader source.");
		let image_resource = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(0),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let last_frame_image_resource = crate::shader::ShaderResourceDescriptor::single(
			crate::shader::ResourceSlot::new(1),
			crate::shader::ResourceKind::StorageImage,
			crate::AccessPolicies::READ,
		);

		let compute_shader = device
			.create_shader(
				None,
				compute_shader_artifact.as_source(),
				ShaderTypes::Compute,
				[image_resource, last_frame_image_resource],
			)
			.expect("Failed to create compute shader");

		let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
			&[PushConstantRange { offset: 0, size: 4 }],
			ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
		));

		let image = device.build_dynamic_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
				.name("Image")
				.extent(Extent::square(2))
				.device_accesses(DeviceAccesses::DeviceToHost),
		);

		let descriptor_set = device.create_descriptor_set(None);
		device.write(&[
			crate::DescriptorWrite::image(descriptor_set, image_resource.slot(), image, Layouts::General),
			crate::DescriptorWrite::image_with_frame(
				descriptor_set,
				last_frame_image_resource.slot(),
				image,
				Layouts::General,
				-1,
			),
		]);

		let command_buffer = device.queue(queue_handle).create_command_buffer(None);

		let signal = device.create_synchronizer(None, true);

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						let data = [0.5f32];

						let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

						pipeline_command.write_push_constant(0, data);
						pipeline_command
							.bind_descriptor_sets(&[descriptor_set])
							.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };
		assert!(
			pixels[0]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[0]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		); // Current frame image
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 }); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 1,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						let data = [1.0f32];

						let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

						pipeline_command.write_push_constant(0, data);
						pipeline_command
							.bind_descriptor_sets(&[descriptor_set])
							.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert_eq!(
			pixels[0],
			RGBAu8 {
				r: 255,
				g: 255,
				b: 255,
				a: 255
			}
		);
		assert!(
			pixels[1]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[1]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		); // Current frame sample from last frame image

		assert!(!device.has_errors());

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 2,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };
		assert_eq!(pixels[0], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });

		assert!(!device.has_errors());

		let copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 3,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer, |command_buffer_recording| {
						copy_handles = command_buffer_recording.transfer_textures(&[image.into()]);
					});
					[]
				},
			);
			copy_handles
		};

		device.wait();

		let pixels = unsafe { std::slice::from_raw_parts(device.get_image_data(copy_handles[0]).as_ptr() as *const RGBAu8, 4) };

		assert!(
			pixels[0]
				== RGBAu8 {
					r: 127,
					g: 127,
					b: 127,
					a: 255
				} || pixels[0]
				== RGBAu8 {
					r: 128,
					g: 128,
					b: 128,
					a: 255
				}
		);
		assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });

		assert!(!device.has_errors());
	}

	pub(crate) fn descriptor_sets(device: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		let signal = device.create_synchronizer(None, true);

		let floats: [f32; 21] = [
			0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
		];

		let vertex_layout = [
			VertexElement::new("POSITION", DataTypes::Float3, 0),
			VertexElement::new("COLOR", DataTypes::Float4, 0),
		];

		let mesh = unsafe {
			device.add_mesh_from_vertices_and_indices(
				3,
				3,
				std::slice::from_raw_parts(floats.as_ptr() as *const u8, (3 * 4 + 4 * 4) * 3),
				std::slice::from_raw_parts([0u16, 1u16, 2u16].as_ptr() as *const u8, 3 * 2),
				&vertex_layout,
			)
		};

		let vertex_shader_code = "
			#version 450 core
			#pragma shader_stage(vertex)

			layout(location = 0) in vec3 in_position;
			layout(location = 1) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0, binding=1) uniform UniformBufferObject {
				mat4 matrix;
			} ubo;

			void main() {
				out_color = in_color;
				gl_Position = vec4(in_position, 1.0);
			}
		";

		let fragment_shader_code = "
			#version 450 core
			#pragma shader_stage(fragment)

			layout(location = 0) in vec4 in_color;

			layout(location = 0) out vec4 out_color;

			layout(set=0,binding=0) uniform sampler2D tex;

			void main() {
				out_color = texture(tex, vec2(0, 0));
			}
		";
		let vertex_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct VertexResources { constant float4x4* matrix [[id(2)]]; };
			struct VertexInput {
				float3 position [[attribute(0)]];
				float4 color [[attribute(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			vertex VertexOutput besl_main(
				VertexInput input [[stage_in]],
				constant VertexResources& resources [[buffer(16)]]) {
				return VertexOutput { resources.matrix[0] * float4(input.position, 1.0), input.color };
			}
		"#;
		let fragment_shader_msl = r#"
			#include <metal_stdlib>
			using namespace metal;
			struct FragmentResources {
				texture2d<float> texture [[id(0)]];
				sampler texture_sampler [[id(1)]];
			};
			struct VertexOutput {
				float4 position [[position]];
				float4 color;
			};
			fragment float4 besl_main(
				VertexOutput input [[stage_in]],
				constant FragmentResources& resources [[buffer(16)]]) {
				return resources.texture.sample(resources.texture_sampler, float2(0.0));
			}
		"#;
		let vertex_shader_hlsl = r#"
			StructuredBuffer<float4x4> matrices : register(t1, space0);
			struct VertexInput { float3 position : POSITION; float4 color : COLOR0; };
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			VertexOutput vertex_main(VertexInput input) {
				VertexOutput output;
				output.position = mul(matrices[0], float4(input.position, 1.0));
				output.color = input.color;
				return output;
			}
		"#;
		let fragment_shader_hlsl = r#"
			SamplerState texture_sampler : register(s0, space0);
			Texture2D<float4> texture_image : register(t0, space0);
			struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
			float4 fragment_main(VertexOutput input) : SV_TARGET0 {
				return texture_image.Sample(texture_sampler, float2(0.0, 0.0));
			}
		"#;
		let vertex_shader_artifact = crate::shader::compile(
			"GHI descriptor test vertex shader",
			ShaderSource::PlatformNative {
				glsl: vertex_shader_code,
				msl: vertex_shader_msl,
				msl_entry_point: "besl_main",
				hlsl: vertex_shader_hlsl,
				hlsl_entry_point: "vertex_main",
			},
		)
		.expect("Failed to compile the descriptor test vertex shader. The most likely cause is invalid native shader source.");
		let fragment_shader_artifact = crate::shader::compile(
			"GHI descriptor test fragment shader",
			ShaderSource::PlatformNative {
				glsl: fragment_shader_code,
				msl: fragment_shader_msl,
				msl_entry_point: "besl_main",
				hlsl: fragment_shader_hlsl,
				hlsl_entry_point: "fragment_main",
			},
		)
		.expect(
			"Failed to compile the descriptor test fragment shader. The most likely cause is invalid native shader source.",
		);

		let buffer_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(1),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let texture_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(0),
			crate::ResourceKind::CombinedImageSampler,
			crate::AccessPolicies::READ,
		);

		let vertex_shader = device
			.create_shader(
				None,
				vertex_shader_artifact.as_source(),
				ShaderTypes::Vertex,
				[buffer_resource],
			)
			.expect("Failed to create vertex shader");
		let fragment_shader = device
			.create_shader(
				None,
				fragment_shader_artifact.as_source(),
				ShaderTypes::Fragment,
				[texture_resource],
			)
			.expect("Failed to create fragment shader");

		let buffer = device.build_dynamic_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::Uniform | Uses::Storage).device_accesses(DeviceAccesses::HostToDevice),
		);

		let sampled_texture = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Image)
				.name("sampled texture")
				.extent(Extent::square(2))
				.device_accesses(DeviceAccesses::HostToDevice)
				.use_case(UseCases::STATIC),
		);

		let pixels = vec![
			RGBAu8 {
				r: 255,
				g: 0,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 255,
				b: 0,
				a: 255,
			},
			RGBAu8 {
				r: 0,
				g: 0,
				b: 255,
				a: 255,
			},
			RGBAu8 {
				r: 255,
				g: 255,
				b: 0,
				a: 255,
			},
		];

		let sampler = device.build_sampler(
			crate::sampler::Builder::new()
				.filtering_mode(FilteringModes::Closest)
				.reduction_mode(SamplingReductionModes::WeightedAverage)
				.mip_map_mode(FilteringModes::Closest)
				.addressing_mode(SamplerAddressingModes::Repeat)
				.min_lod(0.0f32)
				.max_lod(0.0f32),
		);

		let descriptor_set = device.create_descriptor_set(None);
		device.write(&[
			crate::DescriptorWrite::combined_image_sampler(
				descriptor_set,
				texture_resource.slot(),
				sampled_texture,
				sampler,
				Layouts::Read,
			),
			crate::DescriptorWrite::buffer(descriptor_set, buffer_resource.slot(), buffer.into()),
		]);

		assert!(!device.has_errors());

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		let render_target = device.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::STATIC),
		);

		let attachments = [AttachmentDescriptor::new(Formats::RGBA8UNORM)];

		let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
			&[],
			&vertex_layout,
			&[
				ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
				ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
			],
			&attachments,
		));

		let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

		device.start_frame_capture();

		let texure_copy_handles = {
			let mut queue = device.queue(queue_handle);
			let mut texure_copy_handles = Vec::new();
			queue.execute(
				Some(FrameRequest {
					index: 0,
					synchronizer: signal,
				}),
				&[],
				signal,
				|execution| {
					execution.record(command_buffer_handle, |command_buffer_recording| {
						command_buffer_recording.write_image_data(sampled_texture.into(), &pixels);

						let attachments = [AttachmentInformation::new(
							render_target,
							Layouts::RenderTarget,
							ClearValue::Color(RGBA {
								r: 0.0,
								g: 0.0,
								b: 0.0,
								a: 1.0,
							}),
							false,
							true,
						)];

						let raster_render_pass_command = command_buffer_recording.start_render_pass(extent, &attachments);

						let raster_pipeline_command = raster_render_pass_command.bind_raster_pipeline(pipeline);

						raster_pipeline_command.bind_descriptor_sets(&[descriptor_set]);

						raster_pipeline_command.draw_mesh(&mesh);

						raster_render_pass_command.end_render_pass();

						texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
					});
					[]
				},
			);
			texure_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		// assert colored triangle was drawn to texture
		let _pixels = device.get_image_data(texure_copy_handles[0]);

		// TODO: assert rendering results

		assert!(!device.has_errors());
	}

	pub(crate) fn ray_tracing(renderer: &mut impl crate::context::Context, queue_handle: QueueHandle) {
		//! Tests that the render system can perform rendering with multiple frames in flight.
		//! Having multiple frames in flight means allocating and managing multiple resources under a single handle, one for each frame.

		const FRAMES_IN_FLIGHT: usize = 2;

		// let mut window_system = window_system::WindowSystem::new();

		// Use and odd width to make sure there is a middle/center pixel
		let extent = Extent::rectangle(1920, 1080);

		// let window_handle = window_system.create_window("Renderer Test", extent, "test");
		// let swapchain = renderer.bind_to_window(&window_system.get_os_handles_2(&window_handle));

		let positions: [f32; 3 * 3] = [0.0, 1.0, 0.0, 1.0, -1.0, 0.0, -1.0, -1.0, 0.0];

		let colors: [f32; 4 * 3] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0];

		let vertex_positions_buffer = renderer.build_buffer::<[f32; 3 * 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		let vertex_colors_buffer = renderer.build_buffer::<[f32; 4 * 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);
		let index_buffer = renderer.build_buffer::<[u16; 3]>(
			crate::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
				.device_accesses(DeviceAccesses::HostToDevice),
		);

		renderer
			.get_mut_buffer_slice(vertex_positions_buffer)
			.copy_from_slice(&positions);
		renderer.get_mut_buffer_slice(vertex_colors_buffer).copy_from_slice(&colors);
		renderer
			.get_mut_buffer_slice(index_buffer)
			.copy_from_slice(&[0u16, 1u16, 2u16]);

		renderer.sync_buffer(vertex_positions_buffer);
		renderer.sync_buffer(index_buffer);

		let raygen_shader_code = "
#version 460 core
#pragma shader_stage(raygen)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(binding = 0, set = 0) uniform accelerationStructureEXT topLevelAS;
layout(binding = 1, set = 0, rgba8) uniform image2D image;

layout(location = 0) rayPayloadEXT vec3 hitValue;

void main() {
	const vec2 pixelCenter = vec2(gl_LaunchIDEXT.xy) + vec2(0.5);
	const vec2 inUV = pixelCenter/vec2(gl_LaunchSizeEXT.xy);
	vec2 d = inUV * 2.0 - 1.0;
	d.y *= -1.0;

	uint rayFlags = gl_RayFlagsOpaqueEXT;
	uint cullMask = 0xff;
	float tmin = 0.001;
	float tmax = 10.0;

	vec3 origin = vec3(d, -1.0);
	vec3 direction = vec3(0.0, 0.0, 1.0);

	traceRayEXT(topLevelAS, rayFlags, cullMask, 0, 0, 0, origin, tmin, direction, tmax, 0);

	imageStore(image, ivec2(gl_LaunchIDEXT.xy), vec4(hitValue, 1.0));
}
		";

		let closest_hit_shader_code = "
#version 460 core
#pragma shader_stage(closest)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;
hitAttributeEXT vec2 attribs;

layout(binding = 2, set = 0) buffer VertexPositions { vec3 positions[3]; };
layout(binding = 3, set = 0) buffer VertexColors { vec4 colors[3]; };
layout(binding = 4, set = 0) buffer Indices { uint16_t indices[3]; };

void main() {
	const vec3 barycentricCoords = vec3(1.0f - attribs.x - attribs.y, attribs.x, attribs.y);
	ivec3 index = ivec3(indices[3 * gl_PrimitiveID], indices[3 * gl_PrimitiveID + 1], indices[3 * gl_PrimitiveID + 2]);

	vec3[3] vertex_positions = vec3[3](positions[index.x], positions[index.y], positions[index.z]);
	vec4[3] vertex_colors = vec4[3](colors[index.x], colors[index.y], colors[index.z]);

	vec3 position = vertex_positions[0] * barycentricCoords.x + vertex_positions[1] * barycentricCoords.y + vertex_positions[2] * barycentricCoords.z;
	vec4 color = vertex_colors[0] * barycentricCoords.x + vertex_colors[1] * barycentricCoords.y + vertex_colors[2] * barycentricCoords.z;

	hitValue = color.xyz;
}
		";

		let miss_shader_code = "
#version 460 core
#pragma shader_stage(miss)

#extension GL_EXT_scalar_block_layout: enable
#extension GL_EXT_buffer_reference: enable
#extension GL_EXT_buffer_reference2: enable
#extension GL_EXT_shader_16bit_storage: require
#extension GL_EXT_ray_tracing: require

layout(location = 0) rayPayloadInEXT vec3 hitValue;

void main() {
    hitValue = vec3(0.0, 0.0, 0.0);
}
		";

		// Metal ray tracing execution is still intentionally ignored, but native source keeps this shared test portable.
		let raygen_shader_artifact = crate::shader::compile(
			"GHI ray generation test shader",
			ShaderSource::PlatformNative {
				glsl: raygen_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void raygen_main() {}",
				msl_entry_point: "raygen_main",
				hlsl: r#"
struct Payload {
	float3 hit_value;
};

RaytracingAccelerationStructure top_level_as : register(t0, space0);
RWTexture2D<float4> output_image : register(u1, space0);

[shader("raygeneration")]
void raygen_main() {
	uint2 launch_id = DispatchRaysIndex().xy;
	uint2 launch_size = DispatchRaysDimensions().xy;
	float2 pixel_center = float2(launch_id) + float2(0.5, 0.5);
	float2 in_uv = pixel_center / float2(launch_size);
	float2 direction_xy = in_uv * 2.0 - 1.0;
	direction_xy.y *= -1.0;
	direction_xy = lerp(direction_xy, float2(0.0, -0.33333334), 0.001);

	RayDesc ray;
	ray.Origin = float3(direction_xy, -1.0);
	ray.TMin = 0.001;
	ray.Direction = float3(0.0, 0.0, 1.0);
	ray.TMax = 10.0;

	Payload payload;
	payload.hit_value = float3(0.0, 0.0, 0.0);
	TraceRay(top_level_as, RAY_FLAG_FORCE_OPAQUE, 0xff, 0, 1, 0, ray, payload);

	output_image[launch_id] = float4(payload.hit_value, 1.0);
}
"#,
				hlsl_entry_point: "raygen_main",
			},
		)
		.expect("Failed to compile the ray generation test shader. The most likely cause is invalid native shader source.");
		let closest_hit_shader_artifact = crate::shader::compile(
			"GHI closest-hit test shader",
			ShaderSource::PlatformNative {
				glsl: closest_hit_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void closest_hit_main() {}",
				msl_entry_point: "closest_hit_main",
				hlsl: r#"
struct Payload {
	float3 hit_value;
};

StructuredBuffer<float3> positions : register(t2, space0);
StructuredBuffer<float4> colors : register(t3, space0);

[shader("closesthit")]
void closest_hit_main(inout Payload payload, in BuiltInTriangleIntersectionAttributes attributes) {
	float3 barycentric = float3(
		1.0 - attributes.barycentrics.x - attributes.barycentrics.y,
		attributes.barycentrics.x,
		attributes.barycentrics.y
	);
	float4 color = colors[0] * barycentric.x + colors[1] * barycentric.y + colors[2] * barycentric.z;
	payload.hit_value = color.xyz;
}
"#,
				hlsl_entry_point: "closest_hit_main",
			},
		)
		.expect("Failed to compile the closest-hit test shader. The most likely cause is invalid native shader source.");
		let miss_shader_artifact = crate::shader::compile(
			"GHI miss test shader",
			ShaderSource::PlatformNative {
				glsl: miss_shader_code,
				msl: "#include <metal_stdlib>\nusing namespace metal; kernel void miss_main() {}",
				msl_entry_point: "miss_main",
				hlsl: r#"
struct Payload {
	float3 hit_value;
};

[shader("miss")]
void miss_main(inout Payload payload) {
	payload.hit_value = float3(0.0, 0.0, 0.0);
}
"#,
				hlsl_entry_point: "miss_main",
			},
		)
		.expect("Failed to compile the miss test shader. The most likely cause is invalid native shader source.");
		let acceleration_structure_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(0),
			crate::ResourceKind::AccelerationStructure,
			crate::AccessPolicies::READ,
		);
		let output_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(1),
			crate::ResourceKind::StorageImage,
			crate::AccessPolicies::WRITE,
		);
		let position_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(2),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let color_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(3),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);
		let index_resource = crate::ShaderResourceDescriptor::single(
			crate::ResourceSlot::new(4),
			crate::ResourceKind::StorageBuffer,
			crate::AccessPolicies::READ,
		);

		let raygen_shader = renderer
			.create_shader(
				None,
				raygen_shader_artifact.as_source(),
				ShaderTypes::RayGen,
				[acceleration_structure_resource, output_resource],
			)
			.expect("Failed to create raygen shader");
		let closest_hit_shader = renderer
			.create_shader(
				None,
				closest_hit_shader_artifact.as_source(),
				ShaderTypes::ClosestHit,
				[position_resource, color_resource, index_resource],
			)
			.expect("Failed to create closest hit shader");
		let miss_shader = renderer
			.create_shader(None, miss_shader_artifact.as_source(), ShaderTypes::Miss, [])
			.expect("Failed to create miss shader");

		let top_level_acceleration_structure = renderer.create_top_level_acceleration_structure(Some("Top Level"), 1);
		let bottom_level_acceleration_structure =
			renderer.create_bottom_level_acceleration_structure(&BottomLevelAccelerationStructure {
				description: BottomLevelAccelerationStructureDescriptions::Mesh {
					vertex_count: 3,
					vertex_position_encoding: Encodings::FloatingPoint,
					triangle_count: 1,
					index_format: DataTypes::U16,
				},
			});

		let descriptor_set = renderer.create_descriptor_set(None);

		let render_target = renderer.build_image(
			crate::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage)
				.extent(extent)
				.device_accesses(DeviceAccesses::DeviceToHost)
				.use_case(UseCases::DYNAMIC),
		);

		renderer.write(&[
			crate::DescriptorWrite::acceleration_structure(
				descriptor_set,
				acceleration_structure_resource.slot(),
				top_level_acceleration_structure,
			),
			crate::DescriptorWrite::image(descriptor_set, output_resource.slot(), render_target, Layouts::General),
			crate::DescriptorWrite::buffer(descriptor_set, position_resource.slot(), vertex_positions_buffer.into()),
			crate::DescriptorWrite::buffer(descriptor_set, color_resource.slot(), vertex_colors_buffer.into()),
			crate::DescriptorWrite::buffer(descriptor_set, index_resource.slot(), index_buffer.into()),
		]);

		let pipeline = renderer.create_ray_tracing_pipeline(pipelines::ray_tracing::Builder::new(
			&[],
			&[
				ShaderParameter::new(&raygen_shader, ShaderTypes::RayGen),
				ShaderParameter::new(&closest_hit_shader, ShaderTypes::ClosestHit),
				ShaderParameter::new(&miss_shader, ShaderTypes::Miss),
			],
		));

		let rendering_command_buffer_handle = renderer.queue(queue_handle).create_command_buffer(None);

		let render_finished_synchronizer = renderer.create_synchronizer(None, true);

		let instances_buffer = renderer.create_acceleration_structure_instance_buffer(None, 1);

		renderer.write_instance(
			instances_buffer,
			0,
			[[1f32, 0f32, 0f32, 0f32], [0f32, 1f32, 0f32, 0f32], [0f32, 0f32, 1f32, 0f32]],
			0,
			0xFF,
			0,
			bottom_level_acceleration_structure,
		);

		let scratch_buffer = renderer.build_buffer::<[u8; 1024 * 1024]>(
			crate::buffer::Builder::new(Uses::AccelerationStructureBuildScratch).device_accesses(DeviceAccesses::DeviceOnly),
		);

		let raygen_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);
		let miss_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);
		let hit_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
			crate::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
		);

		renderer.write_sbt_entry(raygen_sbt_buffer.into(), 0, pipeline, raygen_shader);
		renderer.write_sbt_entry(miss_sbt_buffer.into(), 0, pipeline, miss_shader);
		renderer.write_sbt_entry(hit_sbt_buffer.into(), 0, pipeline, closest_hit_shader);

		for i in 0..FRAMES_IN_FLIGHT * 10 {
			renderer.start_frame_capture();

			let texure_copy_handles = {
				let mut queue = renderer.queue(queue_handle);
				let mut texure_copy_handles = Vec::new();
				queue.execute(
					Some(FrameRequest {
						index: i as u64,
						synchronizer: render_finished_synchronizer,
					}),
					&[],
					render_finished_synchronizer,
					|execution| {
						execution.record(rendering_command_buffer_handle, |command_buffer_recording| {
							{
								command_buffer_recording.build_bottom_level_acceleration_structures(&[
									BottomLevelAccelerationStructureBuild {
										acceleration_structure: bottom_level_acceleration_structure,
										description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
											vertex_buffer: BufferStridedRange::new(
												vertex_positions_buffer.into(),
												0,
												12,
												12 * 3,
											),
											vertex_count: 3,
											index_buffer: BufferStridedRange::new(index_buffer.into(), 0, 2, 2 * 3),
											vertex_position_encoding: Encodings::FloatingPoint,
											index_format: DataTypes::U16,
											triangle_count: 1,
										},
										scratch_buffer: BufferDescriptor::new(scratch_buffer),
									},
								]);

								command_buffer_recording.build_top_level_acceleration_structure(
									&TopLevelAccelerationStructureBuild {
										acceleration_structure: top_level_acceleration_structure,
										description: TopLevelAccelerationStructureBuildDescriptions::Instance {
											instances_buffer,
											instance_count: 1,
										},
										scratch_buffer: BufferDescriptor::new(scratch_buffer),
									},
								);
							}

							let ray_tracing_pipeline_command = command_buffer_recording.bind_ray_tracing_pipeline(pipeline);

							ray_tracing_pipeline_command.bind_descriptor_sets(&[descriptor_set]);

							ray_tracing_pipeline_command.trace_rays(
								BindingTables {
									raygen: BufferStridedRange::new(raygen_sbt_buffer.into(), 0, 64, 64),
									hit: BufferStridedRange::new(hit_sbt_buffer.into(), 0, 64, 64),
									miss: BufferStridedRange::new(miss_sbt_buffer.into(), 0, 64, 64),
									callable: None,
								},
								1920,
								1080,
								1,
							);

							texure_copy_handles = command_buffer_recording.transfer_textures(&[render_target.into()]);
						});
						[]
					},
				);
				texure_copy_handles
			};

			renderer.end_frame_capture();

			assert!(!renderer.has_errors());

			let pixels = unsafe {
				std::slice::from_raw_parts(
					renderer.get_image_data(texure_copy_handles[0]).as_ptr() as *const RGBAu8,
					(extent.width() * extent.height()) as usize,
				)
			};

			check_triangle(pixels, extent);
		}
	}
}
