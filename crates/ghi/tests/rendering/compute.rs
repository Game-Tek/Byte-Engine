//! Compute dispatch scenarios: unordered groups, and buffer struct members read at their offsets.

use super::common::*;
use super::*;

/// Two dispatches in one unordered group each write their own row of an image, and the readback after the group
/// sees both rows.
///
/// With a `shader_key`, a SPIR-V shader is created from a keyed source and lends its code to the pipeline. Without
/// one, the backend is handed the SPIR-V to keep.
pub(super) fn unordered_dispatches_write_their_own_rows(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
	shader_key: Option<u64>,
) {
	let shader = ghi::shader::compile(
		"GHI unordered dispatch test compute shader",
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
					layout(offset = 128) uint row;
				} push_constants;
				layout(descriptor_heap, descriptor_stride = 64, rgba8) uniform image2D images[];
				layout(local_size_x = 1, local_size_y = 1, local_size_z = 1) in;
				void main() {
					vec4 color = push_constants.row == 0 ? vec4(1.0, 0.0, 0.0, 1.0) : vec4(0.0, 1.0, 0.0, 1.0);
					imageStore(images[push_constants.sets[SET0].x], ivec2(gl_GlobalInvocationID.x, push_constants.row), color);
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
				cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
				static RWTexture2D<float4> image = ResourceDescriptorHeap[_besl_sets[0].x];
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
	let layout = ghi::DescriptorSetLayout::new("Unordered Dispatch Target", [image_resource]);
	let (source, lent_code) = match (&shader, shader_key) {
		(CompiledShaderSource::SPIRV(code), Some(key)) => (ghi::shader::Sources::KeyedSPIRV { key, code }, Some(code)),
		(CompiledShaderSource::SPIRV(code), None) => (ghi::shader::Sources::SPIRV(code.clone().into()), None),
		_ => (shader.as_source(), None),
	};
	let compute_shader = device
		.create_shader(
			None,
			source,
			ShaderTypes::Compute,
			[ghi::ShaderDescriptorSet::new(layout.clone())],
		)
		.expect("Failed to create the unordered dispatch test compute shader");
	let parameter = ShaderParameter::new(&compute_shader, ShaderTypes::Compute);
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[PushConstantRange::new(0, 4)],
		lent_code.map_or(parameter, |code| parameter.with_code(code)),
	));
	let extent = Extent::rectangle(4, 2);
	let image = device.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource | Uses::Clear)
			.name("Unordered Dispatch Target")
			.extent(extent)
			.device_accesses(DeviceAccesses::DeviceToHost),
	);
	let descriptor_set = device.create_descriptor_set(None, &layout);
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

/// The kernel in GLSL. It reads struct members of a buffer declared by set and binding, which the pipeline maps to the
/// buffer's heap slot, and writes them to the output buffer.
const BOUND_BUFFER_GLSL: &str = r#"
#version 460
#pragma shader_stage(compute)
#extension GL_EXT_scalar_block_layout : require
layout(local_size_x = 1) in;
struct Layer { vec4 shape; vec4 low; vec4 high; };
layout(set = 0, binding = 0, scalar) readonly buffer Parameters { mat4 transform; Layer layers[2]; vec4 tail; } parameters;
layout(set = 0, binding = 1, scalar) writeonly buffer Results { vec4 results[]; } results;
void main() {
	results.results[0] = parameters.layers[0].shape;
	results.results[1] = parameters.layers[1].high;
	results.results[2] = parameters.tail;
}
"#;

/// The kernel in MSL. See [`BOUND_BUFFER_GLSL`].
const BOUND_BUFFER_MSL: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct Layer { float4 shape; float4 low; float4 high; };
struct Parameters { float4x4 transform; Layer layers[2]; float4 tail; };
struct Resources {
	device const Parameters* parameters [[id(0)]];
	device float4* results [[id(1)]];
};
kernel void bound_buffer_main(constant Resources& resources [[buffer(16)]]) {
	resources.results[0] = resources.parameters->layers[0].shape;
	resources.results[1] = resources.parameters->layers[1].high;
	resources.results[2] = resources.parameters->tail;
}
"#;

