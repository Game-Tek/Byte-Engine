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

/// How many textures the shared set holds, the size of a bindless texture table.
const TEXTURES: u32 = 1024;

/// Selects what fills the shared texture table before the measured frames.
#[derive(Clone, Copy)]
enum Table {
	/// One placeholder texture in every element, as before a scene's textures arrive.
	Placeholder,
	/// A different texture in every element, as in a loaded scene.
	Loaded,
}

/// Draws once with each of `pipelines` pipelines, which all read one bindless texture set, after changing one
/// texture of that set, as a frame does while textures stream in.
fn render(bencher: Bencher, pipelines: usize, table: Table) {
	let (_instance, _device, mut context, queue) = setup();
	let vertex_source = ghi::shader::compile("benchmark vertex", ShaderSource::PlatformNative {
		glsl: "#version 450\n#pragma shader_stage(vertex)\nvoid main() { vec2 p[3] = vec2[](vec2(0,1),vec2(1,-1),vec2(-1,-1)); gl_Position = vec4(p[gl_VertexIndex], 0, 1); }",
		msl: "#include <metal_stdlib>\nusing namespace metal; vertex float4 vertex_main(uint id [[vertex_id]]) { float2 p[3] = {float2(0,1),float2(1,-1),float2(-1,-1)}; return float4(p[id], 0, 1); }",
		msl_entry_point: "vertex_main",
		hlsl: "float4 vertex_main(uint id : SV_VertexID) : SV_POSITION { float2 p[3] = {float2(0,1),float2(1,-1),float2(-1,-1)}; return float4(p[id], 0, 1); }",
		hlsl_entry_point: "vertex_main",
	})
	.expect("Failed to compile benchmark vertex shader. The most likely cause is invalid native shader source.");
	let vertex = context
		.create_shader(None, vertex_source.as_source(), ShaderTypes::Vertex, [])
		.expect("Failed to create benchmark vertex shader. The most likely cause is unsupported shader code.");

	let textures = ShaderResourceDescriptor::new(
		ResourceSlot::new(0),
		ResourceKind::CombinedImageSampler,
		TEXTURES,
		AccessPolicies::READ,
	);
	let layout = ghi::DescriptorSetLayout::new("Benchmark Textures", [textures]);
	let target = context.build_image(
		ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::RenderTarget)
			.extent(Extent::square(64))
			.device_accesses(DeviceAccesses::DeviceOnly),
	);
	// Each pipeline samples its own element of the shared table, so every one of them reads the set.
	let pipelines: Vec<_> = (0..pipelines)
		.map(|index| {
			let glsl = format!("#version 450\n#pragma shader_stage(fragment)\nlayout(set=0,binding=0) uniform sampler2D textures[{TEXTURES}]; layout(location=0) out vec4 output_color; void main() {{ output_color = texture(textures[{index}], vec2(0.5)); }}");
			let msl = format!("#include <metal_stdlib>\nusing namespace metal; struct Textures {{ texture2d<float> textures [[id(0)]][{TEXTURES}]; sampler samplers [[id({TEXTURES})]][{TEXTURES}]; }}; fragment float4 fragment_main(constant Textures& set [[buffer(16)]]) {{ return set.textures[{index}].sample(set.samplers[{index}], float2(0.5)); }}");
			let hlsl = format!("Texture2D<float4> textures[{TEXTURES}] : register(t0, space0); SamplerState samplers[{TEXTURES}] : register(s0, space0); float4 fragment_main() : SV_TARGET0 {{ return textures[{index}].Sample(samplers[{index}], float2(0.5, 0.5)); }}");
			let source = ghi::shader::compile("benchmark fragment", ShaderSource::PlatformNative {
				glsl: &glsl,
				msl: &msl,
				msl_entry_point: "fragment_main",
				hlsl: &hlsl,
				hlsl_entry_point: "fragment_main",
			})
			.expect("Failed to compile benchmark fragment shader. The most likely cause is invalid native shader source.");
			let fragment = context
				.create_shader(
					None,
					source.as_source(),
					ShaderTypes::Fragment,
					[ghi::ShaderDescriptorSet::new(layout.clone())],
				)
				.expect("Failed to create benchmark fragment shader. The most likely cause is unsupported shader resources.");
			context.create_raster_pipeline(ghi::pipelines::raster::Builder::new(
				&[],
				&[],
				&[
					ShaderParameter::new(&vertex, ShaderTypes::Vertex),
					ShaderParameter::new(&fragment, ShaderTypes::Fragment),
				],
				&[AttachmentDescriptor::new(Formats::RGBA8UNORM)],
			))
		})
		.collect();

	// The first two images are the ones streamed in; the rest fill a loaded table.
	let image_count = match table {
		Table::Placeholder => 2,
		Table::Loaded => 2 + TEXTURES as usize,
	};
	let images: Vec<_> = (0..image_count)
		.map(|_| {
			context.build_image(
				ghi::image::Builder::new(Formats::RGBA8UNORM, Uses::Image)
					.extent(Extent::square(1))
					.device_accesses(DeviceAccesses::HostToDevice),
			)
		})
		.collect();
	let sampler = context.build_sampler(ghi::sampler::Builder::new());
	let set = context.create_descriptor_set(None, &layout);
	let write = |image: usize, element: u32| {
		DescriptorWrite::combined_image_sampler_array(set, textures.slot(), images[image], sampler, Layouts::Read, element)
	};
	// Every element holds a texture, so the whole table is resident.
	let initial: Vec<_> = (0..TEXTURES)
		.map(|element| match table {
			Table::Placeholder => write(0, element),
			Table::Loaded => write(2 + element as usize, element),
		})
		.collect();
	context.write(&initial);

	let command = context.queue(queue).create_command_buffer(None);
	let signal = context.create_synchronizer(None, true);
	let extent = Extent::square(64);
	let mut index = 0u64;
	measure(bencher, || {
		context
			.queue(queue)
			.execute(Some(FrameRequest::new(index, signal)), &[], signal, |execution| {
				let frame = execution.frame().unwrap();
				// One texture of the table changes, as when a streamed texture arrives.
				frame.write(&[write(index as usize % 2, (index / 2) as u32 % pipelines.len() as u32)]);

				execution.record(command, |recording| {
					let attachments = [AttachmentInformation::new(
						target,
						Layouts::RenderTarget,
						ghi::LoadOp::Clear(ClearValue::Color(RGBA::black())),
						ghi::StoreOp::Store,
					)];
					let pass = recording.start_render_pass(extent, &attachments);
					for &pipeline in &pipelines {
						pass.bind_raster_pipeline(pipeline)
							.bind_descriptor_sets(&[set])
							.draw(3, 1, 0, 0);
					}
					pass.end_render_pass();
				});
				[]
			});
		// Report completed-frame latency; the wait remains identical across comparisons.
		context.wait();
		index += 1;
	});
	#[cfg(debug_assertions)]
	assert!(
		!context.has_errors(),
		"Shared set benchmark failed. The most likely cause is invalid command or resource synchronization."
	);
}

#[divan::bench(args = [4, 32])]
fn streamed_texture_shared_by_pipelines_completed(bencher: Bencher, pipelines: usize) {
	render(bencher, pipelines, Table::Placeholder);
}

#[divan::bench(args = [4, 32])]
fn streamed_texture_in_loaded_table_completed(bencher: Bencher, pipelines: usize) {
	render(bencher, pipelines, Table::Loaded);
}
