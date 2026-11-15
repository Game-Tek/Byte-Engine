use super::common::*;
use super::*;

/// The `TriangleDraw` enum selects the draw command a reference-triangle test records.
pub(super) enum TriangleDraw {
	/// Draws an indexed mesh.
	Mesh,
	/// Draws bound vertices with counts read from the second record of an indirect buffer.
	Indirect,
}

/// Draws the reference triangle with the selected draw command and checks the rendered pixels.
pub(super) fn render_triangle(device: &mut impl ghi::context::Context, queue_handle: QueueHandle, draw: TriangleDraw) {
	/// The `Geometry` enum holds the uploads one draw command reads.
	enum Geometry {
		Mesh(MeshHandle),
		Indirect(BufferHandle<[f32; 21]>, BufferHandle<[[u32; 4]; 2]>),
	}

	let floats: [f32; 21] = [
		0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
	];

	let vertex_layout = [
		VertexElement::new("POSITION", DataTypes::Float3, 0),
		VertexElement::new("COLOR", DataTypes::Float4, 0),
	];

	let geometry = match draw {
		TriangleDraw::Mesh => Geometry::Mesh(device.add_mesh_from_vertices_and_indices(
			3,
			3,
			f32_bytes(&floats),
			u16_bytes(&[0, 1, 2]),
			&vertex_layout,
		)),
		TriangleDraw::Indirect => {
			let vertex_buffer = device.build_buffer::<[f32; 21]>(
				ghi::buffer::Builder::new(Uses::Vertex)
					.name("Indirect Triangle Vertices")
					.host_access(HostAccess::Write),
			);
			*buffer_contents(device, vertex_buffer) = floats;

			// The first record draws no instances, so a triangle only appears if the draw reads the selected entry.
			let indirect_buffer = device.build_buffer::<[[u32; 4]; 2]>(
				ghi::buffer::Builder::new(Uses::Indirect)
					.name("Indirect Triangle Draws")
					.host_access(HostAccess::Write),
			);
			*buffer_contents(device, indirect_buffer) = [[3, 0, 0, 0], [3, 1, 0, 0]];
			Geometry::Indirect(vertex_buffer, indirect_buffer)
		}
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

	let render_target = device
		.build_image(ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource).extent(extent));

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

	let texture_copy_handle = {
		device.record(queue_handle, None, |recording| {
			let attachments = [AttachmentInformation::new(
				render_target,
				Layouts::RenderTarget,
				ghi::LoadOp::Clear(ClearValue::Color(RGBA::black())),
				ghi::StoreOp::Store,
			)];

			let render_pass = recording.start_render_pass(extent, &attachments);
			match geometry {
				Geometry::Mesh(mesh) => render_pass.bind_raster_pipeline(pipeline).draw_mesh(&mesh),
				Geometry::Indirect(vertex_buffer, indirect_buffer) => {
					render_pass.bind_vertex_buffers(&[BufferDescriptor::new(vertex_buffer)]);
					render_pass.bind_raster_pipeline(pipeline).draw_indirect(indirect_buffer, 1);
				}
			}
			render_pass.end_render_pass();

			recording
				.transfer_texture(render_target.into())
				.expect("Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.")
		})
	};

	device.wait();

	assert!(!device.has_errors());

	let pixels =
		rgba_pixels(device.get_image_data(texture_copy_handle).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

	check_triangle(&pixels, extent);
}

#[cfg(target_os = "macos")]
/// Uploads one overlapping triangle with a constant depth and color for native Metal depth-state validation.
fn add_depth_state_test_triangle(
	device: &mut impl ghi::context::Context,
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

	device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&vertices), u16_bytes(&indices), vertex_layout)
}

#[cfg(target_os = "macos")]
/// Verifies that a Metal raster pipeline can depth-test without replacing the retained reverse-Z depth.
pub(super) fn render_without_depth_writes(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
	let render_target = device
		.build_image(ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource).extent(extent));
	let depth_target = device.build_image(
		ghi::image::Builder::new(Formats::Depth32, Uses::DepthStencil)
			.extent(extent)
			.optimized_clear_value(ClearValue::Depth(0.0)),
	);
	let copy_handle = {
		device.record(queue_handle, None, |recording| {
			let attachments = [
				AttachmentInformation::new(
					render_target,
					Layouts::RenderTarget,
					ghi::LoadOp::Clear(ClearValue::Color(RGBA::black())),
					ghi::StoreOp::Store,
				),
				AttachmentInformation::new(
					depth_target,
					Layouts::RenderTarget,
					ghi::LoadOp::Clear(ClearValue::Depth(0.0)),
					ghi::StoreOp::Store,
				),
			];
			let render_pass = recording.start_render_pass(extent, &attachments);

			// With reverse-Z, the middle draw passes at 0.9 but must leave the first draw's 0.8 depth intact.
			render_pass.bind_raster_pipeline(depth_write_pipeline).draw_mesh(&first);
			render_pass.bind_raster_pipeline(no_depth_write_pipeline).draw_mesh(&no_write);
			render_pass.bind_raster_pipeline(depth_write_pipeline).draw_mesh(&last);
			// A later no-write draw behind retained opaque depth must still be rejected by the depth test.
			render_pass.bind_raster_pipeline(no_depth_write_pipeline).draw_mesh(&behind);
			render_pass.end_render_pass();

			recording
				.transfer_texture(render_target.into())
				.expect("Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.")
		})
	};

	device.wait();

	assert!(
		!device.has_errors(),
		"Metal depth-state rendering failed. The most likely cause is an invalid pipeline or render-pass attachment configuration.",
	);
	let image_data = device
		.get_image_data(copy_handle)
		.expect("Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.")
		.bytes;
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

