use super::common::*;
use super::*;

pub(super) fn resize(device: &mut impl ghi::context::Context, queue_handle: QueueHandle, copies: ghi::Copies) {
	//! Tests that the render system keeps rendering while the number of frames in flight changes and the render target
	//! is resized. Frames in flight own a copy each of every per-frame resource, so changing their number creates more.
	//! Targets with one copy are one image shared by every frame, so the resize happens while an earlier frame may use it.

	let triangle = mesh(device, &TRIANGLE);
	let pipeline = triangle_pipeline(device, false);
	let mut extent = Extent::rectangle(1280, 720);
	let render_target = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
			.extent(extent)
			.copies(copies),
	);
	let frames = Frames::new(device, queue_handle);

	for i in 0..30 {
		if i == 2 {
			device.set_frames_in_flight(3); // Change from default 2 to 3
		}
		let transfer = record_frame!(device, frames, i,
			frame |frame| if i == 5 {
				extent = Extent::rectangle(1920, 1080);
				frame.resize_image(render_target.into(), extent);
			},
			|recording| draw(recording, render_target, extent, pipeline, &triangle, None)
		);
		device.wait();
		assert!(!device.has_errors());
		check_triangle(&read_pixels(device, transfer), extent);
	}
}

pub(super) fn resize_layers(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that resizing an array image changes how many layers it holds. The first frame grows a two-layer target to
	//! four layers and clears its last layer. The second shrinks it to one layer, which readback accepts only when the
	//! image really holds a single layer, and checks the clear color.

	let extent = Extent::square(16);
	let render_target = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
			.extent(extent)
			.array_layers(std::num::NonZeroU32::new(2)),
	);
	let frames = Frames::new(device, queue_handle);

	for (frame_index, (layers, layer)) in [(4, 3), (1, 0)].into_iter().enumerate() {
		let transfer = record_frame!(device, frames, frame_index as u64,
			frame |frame| frame.resize_image_layers(render_target.into(), extent, std::num::NonZeroU32::new(layers).unwrap()),
			|recording| {
				let attachments = [AttachmentInformation::new(
					render_target,
					Layouts::RenderTarget,
					ghi::LoadOp::Clear(clear_color(RED)),
					ghi::StoreOp::Store,
				)
				.layer(layer)];
				recording.start_render_pass(extent, &attachments).end_render_pass();
				(layers == 1).then(|| transfer(recording, render_target))
			}
		);
		device.wait();
		assert!(!device.has_errors());

		if let Some(transfer) = transfer {
			assert!(read_pixels(device, transfer).iter().all(|pixel| *pixel == RED));
		}
	}
}

pub(super) fn resize_render_target_in_flight(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that resizing a one-copy render target while earlier frames still render to it keeps the old image alive
	//! for them. Unlike [`resize`], no frame waits for the GPU, so frames overlap across each resize.

	let triangle = mesh(device, &TRIANGLE);
	let pipeline = triangle_pipeline(device, false);
	let mut extent = Extent::rectangle(640, 360);
	let render_target = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
			.extent(extent)
			.copies(ghi::Copies::One),
	);
	let frames = Frames::new(device, queue_handle);

	for i in 0..12 {
		record_frame!(device, frames, i,
			frame |frame| if i == 4 || i == 7 {
				extent = Extent::rectangle(extent.width() + 320, extent.height() + 180);
				frame.resize_image(render_target.into(), extent);
			},
			|recording| draw(recording, render_target, extent, pipeline, &triangle, None)
		);
	}

	device.wait();
	assert!(!device.has_errors());
}

pub(super) fn resize_dynamic_buffer(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that growing a per-frame buffer between frames keeps its old storage alive for the frames still in flight.
	//! Every frame copies the persistent CPU source into the GPU buffer, so earlier frames use the old storage.

	let buffer = device.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::Storage)
			.host_access(HostAccess::Write)
			.copies(ghi::Copies::PerFrame)
			.length(64),
	);
	let frames = Frames::new(device, queue_handle);

	for i in 0..12 {
		if i == 4 {
			device.resize_buffer(buffer, 4096);
		}
		record_frame!(device, frames, i, frame |frame| frame.map(buffer).fill(i as u8), |_recording| {});
	}

	device.wait();
	assert!(!device.has_errors());
}

