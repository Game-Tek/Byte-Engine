use std::io::Read as _;

use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};

use crate::rendering::{
	render_pass::{simple_compute, RenderPassBuilder, RenderPassReturn},
	RenderPass, Sink,
};

const AREA_TEXTURE_WIDTH: u32 = 160;
const AREA_TEXTURE_HEIGHT: u32 = 560;
const AREA_TEXTURE_BYTE_COUNT: usize = AREA_TEXTURE_WIDTH as usize * AREA_TEXTURE_HEIGHT as usize * 2;
const SEARCH_TEXTURE_WIDTH: u32 = 64;
const SEARCH_TEXTURE_HEIGHT: u32 = 16;
const SEARCH_TEXTURE_BYTE_COUNT: usize = SEARCH_TEXTURE_WIDTH as usize * SEARCH_TEXTURE_HEIGHT as usize;
const AREA_TEXTURE: &str = include_str!("../../../assets/rendering/smaa/area-texture.zlib.b64");
const SEARCH_TEXTURE: &str = include_str!("../../../assets/rendering/smaa/search-texture.zlib.b64");

/// Decodes one vendored reference LUT while constructing the sink-local pass.
fn decode_lookup_texture(encoded: &str, expected_byte_count: usize, name: &str) -> Vec<u8> {
	let compact = encoded.bytes().filter(|byte| !byte.is_ascii_whitespace()).collect::<Vec<_>>();
	let compressed = base64::decode(compact).unwrap_or_else(|error| {
		panic!("Failed to decode the SMAA {name}. The most likely cause is a damaged vendored LUT: {error}")
	});
	let mut decoder = flate2::read::ZlibDecoder::new(compressed.as_slice());
	let mut pixels = Vec::with_capacity(expected_byte_count);
	decoder.read_to_end(&mut pixels).unwrap_or_else(|error| {
		panic!("Failed to decompress the SMAA {name}. The most likely cause is a damaged vendored LUT: {error}")
	});
	assert_eq!(
		pixels.len(),
		expected_byte_count,
		"Invalid SMAA {name} size. The most likely cause is a LUT that does not match the reference texture dimensions."
	);
	pixels
}

/// Creates and uploads one immutable reference LUT.
fn create_lookup_texture(
	context: &mut ghi::implementation::Context,
	name: &str,
	format: ghi::Formats,
	extent: utils::Extent,
	pixels: &[u8],
) -> ghi::ImageHandle {
	let image = context.build_image(
		ghi::image::Builder::new(format, ghi::Uses::Image | ghi::Uses::TransferDestination)
			.name(name)
			.extent(extent)
			.device_accesses(ghi::DeviceAccesses::HostToDevice)
			.use_case(ghi::UseCases::STATIC),
	);
	context.get_texture_slice_mut(image).copy_from_slice(pixels);
	context.sync_texture(image);
	image
}

/// The `SmaaPass` struct provides complete spatial SMAA 1x for one render sink.
///
/// Install it after tonemapping and before UI or the final presentation blit.
/// The pass uses the reference area and search maps for orthogonal and diagonal
/// patterns, applies corner handling, and performs the canonical neighborhood
/// resolve. It intentionally excludes temporal reprojection and multisample modes.
pub struct SmaaPass {
	edge_pass: simple_compute::Pass,
	weight_pass: simple_compute::Pass,
	neighborhood_pass: simple_compute::Pass,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	edges: ghi::DynamicImageHandle,
	weights: ghi::DynamicImageHandle,
}

impl SmaaPass {
	/// Creates a sink-local SMAA pass and remaps `main` for later passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let source = render_pass_builder.read_from("main");
		let output = render_pass_builder.render_to("main");

