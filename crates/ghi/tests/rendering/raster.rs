use super::common::*;
use super::*;

pub(super) fn render_triangle(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
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

		let texture_copy_handles =
			vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
				"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
			)];

		command_buffer_recording.execute(signal);
		texture_copy_handles
	};

	device.end_frame_capture();

	device.wait();

	assert!(!device.has_errors());

	let pixels =
		rgba_pixels(device.get_image_data(texture_copy_handles[0]).expect(
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
pub(super) fn render_without_depth_writes(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
			.extent(extent)
			.device_accesses(DeviceAccesses::DeviceToHost)
			.use_case(UseCases::STATIC),
	);
	let depth_target = device.build_image(
		ghi::image::Builder::new(Formats::Depth32, Uses::DepthStencil)
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

		let texture_copy_handles =
			vec![recording.transfer_texture(render_target.into()).expect(
				"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
			)];
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