pub(super) fn array_buffer_round_trip(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that an array buffer holds exactly its builder length and that the GPU can copy every element.

	const LENGTH: usize = 5;

	let array_builder = |name| {
		ghi::buffer::Builder::new(Uses::Storage | Uses::TransferSource | Uses::TransferDestination)
			.name(name)
			.length(LENGTH)
			.host_access(HostAccess::Write)
	};
	let source = device.build_buffer::<[u32]>(array_builder("Array Source"));
	let destination = device.build_buffer::<[u32]>(array_builder("Array Destination"));

	let values = buffer_contents(device, source);
	assert_eq!(values.len(), LENGTH);
	values.copy_from_slice(&[3, 1, 4, 1, 5]);

	device.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(
			source.into(),
			0,
			destination.into(),
			0,
			LENGTH * std::mem::size_of::<u32>(),
		)]);
	});

	assert!(!device.has_errors());
	assert_eq!(buffer_contents(device, destination), &[3, 1, 4, 1, 5]);
}

pub(super) fn dynamic_data(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that push constants reach each frame's draw: every frame turns the triangle a quarter turn further, and its
	//! green corner moves to the next screen corner.

	let triangle = mesh(device, &TRIANGLE);
	let pipeline = triangle_pipeline(device, true);
	let extent = Extent::rectangle(1920, 1080);
	let (width, height) = (extent.width() as usize, extent.height() as usize);
	let render_target = device
		.build_image(ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource).extent(extent));
	let frames = Frames::new(device, queue_handle);
	// The pixel the green corner lands on after each quarter turn: bottom right, bottom left, top left, top right.
	let corners = [width * height - 1, width * (height - 1), 0, width - 1];

	for i in 0..20 {
		let angle = (i as f32) * (std::f32::consts::PI / 2.0f32);
		let (sin, cos) = angle.sin_cos();
		let matrix = [
			cos, -sin, 0.0, 0.0, sin, cos, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
		];
		let transfer = record_frame!(device, frames, i, |recording| draw(
			recording,
			render_target,
			extent,
			pipeline,
			&triangle,
			Some(matrix)
		));
		device.wait();
		assert!(!device.has_errors());

		let pixels = read_pixels(device, transfer);
		assert_eq!(pixels[corners[i as usize % 4]], GREEN, "The green corner is not where frame {i} turned it.");
	}
}

pub(super) fn previous_frame_transfers(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests that commands in a frame write that frame's copy of a per-frame image, and that a frame-offset transfer
	//! reads the copy an earlier frame wrote while a plain transfer reads this frame's copy.

	let extent = Extent::square(2);
	let upload_image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferSource | Uses::Clear)
			.extent(extent)
			.copies(ghi::Copies::PerFrame),
	);
	let history_image = device.build_image(
		ghi::image::Builder::new(
			Formats::RGBA8UNORM,
			Uses::Image | Uses::TransferSource | Uses::TransferDestination,
		)
		.extent(extent)
		.copies(ghi::Copies::PerFrame),
	);
	let frames = Frames::new(device, queue_handle);

	let mut transfers = Vec::new();
	for (frame_index, color) in [RED, GREEN].into_iter().enumerate() {
		record_frame!(device, frames, frame_index as u64, |recording| {
			recording.clear_images(&[(upload_image.into(), clear_color(color))]);
			recording.blit_image(upload_image.into(), Layouts::Transfer, history_image.into(), Layouts::Transfer);
			// Only the second frame has an earlier copy to read.
			if frame_index == 1 {
				transfers.push(transfer(recording, history_image));
				transfers.push(recording.transfer_texture_with_frame(history_image.into(), -1).expect(
					"Previous-frame texture transfer failed. The most likely cause is that the backend cannot resolve frame offsets for transfers.",
				));
			}
		});
	}

	device.wait();

	let [current, previous] = transfers[..] else {
		panic!("The second frame must record exactly two transfers.");
	};
	assert!(read_pixels(device, current).iter().all(|pixel| *pixel == GREEN));
	assert!(read_pixels(device, previous).iter().all(|pixel| *pixel == RED));
	assert!(!device.has_errors());
}