		let context = render_pass_builder.context();
		let edges = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("SMAA Edges")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let weights = context.build_dynamic_image(
			ghi::image::Builder::new(ghi::Formats::RGBA8UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name("SMAA Weights")
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		let area_pixels = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let area_texture = create_lookup_texture(
			context,
			"SMAA Area Texture",
			ghi::Formats::RG8UNORM,
			utils::Extent::rectangle(AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT),
			&area_pixels,
		);
		let search_pixels = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let search_texture = create_lookup_texture(
			context,
			"SMAA Search Texture",
			ghi::Formats::R8UNORM,
			utils::Extent::rectangle(SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT),
			&search_pixels,
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let edge_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"SMAA Edge Detection",
				"byte-engine/rendering/smaa/edge-detection.besl",
				"SMAA Edge Detection Shader",
			),
		)
		.expect("Failed to create the SMAA edge shader. The most likely cause is an incompatible shader interface.");
		let weight_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"SMAA Blend Weights",
				"byte-engine/rendering/smaa/blend-weights.besl",
				"SMAA Blend Weights Shader",
			),
		)
		.expect("Failed to create the SMAA weight shader. The most likely cause is an incompatible shader interface.");
		let neighborhood_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"SMAA Neighborhood Blending",
				"byte-engine/rendering/smaa/neighborhood-blending.besl",
				"SMAA Neighborhood Blending Shader",
			),
		)
		.expect("Failed to create the SMAA blend shader. The most likely cause is an incompatible shader interface.");

		let edge_pass = edge_pipeline
			.bind(
				render_pass_builder,
				"SMAA Edge Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("edges", edges),
				],
			)
			.expect("Failed to bind SMAA edge resources. The most likely cause is a changed BESL binding contract.");
		let weight_pass = weight_pipeline
			.bind(
				render_pass_builder,
				"SMAA Weight Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("edges", edges, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler("area_texture", area_texture, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler(
						"search_texture",
						search_texture,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("weights", weights),
				],
			)
			.expect("Failed to bind SMAA weight resources. The most likely cause is a changed BESL binding contract.");
		let neighborhood_pass = neighborhood_pipeline
			.bind(
				render_pass_builder,
				"SMAA Neighborhood Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler("weights", weights, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result", output),
				],
			)
			.expect("Failed to bind SMAA blend resources. The most likely cause is a changed BESL binding contract.");
		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			edge_pass,
			weight_pass,
			neighborhood_pass,
			bypass_pass,
			edges,
			weights,
		}
	}

	/// Resizes retained intermediate images to the current sink extent.
	fn resize_images(&self, frame: &mut ghi::implementation::Frame, extent: utils::Extent) {
		frame.resize_image(self.edges.into(), extent);
		frame.resize_image(self.weights.into(), extent);
	}
}

