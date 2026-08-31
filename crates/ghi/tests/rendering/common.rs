use super::*;

/// Views initialized f32 test data as the byte-oriented mesh upload API expects.
pub(super) fn f32_bytes(values: &[f32]) -> &[u8] {
	// SAFETY: f32 has no padding, and the returned view is bounded by the live source slice.
	unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

/// Views initialized u16 test data as the byte-oriented mesh upload API expects.
pub(super) fn u16_bytes(values: &[u16]) -> &[u8] {
	// SAFETY: u16 has no padding, and the returned view is bounded by the live source slice.
	unsafe { std::slice::from_raw_parts(values.as_ptr().cast(), std::mem::size_of_val(values)) }
}

pub(super) fn compile_shaders() -> (CompiledShaderSource, CompiledShaderSource) {
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

	let vertex_shader_artifact = ghi::shader::compile(
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
	let fragment_shader_artifact = ghi::shader::compile(
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

pub(super) fn compile_shaders_with_model_matrix() -> (CompiledShaderSource, CompiledShaderSource) {
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

	let vertex_shader_artifact = ghi::shader::compile(
		"GHI model-matrix test vertex shader",
		ShaderSource::PlatformNative {
			glsl: vertex_shader_code,
			msl: vertex_shader_msl,
			msl_entry_point: "vertex_main",
			hlsl: vertex_shader_hlsl,
			hlsl_entry_point: "vertex_main",
		},
	)
	.expect("Failed to compile GHI model-matrix test vertex shader. The most likely cause is invalid native shader source.");
	let fragment_shader_artifact = ghi::shader::compile(
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

/// Converts one owned RGBA8 readback into test pixels without borrowing temporary storage.
pub(super) fn rgba_pixels(readback: ghi::TextureReadback) -> Vec<RGBAu8> {
	assert_eq!(
		readback.bytes.len() % std::mem::size_of::<RGBAu8>(),
		0,
		"RGBA8 readback size is invalid. The most likely cause is that the transfer layout does not contain complete pixels."
	);
	readback
		.bytes
		.chunks_exact(std::mem::size_of::<RGBAu8>())
		.map(|pixel| RGBAu8 {
			r: pixel[0],
			g: pixel[1],
			b: pixel[2],
			a: pixel[3],
		})
		.collect()
}

pub(super) fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
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