pub(super) fn multiframe_resources(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	//! Tests frame-local image creation, previous-frame bindings, and sequence wraparound.

	// Use three sequences so frame 2 observes a fresh resource and frame 3 verifies the wrap back to frame 0.
	device.set_frames_in_flight(3);

	// TODO: test multiframe resources for combined image samplers
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
	let layout = ghi::DescriptorSetLayout::new("Multiframe Images", [image_resource, last_frame_image_resource]);
	let compute_shader = shader(
		device,
		"Multiframe Resources",
		ShaderTypes::Compute,
		ShaderSource::PlatformNative {
			glsl: "
				#version 450
				#pragma shader_stage(compute)
				#extension GL_EXT_descriptor_heap : require
				#extension GL_EXT_nonuniform_qualifier : require
				// The pipeline index of the shader's set 0, which the backend specializes.
				layout(constant_id = 1984) const uint SET0 = 0u;
				// Each set's resource and sampler heap base indices come before the push constants.
				layout(push_constant) uniform PushConstants {
					uvec2 sets[15];
					layout(offset = 128) float value;
				} push_constants;
				layout(descriptor_heap, descriptor_stride = 64, rgba8) uniform image2D images[];
				layout(descriptor_heap, descriptor_stride = 64, rgba8) uniform readonly image2D read_images[];
				layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
				void main() {
					uint base = push_constants.sets[SET0].x;
					imageStore(images[base], ivec2(0, 0), vec4(vec3(push_constants.value), 1));
					imageStore(images[base], ivec2(1, 0), imageLoad(read_images[base + 1], ivec2(0, 0)));
				}
			",
			msl: r#"
				#include <metal_stdlib>
				using namespace metal;
				struct Resources {
					texture2d<float, access::write> image [[id(0)]];
					texture2d<float, access::read> last_frame_image [[id(1)]];
				};
				kernel void compute_main(
					uint2 gid [[thread_position_in_grid]],
					constant Resources& resources [[buffer(16)]],
					constant float& value [[buffer(15)]]) {
					resources.image.write(float4(value, value, value, 1.0), uint2(0, 0));
					resources.image.write(resources.last_frame_image.read(uint2(0, 0)), uint2(1, 0));
				}
			"#,
			msl_entry_point: "compute_main",
			hlsl: r#"
				cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
				static RWTexture2D<float4> image = ResourceDescriptorHeap[_besl_sets[0].x];
				static RWTexture2D<float4> last_frame_image = ResourceDescriptorHeap[_besl_sets[0].x+1u];
				struct PushConstant { float value; };
				ConstantBuffer<PushConstant> push_constant : register(b0, space0);
				[numthreads(1, 1, 1)]
				void compute_main(uint3 gid : SV_DispatchThreadID) {
					image[uint2(0, 0)] = float4(push_constant.value.xxx, 1.0);
					image[uint2(1, 0)] = last_frame_image[uint2(0, 0)];
				}
			"#,
			hlsl_entry_point: "compute_main",
		},
		[ghi::ShaderDescriptorSet::new(layout.clone())],
	);
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[PushConstantRange::new(0, 4)],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource)
			.name("Image")
			.extent(Extent::square(2))
			.copies(ghi::Copies::PerFrame),
	);
	let descriptor_set = device.create_descriptor_set(None, &layout);
	device.write(&[
		ghi::DescriptorWrite::image(descriptor_set, image_resource.slot(), image, Layouts::General),
		ghi::DescriptorWrite::image(descriptor_set, last_frame_image_resource.slot(), image, Layouts::General)
			.with_frame_offset(-1),
	]);
	let frames = Frames::new(device, queue_handle);

	// What each frame writes, if anything, and what its first two pixels then hold: the written value, and what the
	// previous frame wrote. Frame 2 writes nothing into its fresh copy, and frame 3 sees frame 0's copy again.
	let gray = [127, 128].map(|value| RGBAu8 { r: value, g: value, b: value, a: 255 });
	let empty = RGBAu8 { r: 0, g: 0, b: 0, a: 0 };
	let white = RGBAu8 { r: 255, g: 255, b: 255, a: 255 };
	let expectations: [(Option<f32>, &[RGBAu8], &[RGBAu8]); 4] = [
		(Some(0.5), &gray, &[empty]),
		(Some(1.0), &[white], &gray),
		(None, &[empty], &[empty]),
		(None, &gray, &[empty]),
	];
	for (frame_index, (value, first, second)) in expectations.into_iter().enumerate() {
		let transfer = record_frame!(device, frames, frame_index as u64, |recording| {
			if let Some(value) = value {
				let bound = recording.bind_compute_pipeline(pipeline);
				bound.write_push_constant(0, [value]);
				bound
					.bind_descriptor_sets(&[descriptor_set])
					.dispatch(DispatchExtent::new(Extent::square(1), Extent::square(1)));
			}
			transfer(recording, image)
		});
		device.wait();

		let pixels = read_pixels(device, transfer);
		assert!(first.contains(&pixels[0]), "frame {frame_index}: {:?}", pixels[0]);
		assert!(second.contains(&pixels[1]), "frame {frame_index}: {:?}", pixels[1]);
		assert!(!device.has_errors());
	}
}