impl RenderPass for SmaaPass {
	fn name(&self) -> &'static str {
		"smaa"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extent = sink.extent();
		self.resize_images(frame, extent);
		let edge_pass = self.edge_pass;
		let weight_pass = self.weight_pass;
		let neighborhood_pass = self.neighborhood_pass;

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("SMAA"),
					|command_buffer| {
						edge_pass.record(command_buffer, extent);
						weight_pass.record(command_buffer, extent);
						neighborhood_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.bypass_pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture};
	use resource_management::shader::{
		besl::backends::{glsl::GLSLShaderGenerator, hlsl::HLSLShaderGenerator, msl::MSLShaderGenerator},
		generator::ShaderGenerationSettings,
	};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, empty_image, rgba, run_at, texture_2d};

	const EDGE_SHADER: &str = include_str!("../../../assets/rendering/smaa/edge-detection.besl");
	const WEIGHT_SHADER: &str = include_str!("../../../assets/rendering/smaa/blend-weights.besl");
	const NEIGHBORHOOD_SHADER: &str = include_str!("../../../assets/rendering/smaa/neighborhood-blending.besl");

	/// Executes edge detection with its storage-image interface.
	fn run_edge(source: &mut Texture, result: &mut Texture, coordinate: [u32; 2]) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(EDGE_SHADER));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), source);
		descriptors.bind_image(ResourceSlot::new(1), result);
		run_at(&program, &mut descriptors, coordinate);
	}

	/// Executes reference weight calculation with all three sampled textures.
	fn run_weights(edges: &mut Texture, area: &mut Texture, search: &mut Texture, result: &mut Texture, coordinate: [u32; 2]) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(WEIGHT_SHADER));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), edges);
		descriptors.bind_texture(ResourceSlot::new(1), area);
		descriptors.bind_texture(ResourceSlot::new(2), search);
		descriptors.bind_image(ResourceSlot::new(3), result);
		run_at(&program, &mut descriptors, coordinate);
	}

	/// Executes the sampled neighborhood resolve for a single coordinate.
	fn run_neighborhood(source: &mut Texture, weights: &mut Texture, result: &mut Texture, coordinate: [u32; 2]) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(NEIGHBORHOOD_SHADER));
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), source);
		descriptors.bind_texture(ResourceSlot::new(1), weights);
		descriptors.bind_image(ResourceSlot::new(2), result);
		run_at(&program, &mut descriptors, coordinate);
	}

	/// Expands a normalized byte LUT into the VM's RGBA texture representation.
	fn vm_lookup_texture(bytes: &[u8], width: u32, height: u32, channels: usize) -> Texture {
		let mut texture = Texture::new(width, height).expect("Failed to create an SMAA VM LUT fixture.");
		for index in 0..(width as usize * height as usize) {
			let red = bytes[index * channels] as f32 / 255.0;
			let green = if channels > 1 {
				bytes[index * channels + 1] as f32 / 255.0
			} else {
				0.0
			};
			texture
				.write([(index as u32) % width, (index as u32) / width], [red, green, 0.0, 1.0])
				.expect("Failed to initialize an SMAA VM LUT texel.");
		}
		texture
	}

	/// Calculates a stable FNV-1a fingerprint for vendored reference data.
	fn fnv1a(bytes: &[u8]) -> u64 {
		bytes.iter().fold(0xcbf29ce484222325, |hash, byte| {
			(hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
		})
	}

	#[test]
	fn smaa_edge_besl_vm_rejects_constant_regions_and_keeps_dominant_edges() {
		let constant = [[0.4, 0.4, 0.4, 1.0]; 5];
		let mut source = texture_2d(5, 1, &constant);
		let mut edges = empty_image(5, 1);
		run_edge(&mut source, &mut edges, [2, 0]);
		assert_rgba_close(rgba(&edges, [2, 0]), [0.0, 0.0, 0.0, 0.0], 0.0);

		let colors = [
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[0.8, 0.8, 0.8, 1.0],
			[0.8, 0.8, 0.8, 1.0],
		];
		let mut source = texture_2d(5, 1, &colors);
		let mut edges = empty_image(5, 1);
		run_edge(&mut source, &mut edges, [2, 0]);
		assert_rgba_close(rgba(&edges, [2, 0]), [1.0, 0.0, 0.0, 0.0], 0.0);
	}

	#[test]
	fn smaa_reference_luts_keep_their_canonical_payloads() {
		let area = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		assert_eq!(fnv1a(&area), 0x247a58bbba65292d);
		assert_eq!(fnv1a(&search), 0x21c1fcf0aa631065);
	}

	#[test]
	fn smaa_weight_besl_vm_finds_a_diagonal_staircase() {
		let mut edge_texels = [[0.0, 0.0, 0.0, 0.0]; 81];
		for offset in -3_i32..=3 {
			let x = (4 + offset) as usize;
			let y = (4 - offset) as usize;
			edge_texels[y * 9 + x] = [1.0, 1.0, 0.0, 0.0];
		}
		let area_bytes = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search_bytes = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let mut edges = texture_2d(9, 9, &edge_texels);
		let mut area = vm_lookup_texture(&area_bytes, AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT, 2);
		let mut search = vm_lookup_texture(&search_bytes, SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT, 1);
		let mut weights = empty_image(9, 9);
		run_weights(&mut edges, &mut area, &mut search, &mut weights, [4, 4]);
		let diagonal_weights = rgba(&weights, [4, 4]);
		assert!(
			diagonal_weights[0] + diagonal_weights[1] > 0.0,
			"The reference diagonal lookup must produce a nonzero north-edge weight."
		);
		assert_eq!(diagonal_weights[2] + diagonal_weights[3], 0.0);
	}

	#[test]
	fn smaa_weight_besl_vm_uses_reference_areas_for_an_orthogonal_line() {
		let mut edge_texels = [[0.0, 0.0, 0.0, 0.0]; 81];
		for x in 1..=7 {
			edge_texels[4 * 9 + x][1] = 1.0;
		}
		edge_texels[4 * 9 + 1][0] = 1.0;
		edge_texels[4 * 9 + 8][0] = 1.0;
		let area_bytes = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search_bytes = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let mut edges = texture_2d(9, 9, &edge_texels);
		let mut area = vm_lookup_texture(&area_bytes, AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT, 2);
		let mut search = vm_lookup_texture(&search_bytes, SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT, 1);
		let mut weights = empty_image(9, 9);
		run_weights(&mut edges, &mut area, &mut search, &mut weights, [4, 4]);
		let line_weights = rgba(&weights, [4, 4]);
		assert!(
			line_weights[0] + line_weights[1] > 0.0,
			"The reference area lookup must classify a bounded orthogonal line."
		);
		assert_eq!(line_weights[2] + line_weights[3], 0.0);
	}

	#[test]
	fn smaa_neighborhood_besl_vm_blends_toward_the_strongest_neighbor() {
		let source_texels = [
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[1.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
			[0.0, 0.0, 1.0, 1.0],
		];
		let mut weight_texels = [[0.0, 0.0, 0.0, 0.0]; 9];
		weight_texels[4][2] = 0.5;
		let mut source = texture_2d(3, 3, &source_texels);
		let mut weights = texture_2d(3, 3, &weight_texels);
		let mut result = empty_image(3, 3);
		run_neighborhood(&mut source, &mut weights, &mut result, [1, 1]);
		assert_rgba_close(rgba(&result, [1, 1]), [0.5, 0.0, 0.5, 1.0], 1e-6);
	}

	/// Verifies every production SMAA stage remains portable across the supported BESL backends.
	#[test]
	fn smaa_shaders_lower_for_all_backends() {
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		for (name, shader) in [
			("edge detection", EDGE_SHADER),
			("blend weights", WEIGHT_SHADER),
			("neighborhood blending", NEIGHBORHOOD_SHADER),
		] {
			let main = simple_compute::compile_test_program(shader);
			GLSLShaderGenerator::new()
				.generate(&settings, &main)
				.unwrap_or_else(|()| panic!("Failed to lower SMAA {name} BESL to GLSL."));
			HLSLShaderGenerator::new()
				.generate(&settings, &main)
				.unwrap_or_else(|()| panic!("Failed to lower SMAA {name} BESL to HLSL."));
			MSLShaderGenerator::new()
				.generate(&settings, &main)
				.unwrap_or_else(|()| panic!("Failed to lower SMAA {name} BESL to MSL."));
		}
	}

	/// Verifies every production SMAA stage compiles with Apple's native Metal compiler.
	#[cfg(target_os = "macos")]
	#[test]
	fn smaa_shaders_compile_to_native_metal() {
		let settings = ShaderGenerationSettings::compute(utils::Extent::square(8));
		for (name, shader) in [
			("smaa-edge-detection", EDGE_SHADER),
			("smaa-blend-weights", WEIGHT_SHADER),
			("smaa-neighborhood-blending", NEIGHBORHOOD_SHADER),
		] {
			let main = simple_compute::compile_test_program(shader);
			let source = MSLShaderGenerator::new()
				.generate(&settings, &main)
				.unwrap_or_else(|()| panic!("Failed to lower production {name} BESL to MSL."));
			resource_management::shader::msl_shader_compiler::compile_msl_source_to_metallib(&source, name)
				.unwrap_or_else(|error| panic!("Failed to compile production {name} MSL: {error}"));
		}
	}
}
