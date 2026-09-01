use super::common::*;
use super::*;

pub(super) fn multiframe_rendering(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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

	let mesh = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&floats), u16_bytes(&[0, 1, 2]), &vertex_layout);

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
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
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
				Some(FrameRequest::new(i as u64, render_finished_synchronizer)),
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

						texture_copy_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			texture_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		let pixels = rgba_pixels(device.get_image_data(texture_copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

		check_triangle(&pixels, extent);
	}
}

pub(super) fn change_frames(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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

	let mesh = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&floats), u16_bytes(&[0, 1, 2]), &vertex_layout);

	let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

	let vertex_shader = device
		.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
		.expect("Failed to create vertex shader");
	let fragment_shader = device
		.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
		.expect("Failed to create fragment shader");

	let extent = Extent::rectangle(1920, 1080);

	let render_target = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
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
				Some(FrameRequest::new(i as u64, render_finished_synchronizer)),
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

						texture_copy_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			texture_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		let pixels = rgba_pixels(device.get_image_data(texture_copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

		check_triangle(&pixels, extent);
	}
}

pub(super) fn resize(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that the render system can perform rendering while resize the render targets.

	const FRAMES_IN_FLIGHT: usize = 3;

	let floats: [f32; 21] = [
		0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
	];

	let vertex_layout = [
		VertexElement::new("POSITION", DataTypes::Float3, 0),
		VertexElement::new("COLOR", DataTypes::Float4, 0),
	];

	let mesh = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&floats), u16_bytes(&[0, 1, 2]), &vertex_layout);

	let (vertex_shader_artifact, fragment_shader_artifact) = compile_shaders();

	let vertex_shader = device
		.create_shader(None, vertex_shader_artifact.as_source(), ShaderTypes::Vertex, [])
		.expect("Failed to create vertex shader");
	let fragment_shader = device
		.create_shader(None, fragment_shader_artifact.as_source(), ShaderTypes::Fragment, [])
		.expect("Failed to create fragment shader");

	let mut extent = Extent::rectangle(1280, 720);

	let render_target = device.build_dynamic_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
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
				Some(FrameRequest::new(i as u64, render_finished_synchronizer)),
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

						texture_copy_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			texture_copy_handles
		};

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		let image_data = device
			.get_image_data(texture_copy_handles[0])
			.expect(
				"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
			)
			.bytes;
		let pixel_count = (extent.width() * extent.height()) as usize;

		assert_eq!(
			image_data.len(),
			pixel_count * std::mem::size_of::<RGBAu8>(),
			"Render-target readback size does not match its resized extent. The most likely cause is that one frame-local image kept its previous extent."
		);
		let pixels = image_data
			.chunks_exact(4)
			.map(|pixel| RGBAu8 {
				r: pixel[0],
				g: pixel[1],
				b: pixel[2],
				a: pixel[3],
			})
			.collect::<Vec<_>>();

		assert_eq!(pixels.len(), (extent.width() * extent.height()) as usize);

		check_triangle(&pixels, extent);
	}
}