/// Uploads, reads, writes, and repacks a small 3D LUT through native Texture3D descriptors, so a copy from a buffer
/// into the volume must fill every depth slice.
pub(super) fn texture3d_lut_round_trip(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let extent = Extent::new(3, 2, 2);
	let mut expected = vec![0u8; (extent.width() * extent.height() * extent.depth() * 4) as usize];
	for z in 0..extent.depth() {
		for y in 0..extent.height() {
			for x in 0..extent.width() {
				let offset = (((z * extent.height() + y) * extent.width() + x) * 4) as usize;
				expected[offset..offset + 4].copy_from_slice(&[
					(17 + x * 29) as u8,
					(31 + y * 53) as u8,
					(47 + z * 71) as u8,
					255,
				]);
			}
		}
	}

	let source = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Image | Uses::TransferDestination)
			.name("3D LUT source")
			.extent(extent),
	);
	let layout = ghi::ImageCopyLayout::new(Formats::RGBA8UNORM, extent, extent.depth() as usize).unwrap();
	let staging = device.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("3D LUT staging")
			.host_access(ghi::HostAccess::Write)
			.length(layout.padded_size),
	);
	layout.write_rows(buffer_contents(device, staging), &expected, layout.compact_bytes_per_row);
	device.record(queue_handle, None, |recording| {
		recording.copy_buffer_to_images(&[layout.copy(staging.into(), 0, source.into(), 0)]);
	});
	let destination = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource)
			.name("3D LUT destination")
			.extent(extent),
	);

	let source_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(0),
		ghi::ResourceKind::SampledImage,
		ghi::AccessPolicies::READ,
	)
	.texture_view_type(ghi::TextureViewTypes::Texture3D);
	let destination_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(1),
		ghi::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	)
	.texture_view_type(ghi::TextureViewTypes::Texture3D);
	let layout = ghi::DescriptorSetLayout::new("Texture3D LUT descriptors", [source_resource, destination_resource]);
	let compute_shader = shader(
		device,
		"Texture3D LUT round trip",
		ShaderTypes::Compute,
		ShaderSource::PlatformNative {
			glsl: r#"
				#version 450
				#pragma shader_stage(compute)
				#extension GL_EXT_descriptor_heap : require
				#extension GL_EXT_nonuniform_qualifier : require
				#extension GL_EXT_samplerless_texture_functions : require
				// The pipeline index of the shader's set 0, and each set's heap base indices.
				layout(constant_id = 1984) const uint SET0 = 0u;
				layout(push_constant) uniform Sets { uvec2 sets[15]; } push_constants;
				// The source is a sampled image and the destination a storage image, as the set's layout declares.
				layout(descriptor_heap, descriptor_stride = 64) uniform texture3D source_textures[];
				layout(descriptor_heap, descriptor_stride = 64, rgba8) uniform writeonly image3D destination_textures[];
				layout(local_size_x=1, local_size_y=1, local_size_z=1) in;
				void main() {
					uint base = push_constants.sets[SET0].x;
					ivec3 location = ivec3(gl_GlobalInvocationID);
					imageStore(destination_textures[base + 1], location, texelFetch(source_textures[base], location, 0));
				}
			"#,
			msl: r#"
				#include <metal_stdlib>
				using namespace metal;
				struct Resources {
					texture3d<float, access::read> source_texture [[id(0)]];
					texture3d<float, access::write> destination_texture [[id(1)]];
				};
				kernel void compute_main(
					uint3 gid [[thread_position_in_grid]],
					constant Resources& resources [[buffer(16)]]) {
					resources.destination_texture.write(resources.source_texture.read(gid), gid);
				}
			"#,
			msl_entry_point: "compute_main",
			hlsl: r#"
				cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
				static Texture3D<float4> source_texture = ResourceDescriptorHeap[_besl_sets[0].x];
				static RWTexture3D<float4> destination_texture = ResourceDescriptorHeap[_besl_sets[0].x+1u];
				[numthreads(1, 1, 1)]
				void compute_main(uint3 gid : SV_DispatchThreadID) {
					destination_texture[gid] = source_texture.Load(int4(gid, 0));
				}
			"#,
			hlsl_entry_point: "compute_main",
		},
		[ghi::ShaderDescriptorSet::new(layout.clone())],
	);
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));
	let descriptor_set = device.create_descriptor_set(Some("Texture3D LUT descriptors"), &layout);
	device.write(&[
		ghi::DescriptorWrite::image(descriptor_set, source_resource.slot(), source, Layouts::Read),
		ghi::DescriptorWrite::image(descriptor_set, destination_resource.slot(), destination, Layouts::General),
	]);

	let transfer = record_frame!(device, Frames::new(device, queue_handle), 0, |recording| {
		recording
			.bind_compute_pipeline(pipeline)
			.bind_descriptor_sets(&[descriptor_set])
			.dispatch(DispatchExtent::new(extent, Extent::new(1, 1, 1)));
		transfer(recording, destination)
	});
	device.wait();

	let readback = device.get_image_data(transfer).expect(
		"Texture3D mapping failed. The most likely cause is that the native 3D copy footprint was not retained through submission.",
	);
	assert_eq!(readback.extent, extent);
	assert_eq!(readback.bytes_per_row, 12);
	assert_eq!(readback.bytes_per_image, 24);
	assert_eq!(readback.bytes, expected);
	assert!(!device.has_errors());
}

