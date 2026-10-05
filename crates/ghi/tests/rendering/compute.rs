//! Compute dispatch scenarios: unordered groups.

use super::common::*;
use super::*;

/// Two dispatches in one unordered group each write their own row of an image, and the readback after the group
/// sees both rows.
pub(super) fn unordered_dispatches_write_their_own_rows(device: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
	let shader = ghi::shader::compile(
		"GHI unordered dispatch test compute shader",
		ShaderSource::PlatformNative {
			glsl: "
				#version 450
				#pragma shader_stage(compute)
				layout(set=0,binding=0, rgba8) uniform image2D img;
				layout(push_constant) uniform PushConstants {
					uint row;
				} push_constants;
				layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
				void main() {
					vec4 color = push_constants.row == 0 ? vec4(1.0, 0.0, 0.0, 1.0) : vec4(0.0, 1.0, 0.0, 1.0);
					imageStore(img, ivec2(gl_GlobalInvocationID.x, push_constants.row), color);
				}
			",
			msl: r#"
				#include <metal_stdlib>
				using namespace metal;
				struct Resources {
					texture2d<float, access::write> image [[id(0)]];
				};
				kernel void compute_main(
					uint2 gid [[thread_position_in_grid]],
					constant Resources& resources [[buffer(16)]],
					constant uint& row [[buffer(15)]]) {
					float4 color = row == 0 ? float4(1.0, 0.0, 0.0, 1.0) : float4(0.0, 1.0, 0.0, 1.0);
					resources.image.write(color, uint2(gid.x, row));
				}
			"#,
			msl_entry_point: "compute_main",
			hlsl: r#"
				RWTexture2D<float4> image : register(u0, space0);
				struct PushConstant { uint row; };
				ConstantBuffer<PushConstant> push_constant : register(b0, space0);
				[numthreads(1, 1, 1)]
				void compute_main(uint3 gid : SV_DispatchThreadID) {
					float4 color = push_constant.row == 0 ? float4(1.0, 0.0, 0.0, 1.0) : float4(0.0, 1.0, 0.0, 1.0);
					image[uint2(gid.x, push_constant.row)] = color;
				}
			"#,
			hlsl_entry_point: "compute_main",
		},
	)
	.expect(
		"Failed to compile the unordered dispatch test compute shader. The most likely cause is invalid native shader source.",
	);
	let image_resource = ghi::shader::ShaderResourceDescriptor::single(
		ghi::shader::ResourceSlot::new(0),
		ghi::shader::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
	let compute_shader = device
		.create_shader(None, shader.as_source(), ShaderTypes::Compute, [image_resource])
		.expect("Failed to create the unordered dispatch test compute shader");
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[PushConstantRange::new(0, 4)],
		ShaderParameter::new(&compute_shader, ShaderTypes::Compute),
	));
	let extent = Extent::rectangle(4, 2);
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource)
			.name("Unordered Dispatch Target")
			.extent(extent)
			.device_accesses(DeviceAccesses::DeviceToHost),
	);
	let descriptor_set = device.create_descriptor_set(None);
	device.write(&[ghi::DescriptorWrite::image(
		descriptor_set,
		image_resource.slot(),
		image,
		Layouts::General,
	)]);

	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	let mut transfer = None;
	device
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				recording.clear_images(&[(image.into(), ghi::ClearValue::Color(RGBA::new(0.0, 0.0, 0.0, 0.0)))]);
				recording.unordered(|recording| {
					for row in 0..extent.height() {
						let recording = recording.bind_compute_pipeline(pipeline);
						recording.bind_descriptor_sets(&[descriptor_set]);
						recording.write_push_constant(0, [row]);
						recording.dispatch(DispatchExtent::new(Extent::line(extent.width()), Extent::square(1)));
					}
				});
				transfer = Some(recording.transfer_texture(image.into()).expect(
					"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
				));
			});
			[]
		});
	device.wait();
	assert!(!device.has_errors());

	let pixels =
		rgba_pixels(device.get_image_data(transfer.unwrap()).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));
	let red = RGBAu8 {
		r: 255,
		g: 0,
		b: 0,
		a: 255,
	};
	let green = RGBAu8 {
		r: 0,
		g: 255,
		b: 0,
		a: 255,
	};
	let width = extent.width() as usize;
	assert!(
		pixels[..width].iter().all(|pixel| *pixel == red),
		"row 0: {:?}",
		&pixels[..width]
	);
	assert!(
		pixels[width..2 * width].iter().all(|pixel| *pixel == green),
		"row 1: {:?}",
		&pixels[width..]
	);
}