// The rendering scenario shares one resource setup across all dynamic-data frame transitions.
#[allow(clippy::too_many_lines)]
pub(super) fn dynamic_data(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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

	let mesh = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&floats), u16_bytes(&[0, 1, 2]), &vertex_layout);

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
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
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
		device.build_buffer::<u8>(ghi::buffer::Builder::new(Uses::Storage).device_accesses(DeviceAccesses::HostToDevice));

	let command_buffer_handle = device.queue(queue_handle).create_command_buffer(None);

	let render_finished_synchronizer = device.create_synchronizer(None, true);

	for i in 0..FRAMES_IN_FLIGHT * 10 {
		device.start_frame_capture();

		let copy_texture_handles = {
			let mut queue = device.queue(queue_handle);
			let mut copy_texture_handles = Vec::new();
			queue.execute(
				Some(FrameRequest::new(i as u64, render_finished_synchronizer)),
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

						copy_texture_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			copy_texture_handles
		};

		device.end_frame_capture();

		device.wait();

		assert!(!device.has_errors());

		let pixels = rgba_pixels(device.get_image_data(copy_texture_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

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

pub(super) fn dynamic_textures(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that dynamic textures write to the current frame image instead of always writing to the root image.

	let extent = Extent::square(2);
	let pixel_count = (extent.width() * extent.height()) as usize;

	let upload_image = device.build_dynamic_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferSource)
			.extent(extent)
			.device_accesses(DeviceAccesses::HostToDevice),
	);

	let readback_image = device.build_dynamic_image(
		ghi::image::Builder::new(
			Formats::RGBA8UNORM,
			Uses::Image | Uses::TransferSource | Uses::TransferDestination,
		)
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
				Some(FrameRequest::new(frame_index as u64, render_finished_synchronizer)),
				&[],
				render_finished_synchronizer,
				|execution| {
					let frame = execution.frame().unwrap();

					let texture_slice = frame.get_mut_dynamic_texture_slice(upload_image.into());
					for pixel in texture_slice.chunks_exact_mut(4).take(pixel_count) {
						pixel.copy_from_slice(&[expected_color.r, expected_color.g, expected_color.b, expected_color.a]);
					}
					frame.sync_texture(upload_image.into());

					execution.record(command_buffer_handle, |command_buffer_recording| {
						command_buffer_recording.blit_image(
							upload_image.into(),
							Layouts::Transfer,
							readback_image.into(),
							Layouts::Transfer,
						);
						texture_copy_handles = vec![command_buffer_recording.transfer_texture(readback_image.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			texture_copy_handles
		};

		device.wait();

		let pixels = rgba_pixels(device.get_image_data(texture_copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

		assert!(pixels.iter().all(|pixel| *pixel == expected_color));
		assert!(!device.has_errors());
	}
}

// The rendering scenario validates one resource set across its complete multi-frame lifetime.
#[allow(clippy::too_many_lines)]
pub(super) fn multiframe_resources(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests frame-local image creation, previous-frame bindings, and sequence wraparound.

	// Use three sequences so frame 2 observes a fresh resource and frame 3 verifies the wrap back to frame 0.
	device.set_frames_in_flight(3);

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
	let compute_shader_artifact = ghi::shader::compile(
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
	let image_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(0),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
	let last_frame_image_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(1),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::READ,
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
		&[PushConstantRange::new(0, 4)],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));

	let image = device.build_dynamic_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource)
			.name("Image")
			.extent(Extent::square(2))
			.device_accesses(DeviceAccesses::DeviceToHost),
	);

	let descriptor_set = device.create_descriptor_set(None);
	device.write(&[
		ghi::DescriptorWrite::image(descriptor_set, image_resource.slot(), image, Layouts::General),
		ghi::DescriptorWrite::image_with_frame(descriptor_set, last_frame_image_resource.slot(), image, Layouts::General, -1),
	]);

	let command_buffer = device.queue(queue_handle).create_command_buffer(None);

	let signal = device.create_synchronizer(None, true);

	let copy_handles = {
		let mut queue = device.queue(queue_handle);
		let mut copy_handles = Vec::new();
		queue.execute(Some(FrameRequest::new(0, signal)), &[], signal, |execution| {
			execution.record(command_buffer, |command_buffer_recording| {
				let data = [0.5f32];

				let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

				pipeline_command.write_push_constant(0, data);
				pipeline_command
					.bind_descriptor_sets(&[descriptor_set])
					.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

				copy_handles = vec![command_buffer_recording.transfer_texture(image.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				)];
			});
			[]
		});
		copy_handles
	};

	device.wait();

	let pixels =
		rgba_pixels(device.get_image_data(copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

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
		queue.execute(Some(FrameRequest::new(1, signal)), &[], signal, |execution| {
			execution.record(command_buffer, |command_buffer_recording| {
				let data = [1.0f32];

				let pipeline_command = command_buffer_recording.bind_compute_pipeline(pipeline);

				pipeline_command.write_push_constant(0, data);
				pipeline_command
					.bind_descriptor_sets(&[descriptor_set])
					.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));

				copy_handles = vec![command_buffer_recording.transfer_texture(image.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				)];
			});
			[]
		});
		copy_handles
	};

	device.wait();

	let pixels =
		rgba_pixels(device.get_image_data(copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

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
		queue.execute(Some(FrameRequest::new(2, signal)), &[], signal, |execution| {
			execution.record(command_buffer, |command_buffer_recording| {
				copy_handles = vec![command_buffer_recording.transfer_texture(image.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				)];
			});
			[]
		});
		copy_handles
	};

	device.wait();

	let pixels =
		rgba_pixels(device.get_image_data(copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

	assert_eq!(pixels[0], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });
	assert_eq!(pixels[1], RGBAu8 { r: 0, g: 0, b: 0, a: 0 });
	assert!(!device.has_errors());

	let copy_handles = {
		let mut queue = device.queue(queue_handle);
		let mut copy_handles = Vec::new();
		queue.execute(Some(FrameRequest::new(3, signal)), &[], signal, |execution| {
			execution.record(command_buffer, |command_buffer_recording| {
				copy_handles = vec![command_buffer_recording.transfer_texture(image.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				)];
			});
			[]
		});
		copy_handles
	};

	device.wait();

	let pixels =
		rgba_pixels(device.get_image_data(copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

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

// The rendering scenario keeps descriptor creation, mutation, binding, and validation in one contiguous contract.
#[allow(clippy::too_many_lines)]
pub(super) fn descriptor_sets(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let signal = device.create_synchronizer(None, true);

	let floats: [f32; 21] = [
		0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, -1.0, 0.0, 0.0, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 0.0, 0.0, 1.0, 1.0,
	];

	let vertex_layout = [
		VertexElement::new("POSITION", DataTypes::Float3, 0),
		VertexElement::new("COLOR", DataTypes::Float4, 0),
	];

	let mesh = device.add_mesh_from_vertices_and_indices(3, 3, f32_bytes(&floats), u16_bytes(&[0, 1, 2]), &vertex_layout);

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
	let vertex_shader_artifact = ghi::shader::compile(
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
	let fragment_shader_artifact = ghi::shader::compile(
		"GHI descriptor test fragment shader",
		ShaderSource::PlatformNative {
			glsl: fragment_shader_code,
			msl: fragment_shader_msl,
			msl_entry_point: "besl_main",
			hlsl: fragment_shader_hlsl,
			hlsl_entry_point: "fragment_main",
		},
	)
	.expect("Failed to compile the descriptor test fragment shader. The most likely cause is invalid native shader source.");

	let buffer_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(1),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
	);
	let texture_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(0),
		ghi::ResourceKind::CombinedImageSampler,
		ghi::AccessPolicies::READ,
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
		ghi::buffer::Builder::new(Uses::Uniform | Uses::Storage).device_accesses(DeviceAccesses::HostToDevice),
	);

	let sampled_texture = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Image)
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
		ghi::sampler::Builder::new()
			.filtering_mode(FilteringModes::Closest)
			.reduction_mode(SamplingReductionModes::WeightedAverage)
			.mip_map_mode(FilteringModes::Closest)
			.addressing_mode(SamplerAddressingModes::Repeat)
			.min_lod(0.0f32)
			.max_lod(0.0f32),
	);

	let descriptor_set = device.create_descriptor_set(None);
	device.write(&[
		ghi::DescriptorWrite::combined_image_sampler(
			descriptor_set,
			texture_resource.slot(),
			sampled_texture,
			sampler,
			Layouts::Read,
		),
		ghi::DescriptorWrite::buffer(descriptor_set, buffer_resource.slot(), buffer.into()),
	]);

	assert!(!device.has_errors());

	// Use and odd width to make sure there is a middle/center pixel
	let extent = Extent::rectangle(1920, 1080);

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

	let texure_copy_handles = {
		let mut queue = device.queue(queue_handle);
		let mut texure_copy_handles = Vec::new();
		queue.execute(Some(FrameRequest::new(0, signal)), &[], signal, |execution| {
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

				texure_copy_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				)];
			});
			[]
		});
		texure_copy_handles
	};

	device.end_frame_capture();

	device.wait();

	// assert colored triangle was drawn to texture
	let _pixels = device
		.get_image_data(texure_copy_handles[0])
		.expect("Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.")
		.bytes;

	// TODO: assert rendering results

	assert!(!device.has_errors());
}