/// Checks whole and region copies from a buffer into an R8 image against GPU readback, including untouched texels and
/// image edges.
pub(super) fn texture_region_uploads(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	const WIDTH: usize = 37;
	const HEIGHT: usize = 29;

	let extent = Extent::rectangle(WIDTH as u32, HEIGHT as u32);
	let image = device.build_image(
		ghi::image::Builder::new(
			Formats::R8UNORM,
			Uses::Image | Uses::TransferSource | Uses::TransferDestination,
		)
		.extent(extent),
	);
	let readback = device.build_image(
		ghi::image::Builder::new(Formats::R8UNORM, Uses::TransferSource | Uses::TransferDestination).extent(extent),
	);
	// Each frame writes its own copy, so a frame never overwrites rows an earlier frame's copy still reads.
	let staging = device.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.host_access(HostAccess::Write)
			.copies(ghi::Copies::PerFrame)
			.length(ghi::ImageCopyLayout::new(Formats::R8UNORM, extent, 1).unwrap().padded_size),
	);
	let frames = Frames::new(device, queue_handle);
	let mut expected = vec![17u8; WIDTH * HEIGHT];
	let patches = [([0, 0], [WIDTH as u32, HEIGHT as u32]), ([3, 5], [7, 9]), ([31, 24], [6, 5])]
		.map(|(offset, size)| ghi::image::Region { offset, size });
	for (index, patch) in patches.into_iter().enumerate() {
		// The first copy covers the whole image with its starting value; later ones patch rectangles of it.
		let start = patch.offset[1] as usize * WIDTH + patch.offset[0] as usize;
		if index > 0 {
			for row in 0..patch.size[1] as usize {
				expected[start + row * WIDTH..][..patch.size[0] as usize].fill(40 + index as u8);
			}
		}
		// A region copy's source holds only the region's rows.
		let layout = ghi::ImageCopyLayout::new(Formats::R8UNORM, Extent::rectangle(patch.size[0], patch.size[1]), 1).unwrap();
		let copy = layout.copy(staging.into(), 0, image.into(), 0).region(patch);
		let transfer = record_frame!(device, frames, index as u64,
			frame |frame| layout.write_rows(frame.map(staging), &expected[start..], WIDTH),
			|recording| {
				recording.copy_buffer_to_images(&[copy]);
				recording.blit_image(image.into(), Layouts::Transfer, readback.into(), Layouts::Transfer);
				transfer(recording, readback)
			}
		);
		device.wait();
		assert_eq!(device.get_image_data(transfer).unwrap().bytes, expected);
		assert!(!device.has_errors());
	}
}

