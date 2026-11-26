use super::*;

pub(super) const BLACK: RGBAu8 = RGBAu8 {
	r: 0,
	g: 0,
	b: 0,
	a: 255,
};
pub(super) const RED: RGBAu8 = RGBAu8 {
	r: 255,
	g: 0,
	b: 0,
	a: 255,
};
pub(super) const GREEN: RGBAu8 = RGBAu8 {
	r: 0,
	g: 255,
	b: 0,
	a: 255,
};
pub(super) const BLUE: RGBAu8 = RGBAu8 {
	r: 0,
	g: 0,
	b: 255,
	a: 255,
};

/// The reference triangle: the position and color of its red top, green bottom-right, and blue bottom-left corners.
pub(super) const TRIANGLE: [f32; 21] = [
	0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
];

/// The vertex layout of every mesh the raster scenarios draw: a position and a color.
pub(super) const VERTEX_LAYOUT: [VertexElement<'static>; 2] = [
	VertexElement::new("POSITION", DataTypes::Float3, 0),
	VertexElement::new("COLOR", DataTypes::Float4, 0),
];

/// The `Frames` struct holds what every frame of a scenario submits through: its queue, one reused command buffer, and
/// the synchronizer each frame signals. Record a frame with [`record_frame!`].
#[derive(Clone, Copy)]
pub(super) struct Frames {
	pub(super) queue: QueueHandle,
	pub(super) commands: ghi::CommandBufferHandle,
	pub(super) synchronizer: ghi::SynchronizerHandle,
}

impl Frames {
	pub(super) fn new(device: &mut impl ghi::context::Context, queue: QueueHandle) -> Self {
		let commands = device.queue(queue).create_command_buffer(None);
		Self {
			queue,
			commands,
			synchronizer: device.create_synchronizer(None, true),
		}
	}
}

/// Returns a one-copy buffer's contents, for tests that write inputs before submitting and read results after waiting.
pub(super) fn buffer_contents<'a, T: ?Sized + ghi::buffer::BufferContents>(
	context: &'a mut impl ghi::context::Context,
	buffer: ghi::BufferHandle<T>,
) -> &'a mut T {
	// SAFETY: Tests call this only while no submitted command uses the buffer, and drop the view before mapping again.
	let (address, byte_count) = unsafe { context.map_unsynchronized(buffer) }.into_raw_parts();
	// SAFETY: The mapping covers the buffer's whole typed allocation.
	unsafe { &mut *T::from_raw_parts(address as *mut u8, byte_count).expect("A mapped test buffer holds its type.") }
}

/// Compiles `source` for the backend under test and creates a shader from it.
pub(super) fn shader(
	device: &mut impl ghi::context::Context,
	name: &str,
	stage: ShaderTypes,
	source: ShaderSource,
	sets: impl IntoIterator<Item = ghi::ShaderDescriptorSet>,
) -> ShaderHandle {
	let compiled =
		ghi::shader::compile(name, source).unwrap_or_else(|error| panic!("Failed to compile shader {name:?}: {error}"));
	device
		.create_shader(Some(name), compiled.as_source(), stage, sets)
		.unwrap_or_else(|_| panic!("Failed to create shader {name:?}. The most likely cause is invalid native shader source."))
}

/// Uploads a three-vertex mesh in [`VERTEX_LAYOUT`].
pub(super) fn mesh(device: &mut impl ghi::context::Context, vertices: &[f32; 21]) -> MeshHandle {
	device.add_mesh_from_vertices_and_indices(
		3,
		3,
		bytemuck::cast_slice(vertices),
		bytemuck::cast_slice(&[0u16, 1, 2]),
		&VERTEX_LAYOUT,
	)
}

/// Creates the vertex and fragment shaders that pass each vertex's color through. With `model_matrix`, the vertex
/// shader transforms positions by a matrix pushed at offset 0.
pub(super) fn triangle_shaders(device: &mut impl ghi::context::Context, model_matrix: bool) -> (ShaderHandle, ShaderHandle) {
	const MSL_TYPES: &str = r#"
		#include <metal_stdlib>
		using namespace metal;
		struct VertexInput { float3 position [[attribute(0)]]; float4 color [[attribute(1)]]; };
		struct VertexOutput { float4 position [[position]]; float4 color; };
	"#;
	const HLSL_TYPES: &str = r#"
		struct VertexInput { float3 position : TEXCOORD0; float4 color : TEXCOORD1; };
		struct VertexOutput { float4 position : SV_POSITION; float4 color : COLOR0; };
	"#;
	// Each language declares the matrix only when the pipeline pushes one. In GLSL, push constants start after the 128
	// bytes of descriptor set base indices.
	let (glsl_matrix, glsl_product, msl_matrix, msl_product, hlsl_matrix, hlsl_position) = if model_matrix {
		(
			"layout(push_constant) uniform ModelMatrix { layout(offset = 128) mat4 model_matrix; } push_constants;",
			"push_constants.model_matrix * ",
			", constant float4x4& model_matrix [[buffer(15)]]",
			"model_matrix * ",
			"struct PushConstant { float4x4 model_matrix; }; ConstantBuffer<PushConstant> push_constant : register(b0, space0);",
			"mul(push_constant.model_matrix, float4(input.position, 1.0))",
		)
	} else {
		("", "", "", "", "", "float4(input.position, 1.0)")
	};
	let glsl = format!(
		"#version 450
		#pragma shader_stage(vertex)
		layout(location = 0) in vec3 in_position;
		layout(location = 1) in vec4 in_color;
		layout(location = 0) out vec4 out_color;
		{glsl_matrix}
		void main() {{
			out_color = in_color;
			gl_Position = {glsl_product}vec4(in_position, 1.0);
		}}"
	);
	let msl = format!(
		"{MSL_TYPES}
		vertex VertexOutput vertex_main(VertexInput input [[stage_in]]{msl_matrix}) {{
			return VertexOutput {{ {msl_product}float4(input.position, 1.0), input.color }};
		}}"
	);
	let hlsl = format!(
		"{HLSL_TYPES}
		{hlsl_matrix}
		VertexOutput vertex_main(VertexInput input) {{
			VertexOutput output;
			output.position = {hlsl_position};
			output.color = input.color;
			return output;
		}}"
	);
	let vertex = shader(
		device,
		"Triangle Vertex",
		ShaderTypes::Vertex,
		ShaderSource::PlatformNative {
			glsl: &glsl,
			msl: &msl,
			msl_entry_point: "vertex_main",
			hlsl: &hlsl,
			hlsl_entry_point: "vertex_main",
		},
		[],
	);
	let fragment = shader(
		device,
		"Triangle Fragment",
		ShaderTypes::Fragment,
		ShaderSource::PlatformNative {
			glsl: "#version 450
				#pragma shader_stage(fragment)
				layout(location = 0) in vec4 in_color;
				layout(location = 0) out vec4 out_color;
				void main() { out_color = in_color; }",
			msl: &format!(
				"{MSL_TYPES} fragment float4 fragment_main(VertexOutput input [[stage_in]]) {{ return input.color; }}"
			),
			msl_entry_point: "fragment_main",
			hlsl: &format!("{HLSL_TYPES} float4 fragment_main(VertexOutput input) : SV_TARGET0 {{ return input.color; }}"),
			hlsl_entry_point: "fragment_main",
		},
		[],
	);
	(vertex, fragment)
}

