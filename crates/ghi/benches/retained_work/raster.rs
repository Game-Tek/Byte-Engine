use ghi::{
	command_buffer::{
		BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _, CommandBufferRecording as _,
		RasterizationRenderPassMode as _,
	},
	pipelines::{ShaderParameter, raster::AttachmentDescriptor},
	shader::ShaderSource,
};
use utils::{Extent, RGBA};

use super::*;

/// Selects the changing scene workload while keeping four draws per object.
#[derive(Clone, Copy)]
enum Workload {
	Animated,
	MixedMotion,
	DescriptorChurn,
	RebindSubdraws,
	FourRecordings,
}

/// Updates and renders a tiled scene, then verifies every object's latest color.
// Keep the complete GPU workflow together, matching the rendering tests.
#[allow(clippy::excessive_nesting)]
fn render(bencher: Bencher, objects: usize, workload: Workload) {
	let (_instance, _device, mut context, queue) = setup();
	let vertex_source = ghi::shader::compile("benchmark vertex", ShaderSource::PlatformNative {
		glsl: "#version 450\n#pragma shader_stage(vertex)\nlayout(push_constant) uniform Transform { vec4 tile; uint object_index; }; void main() { vec2 p[3] = vec2[](vec2(0,1),vec2(1,-1),vec2(-1,-1)); gl_Position = vec4(p[gl_VertexIndex]*tile.xy+tile.zw,0,1); }",
		msl: "#include <metal_stdlib>\nusing namespace metal; vertex float4 vertex_main(uint id [[vertex_id]], constant float4& tile [[buffer(15)]]) { float2 p[3] = {float2(0,1),float2(1,-1),float2(-1,-1)}; return float4(p[id]*tile.xy+tile.zw,0,1); }",
		msl_entry_point: "vertex_main",
		hlsl: "cbuffer Transform : register(b0, space0) { float4 tile; }; float4 vertex_main(uint id : SV_VertexID) : SV_POSITION { float2 p[3] = {float2(0,1),float2(1,-1),float2(-1,-1)}; return float4(p[id]*tile.xy+tile.zw,0,1); }",
		hlsl_entry_point: "vertex_main",
	}).expect("Failed to compile benchmark vertex shader. The most likely cause is invalid native shader source.");
	// All native sources read slot zero so descriptor retention remains observable at the public API seam.
	let fragment_source = ghi::shader::compile("benchmark fragment", ShaderSource::PlatformNative {
		glsl: "#version 450\n#pragma shader_stage(fragment)\nlayout(set=0,binding=0,std430) readonly buffer Colors { vec4 colors[]; }; layout(push_constant) uniform Object { vec4 tile; uint object_index; }; layout(location=0) out vec4 output_color; void main() { output_color = colors[object_index]; }",
		msl: "#include <metal_stdlib>\nusing namespace metal; struct Resources { device float4* color [[id(0)]]; }; struct Object { float4 tile; uint object_index; }; fragment float4 fragment_main(constant Resources& resources [[buffer(16)]], constant Object& object [[buffer(15)]]) { return resources.color[object.object_index]; }",
		msl_entry_point: "fragment_main",
		hlsl: "StructuredBuffer<float4> colors : register(t0, space0); cbuffer Object : register(b0, space0) { float4 tile; uint object_index; }; float4 fragment_main() : SV_TARGET0 { return colors[object_index]; }",
		hlsl_entry_point: "fragment_main",
	}).expect("Failed to compile benchmark fragment shader. The most likely cause is invalid native shader source.");
	let resource = ShaderResourceDescriptor::single(ResourceSlot::new(0), ResourceKind::StorageBuffer, AccessPolicies::READ);
	let vertex = context
		.create_shader(None, vertex_source.as_source(), ShaderTypes::Vertex, [])
		.expect("Failed to create benchmark vertex shader. The most likely cause is unsupported shader code.");
	let fragment = context
		.create_shader(None, fragment_source.as_source(), ShaderTypes::Fragment, [resource])
		.expect("Failed to create benchmark fragment shader. The most likely cause is unsupported shader resources.");
	let pipeline = context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
		&[ghi::pipelines::PushConstantRange::new(0, 32)],
		&[],
		&[
			ShaderParameter::new(&vertex, ShaderTypes::Vertex),
			ShaderParameter::new(&fragment, ShaderTypes::Fragment),
		],
		&[AttachmentDescriptor::new(Formats::RGBA8UNORM)],
	));
	// Each object occupies one tile, making every consumed update visible in readback.
	let columns = 16usize;
	let rows = objects / columns;
	let extent = Extent::rectangle((columns * 16) as _, (rows * 16) as _);
	let target = context.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget | Uses::TransferSource)
			.extent(extent)
			.device_accesses(DeviceAccesses::DeviceToHost),
	);
	// A packed object buffer models per-frame instance data. Both alternatives have identical capacity.
	let buffers = std::array::from_fn::<_, 2, _>(|_| {
		context.build_dynamic_buffer::<[[f32; 4]; 256]>(
			ghi::buffer::Builder::new(Uses::Storage).device_accesses(DeviceAccesses::HostToDevice),
		)
	});
	let set = context.create_descriptor_set(None);
	let writes = buffers.map(|buffer| DescriptorWrite::buffer(set, resource.slot(), buffer.into()));
	context.write(&[writes[0]]);
	let mut expected = vec![[0u8; 4]; objects];
	let transforms: Vec<_> = (0..objects)
		.map(|object| {
			[
				1.0 / columns as f32,
				1.0 / rows as f32,
				-1.0 + (2 * (object % columns) + 1) as f32 / columns as f32,
				-1.0 + (2 * (object / columns) + 1) as f32 / rows as f32,
			]
		})
		.collect();
	let commands: Vec<_> = (0..4).map(|_| context.queue(queue).create_command_buffer(None)).collect();
	let signal = context.create_synchronizer(None, true);
	let mut index = 0u64;
	measure(bencher, || {
		context
			.queue(queue)
			.execute(Some(FrameRequest::new(index, signal)), &[], signal, |execution| {
				let frame = execution.frame().unwrap();
				let active = if matches!(workload, Workload::DescriptorChurn) {
					index as usize % 2
				} else {
					0
				};
				if matches!(workload, Workload::DescriptorChurn) {
					// Consume each replacement this frame; pending writes cannot accumulate without submissions.
					frame.write(&[writes[active]]);
				}
				let data = frame.get_mut_dynamic_buffer_slice(buffers[active]);
				for object in 0..objects {
					let selected = object % 4 == index as usize % 4;
					if index < 2 || !matches!(workload, Workload::MixedMotion) || selected {
						let phase = (index + object as u64) % 8;
						let color = [(phase & 1) as f32, ((phase >> 1) & 1) as f32, ((phase >> 2) & 1) as f32, 1.0];
						expected[object] = color.map(|v| (v * 255.0) as u8);
					}
					// Repopulate the active slot from retained CPU data so mixed updates do not leave older slots stale.
					data[object] = expected[object].map(|value| value as f32 / 255.0);
				}
				frame.sync_buffer(buffers[active]);

				let recordings = if matches!(workload, Workload::FourRecordings) { 4 } else { 1 };
				for (part, &command) in commands[..recordings].iter().enumerate() {
					execution.record(command, |recording| {
						let attachments = [AttachmentInformation::new(
							target,
							Layouts::RenderTarget,
							if part == 0 {
								ghi::LoadOp::Clear(ClearValue::Color(RGBA::black()))
							} else {
								ghi::LoadOp::Load
							},
							ghi::StoreOp::Store,
						)];
						let pass = recording.start_render_pass(extent, &attachments);
						let bound = pass.bind_raster_pipeline(pipeline).bind_descriptor_sets(&[set]);
						for object in part * objects / recordings..(part + 1) * objects / recordings {
							// Object transforms change every frame, including in the mixed-motion case.
							let mut transform = transforms[object];
							let scale = 0.75 + 0.025 * (index % 8) as f32;
							transform[0] *= scale;
							transform[1] *= scale;
							bound.write_push_constant(0, transform);
							bound.write_push_constant(16, object as u32);
							for subdraw in 0..4 {
								if (object != part * objects / recordings || subdraw != 0)
									&& matches!(workload, Workload::RebindSubdraws)
								{
									bound.bind_descriptor_sets(&[set]);
								}
								bound.draw(3, 1, 0, 0);
							}
						}
						pass.end_render_pass();
					});
				}
				[]
			});
		// Report completed-frame latency; the wait remains identical across comparisons.
		context.wait();
		index += 1;
	});
	let mut readback = None;
	context
		.queue(queue)
		.execute(Some(FrameRequest::new(index, signal)), &[], signal, |execution| {
			execution.record(commands[0], |recording| {
				readback = Some(
					recording
						.transfer_texture(target.into())
						.expect("Benchmark readback failed. The most likely cause is an unsupported transfer source."),
				);
			});
			[]
		});
	context.wait();
	let pixels = context
		.get_image_data(readback.unwrap())
		.expect("Benchmark mapping failed. The most likely cause is incomplete GPU readback.");
	// Native viewport conventions can invert Y; require one consistent orientation across the image.
	let matches = |flip_y: bool| {
		(0..objects).all(|object| {
			let row = if flip_y {
				rows - 1 - object / columns
			} else {
				object / columns
			};
			let offset = (row * 16 + 8) * pixels.bytes_per_row + (object % columns * 16 + 8) * 4;
			pixels.bytes[offset..offset + 4] == expected[object]
		})
	};
	assert!(
		matches(false) || matches(true),
		"Benchmark pixels differ. The most likely cause is a stale object update, invalid binding, or lost attachment contents."
	);
	#[cfg(debug_assertions)]
	assert!(
		!context.has_errors(),
		"Scene benchmark failed. The most likely cause is invalid command or resource synchronization."
	);
}

#[divan::bench(args = [64, 256])]
fn animated_objects_completed(bencher: Bencher, objects: usize) {
	render(bencher, objects, Workload::Animated);
}

#[divan::bench(args = [64, 256])]
fn mixed_motion_completed(bencher: Bencher, objects: usize) {
	render(bencher, objects, Workload::MixedMotion);
}

#[divan::bench(args = [64, 256])]
fn streamed_descriptors_completed(bencher: Bencher, objects: usize) {
	render(bencher, objects, Workload::DescriptorChurn);
}

#[divan::bench(args = [64, 256])]
fn animated_rebind_subdraws_completed(bencher: Bencher, objects: usize) {
	render(bencher, objects, Workload::RebindSubdraws);
}

#[divan::bench(args = [64, 256])]
fn animated_four_recordings_completed(bencher: Bencher, objects: usize) {
	render(bencher, objects, Workload::FourRecordings);
}