/// Creates a second context on `device`, the way a resource loader gets its own.
fn create_loader_context(device: &BackendDevice) -> BackendContext {
	use ghi::device::Device as _;

	device.create_context().expect(
		"Failed to create a second GHI test context. The most likely cause is that the active backend cannot create several contexts from one device.",
	)
}

pub(super) fn exported_image_keeps_contents_and_mips(
	device: &BackendDevice,
	context: &mut BackendContext,
	queue_handle: QueueHandle,
) {
	//! Tests that an image one context filled keeps its contents and mip count after another context interns it.
	//! Loaders upload images in their own context and export them, so the rendering context must read the uploaded
	//! base mip back and accept a copy into the last mip.

	const MIP_LEVELS: u32 = 3;
	const SIDE: usize = 4;
	const ROW_BYTES: usize = SIDE * 4;

	let expected = (0..SIDE * ROW_BYTES).map(|index| index as u8).collect::<Vec<_>>();
	let mut loader = create_loader_context(device);
	let image = loader.build_image(
		ghi::image::Builder::new(
			Formats::RGBA8UNORM,
			Uses::Image | Uses::TransferDestination | Uses::TransferSource,
		)
		.name("Exported Image")
		.extent(Extent::square(SIDE as u32))
		.mip_levels(MIP_LEVELS),
	);
	let base = ghi::ImageCopyLayout::new(Formats::RGBA8UNORM, Extent::square(SIDE as u32), 1).unwrap();
	let staging = loader.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Exported Image Staging")
			.host_access(HostAccess::Write)
			.length(base.padded_size),
	);
	base.write_rows(buffer_contents(&mut loader, staging), &expected, ROW_BYTES);
	loader.record(queue_handle, None, |recording| {
		recording.copy_buffer_to_images(&[base.copy(staging.into(), 0, image.into(), 0)]);
	});
	let image = context.intern_image(loader.export_image(image));

	let last_mip_layout = ghi::ImageCopyLayout::new(Formats::RGBA8UNORM, Extent::square(1), 1).unwrap();
	let last_mip = context.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Last Mip Staging")
			.host_access(HostAccess::Write)
			.length(last_mip_layout.padded_size),
	);
	let transfer = record_frame!(context, Frames::new(context, queue_handle), 0, |recording| {
		recording.copy_buffer_to_images(&[last_mip_layout.copy(last_mip.into(), 0, image.into(), MIP_LEVELS - 1)]);
		transfer(recording, image)
	});
	context.wait();

	let readback = context.get_image_data(transfer).unwrap();
	for (row, texels) in expected.chunks(ROW_BYTES).enumerate() {
		assert_eq!(&readback.bytes[row * readback.bytes_per_row..][..ROW_BYTES], texels);
	}
	assert!(!ghi::context::Context::has_errors(context));
}