/// Creates a pipeline that draws [`VERTEX_LAYOUT`] meshes in their vertex colors into one RGBA8 attachment. With
/// `model_matrix`, it transforms positions by a 4x4 matrix pushed at offset 0.
pub(super) fn triangle_pipeline(device: &mut impl ghi::context::Context, model_matrix: bool) -> PipelineHandle {
	let (vertex, fragment) = triangle_shaders(device, model_matrix);
	let push_constants = if model_matrix {
		&[PushConstantRange::new(0, 16 * 4)][..]
	} else {
		&[]
	};
	device.create_raster_pipeline(pipelines::raster::Builder::new(
		push_constants,
		&VERTEX_LAYOUT,
		&[
			ShaderParameter::new(&vertex, ShaderTypes::Vertex),
			ShaderParameter::new(&fragment, ShaderTypes::Fragment),
		],
		&[AttachmentDescriptor::new(Formats::RGBA8UNORM)],
	))
}

/// Records a render pass that clears `target` to black and draws `mesh` with `pipeline`, pushing `model_matrix` when
/// there is one, then a transfer of `target` to read back.
pub(super) fn draw(
	recording: &mut impl CommandBufferRecording,
	target: ImageHandle,
	extent: Extent,
	pipeline: PipelineHandle,
	mesh: &MeshHandle,
	model_matrix: Option<[f32; 16]>,
) -> ghi::TextureCopyHandle {
	let attachments = [AttachmentInformation::new(
		target,
		Layouts::RenderTarget,
		ghi::LoadOp::Clear(ClearValue::Color(RGBA::black())),
		ghi::StoreOp::Store,
	)];
	let render_pass = recording.start_render_pass(extent, &attachments);
	let bound = render_pass.bind_raster_pipeline(pipeline);
	if let Some(matrix) = model_matrix {
		bound.write_push_constant(0, matrix);
	}
	bound.draw_mesh(mesh);
	render_pass.end_render_pass();
	transfer(recording, target)
}

/// Records a transfer of `image` to read back.
pub(super) fn transfer(recording: &mut impl CommandBufferRecording, image: ImageHandle) -> ghi::TextureCopyHandle {
	recording
		.transfer_texture(image.into())
		.expect("Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.")
}

/// Reads back an RGBA8 transfer as pixels.
pub(super) fn read_pixels(device: &mut impl ghi::context::Context, transfer: ghi::TextureCopyHandle) -> Vec<RGBAu8> {
	let readback = device
		.get_image_data(transfer)
		.expect("Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.");
	bytemuck::cast_slice(&readback.bytes).to_vec()
}

/// Returns the clear value that fills an image with `color`.
pub(super) fn clear_color(color: RGBAu8) -> ClearValue {
	let channel = |value: u8| f32::from(value) / 255.0;
	ClearValue::Color(RGBA::new(
		channel(color.r),
		channel(color.g),
		channel(color.b),
		channel(color.a),
	))
}

/// Checks the pixels of the reference triangle drawn over the whole of `extent`.
pub(super) fn check_triangle(pixels: &[RGBAu8], extent: Extent) {
	let (width, height) = (extent.width() as usize, extent.height() as usize);
	assert_eq!(pixels.len(), width * height);
	// The top corners are outside the triangle, and the bottom ones on its green and blue corners.
	assert_eq!(pixels[0], BLACK);
	assert_eq!(pixels[width - 1], BLACK);
	assert_eq!(pixels[width * (height - 1)], BLUE);
	assert_eq!(pixels[width * height - 1], GREEN);
	// An odd width has a center column, whose top pixel is the red corner.
	if width % 2 == 1 {
		assert_eq!(pixels[width / 2], RED);
	}
	// Implementations round the middle of the bottom edge slightly differently.
	let bottom_center = pixels[width * height - width / 2];
	assert!(
		[
			RGBAu8 {
				r: 0,
				g: 127,
				b: 127,
				a: 255
			},
			RGBAu8 {
				r: 0,
				g: 128,
				b: 127,
				a: 255
			}
		]
		.contains(&bottom_center),
		"{bottom_center:?}"
	);
}
