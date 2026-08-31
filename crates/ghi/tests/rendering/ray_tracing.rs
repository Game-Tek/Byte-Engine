use super::common::*;
use super::*;

// The rendering scenario deliberately keeps acceleration-structure creation, binding, dispatch, and validation contiguous.
#[allow(clippy::too_many_lines)]
pub(super) fn ray_tracing(renderer: &mut impl ghi::context::Context, queue_handle: QueueHandle) {
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
		ghi::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
			.device_accesses(DeviceAccesses::HostToDevice),
	);
	let vertex_colors_buffer = renderer.build_buffer::<[f32; 4 * 3]>(
		ghi::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
			.device_accesses(DeviceAccesses::HostToDevice),
	);
	let index_buffer = renderer.build_buffer::<[u16; 3]>(
		ghi::buffer::Builder::new(Uses::Storage | Uses::AccelerationStructureBuild)
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
	let raygen_shader_artifact = ghi::shader::compile(
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
	let closest_hit_shader_artifact = ghi::shader::compile(
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
	let miss_shader_artifact = ghi::shader::compile(
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
	let acceleration_structure_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(0),
		ghi::ResourceKind::AccelerationStructure,
		ghi::AccessPolicies::READ,
	);
	let output_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(1),
		ghi::ResourceKind::StorageImage,
		ghi::AccessPolicies::WRITE,
	);
	let position_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(2),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
	);
	let color_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(3),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
	);
	let index_resource = ghi::ShaderResourceDescriptor::single(
		ghi::ResourceSlot::new(4),
		ghi::ResourceKind::StorageBuffer,
		ghi::AccessPolicies::READ,
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
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Storage | Uses::TransferSource)
			.extent(extent)
			.device_accesses(DeviceAccesses::DeviceToHost)
			.use_case(UseCases::DYNAMIC),
	);

	renderer.write(&[
		ghi::DescriptorWrite::acceleration_structure(
			descriptor_set,
			acceleration_structure_resource.slot(),
			top_level_acceleration_structure,
		),
		ghi::DescriptorWrite::image(descriptor_set, output_resource.slot(), render_target, Layouts::General),
		ghi::DescriptorWrite::buffer(descriptor_set, position_resource.slot(), vertex_positions_buffer.into()),
		ghi::DescriptorWrite::buffer(descriptor_set, color_resource.slot(), vertex_colors_buffer.into()),
		ghi::DescriptorWrite::buffer(descriptor_set, index_resource.slot(), index_buffer.into()),
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
		ghi::buffer::Builder::new(Uses::AccelerationStructureBuildScratch).device_accesses(DeviceAccesses::DeviceOnly),
	);

	let raygen_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
	);
	let miss_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
	);
	let hit_sbt_buffer = renderer.build_buffer::<[u8; 64]>(
		ghi::buffer::Builder::new(Uses::ShaderBindingTable).device_accesses(DeviceAccesses::HostToDevice),
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
				Some(FrameRequest::new(i as u64, render_finished_synchronizer)),
				&[],
				render_finished_synchronizer,
				|execution| {
					execution.record(rendering_command_buffer_handle, |command_buffer_recording| {
						{
							command_buffer_recording.build_bottom_level_acceleration_structures(&[
								BottomLevelAccelerationStructureBuild {
									acceleration_structure: bottom_level_acceleration_structure,
									description: BottomLevelAccelerationStructureBuildDescriptions::Mesh {
										vertex_buffer: BufferStridedRange::new(vertex_positions_buffer.into(), 0, 12, 12 * 3),
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

						texure_copy_handles = vec![command_buffer_recording.transfer_texture(render_target.into()).expect(
							"Texture transfer failed. The most likely cause is that the test image is not a valid transfer source.",
						)];
					});
					[]
				},
			);
			texure_copy_handles
		};

		renderer.end_frame_capture();

		assert!(!renderer.has_errors());

		let pixels = rgba_pixels(renderer.get_image_data(texure_copy_handles[0]).expect(
			"Texture mapping failed. The most likely cause is that the transfer handle was not recorded by this context.",
		));

		check_triangle(&pixels, extent);
	}
}