pub(super) fn imported_buffer_receives_copies(device: &BackendDevice, context: &mut BackendContext, queue_handle: QueueHandle) {
	//! Tests that a context can copy into a buffer another context owns and shared with it.
	//! Loaders append geometry to buffers the rendering context draws from, so the owner must see those bytes.

	let expected: [u8; 64] = std::array::from_fn(|index| (index * 3) as u8);
	let destination = context.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferDestination)
			.name("Shared Destination")
			.host_access(HostAccess::Write),
	);
	let mut loader = create_loader_context(device);
	let imported = loader.import_buffer(context.share_buffer(destination));
	let staging = loader.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Shared Source")
			.host_access(HostAccess::Write),
	);
	*buffer_contents(&mut loader, staging) = expected;
	loader.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(staging.into(), 0, imported.into(), 0, 64)]);
	});

	assert_eq!(buffer_contents(context, destination), &expected);
	assert!(!ghi::context::Context::has_errors(&loader));
}

pub(super) fn imported_private_buffer_keeps_copies_made_before_its_owner_records(
	device: &BackendDevice,
	context: &mut BackendContext,
	queue_handle: QueueHandle,
) {
	//! Tests that a buffer without host access starts zeroed before another context writes it, never after.
	//! The lightmap bake shares its geometry buffers and loads the scene into them before it records anything.

	let expected: [u8; 64] = std::array::from_fn(|index| (index * 3 + 1) as u8);
	let destination = context.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferDestination | Uses::TransferSource).name("Shared Private Destination"),
	);
	let mut loader = create_loader_context(device);
	let imported = loader.import_buffer(context.share_buffer(destination));
	let staging = loader.build_buffer_with::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Shared Private Source")
			.host_access(HostAccess::Write),
		|bytes| *bytes = expected,
	);
	loader.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(staging.into(), 0, imported.into(), 0, 64)]);
	});

	// The owner's first recording must not zero what the loader already wrote.
	let readback = context.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferDestination)
			.name("Shared Private Readback")
			.host_access(HostAccess::Read),
	);
	context.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(destination.into(), 0, readback.into(), 0, 64)]);
	});

	assert_eq!(buffer_contents(context, readback), &expected);
	assert!(!ghi::context::Context::has_errors(&loader));
	assert!(!ghi::context::Context::has_errors(context));
}

pub(super) fn imported_private_buffer_keeps_copies_made_while_its_zero_fill_runs(
	device: &BackendDevice,
	context: &mut BackendContext,
	queue_handle: QueueHandle,
) {
	//! Tests that sharing a buffer waits for a zero fill a frame already submitted, so the fill can't overwrite a copy
	//! another context makes after sharing. A large buffer keeps the fill running while the copy is submitted.

	const SIZE: usize = 256 << 20;
	let expected = [42u8; 64];
	let destination = context.build_buffer::<[u8]>(
		ghi::buffer::Builder::new(Uses::TransferDestination | Uses::TransferSource)
			.name("Shared Private Destination")
			.length(SIZE),
	);
	// The loader is ready before the fill starts, so its copy follows the fill as closely as possible.
	let mut loader = create_loader_context(device);
	let staging = loader.build_buffer_with::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Shared Private Source")
			.host_access(HostAccess::Write),
		|bytes| *bytes = expected,
	);
	let readback = context.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferDestination)
			.name("Shared Private Readback")
			.host_access(HostAccess::Read),
	);
	// The frame's recording submits the pending zero fill, and frames don't wait for their work.
	let command_buffer = context.queue(queue_handle).create_command_buffer(None);
	let signal = context.create_synchronizer(None, true);
	context
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, signal)), &[], signal, |execution| {
			execution.record(command_buffer, |_| {});
			[]
		});
	let imported = loader.import_buffer(context.share_buffer(destination));
	loader.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(staging.into(), 0, imported.into(), 0, 64)]);
	});

	context.wait();
	context.record(queue_handle, None, |recording| {
		recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(destination.into(), 0, readback.into(), 0, 64)]);
	});

	assert_eq!(buffer_contents(context, readback), &expected);
	assert!(!ghi::context::Context::has_errors(&loader));
	assert!(!ghi::context::Context::has_errors(context));
}

