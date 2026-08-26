use super::common::*;
use super::*;

pub(super) fn present(renderer: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
			Some(FrameRequest::new(0, render_finished_synchronizer)),
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

pub(super) fn multiframe_present(renderer: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
				Some(FrameRequest::new(i, render_finished_synchronizer)),
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