/// The kernel in HLSL. See [`BOUND_BUFFER_GLSL`]. It reads the buffers as `uint` words, since their views have the
/// layout's default four-byte stride: the first layer's `shape` starts at word 16, the second layer's `high` at word
/// 36, and `tail` at word 40.
const BOUND_BUFFER_HLSL: &str = r#"
cbuffer _BeslSets : register(b1, space0) { uint4 _besl_sets[1]; };
[numthreads(1, 1, 1)]
void besl_main() {
	StructuredBuffer<uint> parameters = ResourceDescriptorHeap[_besl_sets[0].x];
	RWStructuredBuffer<uint> results = ResourceDescriptorHeap[_besl_sets[0].x + 1];
	for (uint word = 0; word < 4; word++) {
		results[word] = parameters[16 + word];
		results[4 + word] = parameters[36 + word];
		results[8 + word] = parameters[40 + word];
	}
}
"#;

/// A buffer's struct members read from their own offsets, past a matrix and inside an array of structs.
///
/// Word `i` of the input holds `i`, so each result names the words it came from. Vulkan drivers have read such
/// members from the buffer's start when the shader reached the buffer through the heap alias, so shaders declare
/// buffers by set and binding.
pub(super) fn bound_buffer_struct_members_read_at_their_offsets(
	device: &mut impl ghi::context::Context,
	queue_handle: QueueHandle,
) {
	let layout = ghi::DescriptorSetLayout::new(
		"Bound Buffer Members",
		[
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(0),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::READ,
			),
			ghi::ShaderResourceDescriptor::single(
				ghi::ResourceSlot::new(1),
				ghi::ResourceKind::StorageBuffer,
				ghi::AccessPolicies::WRITE,
			),
		],
	);
	let shader = super::ray_tracing::native_shader(
		device,
		"Bound Buffer Members",
		ShaderTypes::Compute,
		(BOUND_BUFFER_GLSL, BOUND_BUFFER_MSL, "bound_buffer_main", BOUND_BUFFER_HLSL),
		[ghi::ShaderDescriptorSet::new(layout.clone())],
	);
	let pipeline = device.create_compute_pipeline(pipelines::compute::Builder::new(
		&[],
		ShaderParameter::new(&shader, ShaderTypes::Compute),
	));
	let parameters = device.build_buffer::<[f32]>(
		ghi::buffer::Builder::new(Uses::Storage)
			.name("Bound Buffer Parameters")
			.length(44)
			.device_accesses(DeviceAccesses::HostToDevice),
	);
	for (index, word) in device.get_mut_buffer_slice(parameters).iter_mut().enumerate() {
		*word = index as f32;
	}
	device.sync_buffer(parameters);
	let results = device.build_buffer::<[[f32; 4]]>(
		ghi::buffer::Builder::new(Uses::Storage)
			.name("Bound Buffer Results")
			.length(3)
			.device_accesses(DeviceAccesses::DeviceToHost),
	);
	let set = device.create_descriptor_set(None, &layout);
	device.write(&[
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(0), parameters.into()),
		ghi::DescriptorWrite::buffer(set, ghi::ResourceSlot::new(1), results.into()),
	]);

	let command_buffer = device.queue(queue_handle).create_command_buffer(None);
	let synchronizer = device.create_synchronizer(None, true);
	device
		.queue(queue_handle)
		.execute(Some(FrameRequest::new(0, synchronizer)), &[], synchronizer, |execution| {
			execution.record(command_buffer, |recording| {
				recording
					.bind_compute_pipeline(pipeline)
					.bind_descriptor_sets(&[set])
					.dispatch(DispatchExtent::new(Extent::line(1), Extent::line(1)));
			});
			[]
		});
	device.wait();
	assert!(!device.has_errors());

	// The layers start after the 16-word matrix and take 12 words each; the tail follows them at word 40.
	assert_eq!(
		device.get_buffer_slice(results),
		&[[16.0, 17.0, 18.0, 19.0], [36.0, 37.0, 38.0, 39.0], [40.0, 41.0, 42.0, 43.0]]
	);
}