pub(super) fn transferred_mapping_feeds_every_later_copy(context: &mut BackendContext, queue_handle: QueueHandle) {
	//! Tests that bytes written through a transferred mapping reach copies submitted after the first one.
	//! Loaders keep writing a staging buffer through such a mapping from other threads, one batch after another.

	let staging = context.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::TransferSource)
			.name("Transferred Staging")
			.host_access(HostAccess::Write),
	);
	let destination = context.build_buffer::<[u8; 128]>(
		ghi::buffer::Builder::new(Uses::TransferDestination)
			.name("Transferred Destination")
			.host_access(HostAccess::Write),
	);
	// SAFETY: The context outlives the mapping, and the buffer is not mapped again while it exists.
	let (address, byte_count) = unsafe { context.map_unsynchronized(staging) }.into_raw_parts();
	assert_eq!(byte_count, 64);
	let write = |value: u8| {
		// SAFETY: The transferred mapping covers 64 writable bytes, and no submitted copy reads them while they change.
		unsafe { std::slice::from_raw_parts_mut(address as *mut u8, 64) }.fill(value);
	};

	for (batch, value) in [3u8, 7].into_iter().enumerate() {
		write(value);
		context.record(queue_handle, None, |recording| {
			recording.copy_buffers(&[ghi::BufferCopyDescriptor::new(
				staging.into(),
				0,
				destination.into(),
				batch * 64,
				64,
			)]);
		});
	}

	let contents = buffer_contents(context, destination);
	assert_eq!(&contents[..64], &[3; 64]);
	assert_eq!(&contents[64..], &[7; 64]);
	assert!(!ghi::context::Context::has_errors(context));
}

pub(super) fn dynamic_buffer_created_in_a_frame_has_a_copy_per_frame(context: &mut BackendContext, queue_handle: QueueHandle) {
	//! Tests that a per-frame buffer created while a frame is recorded gives every frame in flight its own copy.
	//! Frames overlap on the GPU, so a copy the next frame shared would let the CPU overwrite data a frame still reads.

	context.set_frames_in_flight(2);
	let signal = context.create_synchronizer(None, true);
	let mut buffer = None;

	for frame_index in 0..4u64 {
		context
			.queue(queue_handle)
			.execute(Some(FrameRequest::new(frame_index, signal)), &[], signal, |execution| {
				let frame = execution.frame().expect("The test requested a frame.");
				match frame_index {
					// Frame 1 is the second sequence, so the buffer is created while that sequence records.
					1 => {
						let created = frame.build_buffer::<[u32; 4]>(
							ghi::buffer::Builder::new(Uses::Storage)
								.host_access(HostAccess::Write)
								.copies(ghi::Copies::PerFrame),
						);
						*frame.map(created) = [1; 4];
						buffer = Some(created);
					}
					// Frame 2 is the first sequence again. Its copy never received frame 1's write.
					2 => assert_eq!(
						*frame.map(buffer.expect("Frame 1 created the buffer.")),
						[0; 4],
						"A per-frame buffer created in frame 1 shares a copy with frame 2. The most likely cause is that its per-frame copies are created lazily."
					),
					// Frame 3 reuses frame 1's sequence and still sees what frame 1 wrote.
					3 => assert_eq!(*frame.map(buffer.expect("Frame 1 created the buffer.")), [1; 4]),
					_ => {}
				}
				[]
			});
	}
	context.wait();
	assert!(!ghi::context::Context::has_errors(context));
}