/// Draws two overlapping triangles in a render pass without attachments, whose fragment shader keeps each pixel's
/// largest depth in an unsigned storage image with an image atomic, and checks every pixel holds the depth of the
/// nearest triangle covering it, or nothing.
pub(super) fn render_without_attachments_through_image_atomics(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let vertex_layout = [
		VertexElement::new("POSITION", DataTypes::Float3, 0),
		VertexElement::new("COLOR", DataTypes::Float4, 0),
	];
	// The reference triangle's winding, at reversed depths: the small triangle in front of the large one.
	let triangle = |scale: f32, depth: f32| -> [f32; 21] {
		[
			0.0, scale, depth, 1.0, 1.0, 1.0, 1.0, scale, -scale, depth, 1.0, 1.0, 1.0, 1.0, -scale, -scale, depth, 1.0, 1.0,
			1.0, 1.0,
		]
	};
	let large = triangle(1.0, 0.25);
	let small = triangle(0.5, 0.75);
	let large = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&large), u16_bytes(&[0, 1, 2]), &vertex_layout);
	let small = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&small), u16_bytes(&[0, 1, 2]), &vertex_layout);

	let fragment_shader_source = ghi::shader::compile(
		"GHI attachment-less atomic depth fragment shader",
		ShaderSource::PlatformNative {
			glsl: "
				#version 450
				#pragma shader_stage(fragment)
				#extension GL_EXT_descriptor_heap : require
				#extension GL_EXT_nonuniform_qualifier : require
				layout(constant_id = 1984) const uint SET0 = 0u;
				layout(push_constant) uniform PushConstants { uvec2 sets[15]; } push_constants;
				layout(descriptor_heap, descriptor_stride = 64, r32ui) uniform uimage2D images[];
				void main() {
					imageAtomicMax(images[push_constants.sets[SET0].x], ivec2(gl_FragCoord.xy), floatBitsToUint(gl_FragCoord.z));
				}
			",
			msl: r#"
				#include <metal_stdlib>
				using namespace metal;
				struct Resources {
					texture2d<uint, access::read_write> depths [[id(0)]];
				};
				fragment void fragment_main(float4 position [[position]], constant Resources& resources [[buffer(16)]]) {
					resources.depths.atomic_fetch_max(uint2(position.xy), as_type<uint>(position.z));
				}
			"#,
			msl_entry_point: "fragment_main",
			hlsl: r#"
				cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
				static RWTexture2D<uint> depths = ResourceDescriptorHeap[_besl_sets[0].x];
				void fragment_main(float4 position : SV_Position) {
					uint previous;
					InterlockedMax(depths[uint2(position.xy)], asuint(position.z), previous);
				}
			"#,
			hlsl_entry_point: "fragment_main",
		},
	)
	.expect("Failed to compile the attachment-less fragment shader. The most likely cause is invalid native shader source.");
	let depths_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(0),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::READ_WRITE,
	);
	let layout = ghi::DescriptorSetLayout::new("Atomic Depths", [depths_resource]);
	let (vertex_shader_source, _) = compile_shaders();
	let vertex_shader = device
		.create_shader(None, vertex_shader_source.as_source(), ShaderTypes::Vertex, [])
		.expect("Failed to create the attachment-less vertex shader");
	let fragment_shader = device
		.create_shader(
			None,
			fragment_shader_source.as_source(),
			ShaderTypes::Fragment,
			[ghi::ShaderDescriptorSet::new(layout.clone())],
		)
		.expect("Failed to create the attachment-less fragment shader");
	let pipeline = device.create_raster_pipeline(pipelines::raster::Builder::new(
		&[],
		&vertex_layout,
		&[
			ShaderParameter::new(&vertex_shader, ShaderTypes::Vertex),
			ShaderParameter::new(&fragment_shader, ShaderTypes::Fragment),
		],
		&[],
	));

	let extent = Extent::square(9);
	let depths = device.build_image(
		ghi::image::Builder::new(
			Formats::U32,
			Uses::Storage | Uses::Atomic | Uses::TransferSource | Uses::Clear,
		)
		.name("Atomic Depths")
		.extent(extent),
	);
	let descriptor_set = device.create_descriptor_set(None, &layout);
	device.write(&[ghi::DescriptorWrite::image(
		descriptor_set,
		depths_resource.slot(),
		depths,
		Layouts::General,
	)]);

	let copy_handle = device.record(queue_handle, None, |recording| {
		recording.clear_images(&[(depths.into(), ClearValue::Integer(0, 0, 0, 0))]);
		let render_pass = recording.start_render_pass(extent, &[]);
		// The nearer triangle draws first, so the far one's smaller depths must not replace it.
		let bound = render_pass.bind_raster_pipeline(pipeline);
		bound.bind_descriptor_sets(&[descriptor_set]);
		bound.draw_mesh(&small);
		bound.draw_mesh(&large);
		render_pass.end_render_pass();
		recording
			.transfer_texture(depths.into())
			.expect("Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.")
	});
	device.wait();
	assert!(
		!device.has_errors(),
		"The attachment-less render pass failed. The most likely cause is a backend that requires an attachment to rasterize."
	);

	let bytes = device
		.get_image_data(copy_handle)
		.expect("Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.")
		.bytes;
	let texel = |x: u32, y: u32| {
		let index = ((y * extent.width() + x) * 4) as usize;
		f32::from_bits(u32::from_ne_bytes(bytes[index..index + 4].try_into().unwrap()))
	};
	// The center lies in both triangles, a pixel near the bottom corners only in the large one, and the top corners
	// in neither.
	assert_eq!(texel(4, 4), 0.75);
	assert_eq!(texel(1, 7), 0.25);
	assert_eq!(texel(0, 0), 0.0);
}
