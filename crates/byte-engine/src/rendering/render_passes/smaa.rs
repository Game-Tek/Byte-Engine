use std::io::Read as _;

use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
};

use crate::rendering::{
	RenderPass, Sink,
	render_pass::{RenderPassBuilder, RenderPassReturn, simple_tile},
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
/// Install it after the scene pass that finalizes the desired `main` color input.
/// The pass uses the reference area and search maps for orthogonal and diagonal
/// patterns, applies corner handling, and resolves into a separate `main` target.
/// It intentionally excludes temporal reprojection and multisample modes.
pub struct SmaaPass {
	edge_pass: simple_tile::Pass,
	resolve_pass: simple_tile::Pass,
}

impl SmaaPass {
	/// Creates a sink-local SMAA pass and remaps `main` for later passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder<'_>) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		// Both passes write tile outputs. The builder gives `main`'s replacement the uses that needs, and the edges take
		// them here.
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("SMAA Output"));

		let edge_uses = simple_tile::output_uses(&*render_pass_builder.context());
		let edges = render_pass_builder.create_render_target(
			ghi::image::Builder::new(ghi::Formats::R8UNORM, ghi::Uses::Image | edge_uses).name("SMAA Edges"),
		);

		let context = render_pass_builder.context();
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
		let sampler = context.build_sampler(ghi::sampler::Builder::new());

		let edge_pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"SMAA Edge Detection",
				"byte_engine/rendering/smaa/edge-detection.pipeline",
				&[ghi::Formats::R8UNORM],
			),
		);
		let resolve_pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"SMAA Blend and Neighborhood",
				"byte_engine/rendering/smaa/blend-weights.pipeline",
				&[output.format()],
			),
		);

		let edge_pass = edge_pipeline.bind(
			"SMAA Edge Descriptor Set",
			&[simple_tile::Resource::combined_image_sampler(
				"source",
				source,
				sampler,
				ghi::Layouts::Read,
			)],
			&[simple_tile::Output::new("edges", ghi::BaseImageHandle::from(edges))],
		);
		let resolve_pass = resolve_pipeline.bind(
			"SMAA Resolve Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("source", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("edges", edges, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("area_texture", area_texture, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("search_texture", search_texture, sampler, ghi::Layouts::Read),
			],
			&[simple_tile::Output::new("result", output)],
		);

		Self { edge_pass, resolve_pass }
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
		let edge_pass = self.edge_pass.ready(frame)?;
		let resolve_pass = self.resolve_pass.ready(frame)?;
		let extent = sink.extent();

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("SMAA"),
					|command_buffer| {
						edge_pass.record(command_buffer, extent);
						resolve_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, WorkgroupState, output_slot};

	use super::*;
	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{PIXEL_TILE, assert_rgba_close, empty_image, rgba, texture_2d, tile_lanes};

	const EDGE_SHADER: &str = include_str!("../../../assets/rendering/smaa/edge-detection.besl");
	const RESOLVE_SHADER: &str = include_str!("../../../assets/rendering/smaa/blend-weights.besl");
	/// Builds every lane of the SMAA tile whose top-left pixel is `origin`, over outputs of `extent`.
	fn tile_configs(origin: [u32; 2], extent: [u32; 2]) -> Vec<besl::vm::ExecutionConfig> {
		tile_lanes(
			[origin[0] / PIXEL_TILE, origin[1] / PIXEL_TILE],
			PIXEL_TILE,
			PIXEL_TILE,
			extent,
		)
	}

	/// Executes edge detection with its shared luma cache for one complete tile over outputs of `extent`.
	fn run_edge_tile(source: &mut Texture, result: &mut Texture, origin: [u32; 2], extent: [u32; 2]) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(EDGE_SHADER));
		let configs = tile_configs(origin, extent);
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), source);
		descriptors.bind_image(output_slot(0), result);
		descriptors.bind_workgroup_state(&mut workgroup);
		program
			.run_workgroup(&mut descriptors, &configs)
			.expect("Failed to execute the SMAA edge tile. The most likely cause is invalid shared-cache synchronization.");
	}

	/// Executes the fused reference-weight and neighborhood resolve for one tile over outputs of `extent`.
	fn run_resolve_tile(
		source: &mut Texture,
		edges: &mut Texture,
		area: &mut Texture,
		search: &mut Texture,
		result: &mut Texture,
		origin: [u32; 2],
		extent: [u32; 2],
	) {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(RESOLVE_SHADER));
		let configs = tile_configs(origin, extent);
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), source);
		descriptors.bind_texture(ResourceSlot::new(1), edges);
		descriptors.bind_texture(ResourceSlot::new(2), area);
		descriptors.bind_texture(ResourceSlot::new(3), search);
		descriptors.bind_image(output_slot(0), result);
		descriptors.bind_workgroup_state(&mut workgroup);
		program
			.run_workgroup(&mut descriptors, &configs)
			.expect("Failed to execute the fused SMAA tile. The most likely cause is invalid shared-cache synchronization.");
	}

	/// Builds one VM edge texel using the production R8 bit layout.
	fn packed_edge(west: bool, north: bool) -> [f32; 4] {
		let bits = u32::from(west) | (u32::from(north) << 1);
		[bits as f32 / 3.0, 0.0, 0.0, 0.0]
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

	/// Resolves a 32x32 image whose only edges form an orthogonal line on the boundary before row or column `line`,
	/// ending in crossing edges as in [`smaa_fused_resolve_besl_vm_uses_reference_areas_for_an_orthogonal_line`]. The
	/// source is white before the boundary and black from it on.
	fn resolve_orthogonal_line(line: u32, vertical: bool) -> Texture {
		const EXTENT: u32 = 32;
		// `along` runs with the line and `across` crosses it.
		let index = |along: u32, across: u32| {
			let (x, y) = if vertical { (across, along) } else { (along, across) };
			(y * EXTENT + x) as usize
		};
		let edge = |on_line: bool, crossing: bool| {
			if vertical {
				packed_edge(on_line, crossing)
			} else {
				packed_edge(crossing, on_line)
			}
		};
		let mut edge_texels = vec![[0.0, 0.0, 0.0, 0.0]; (EXTENT * EXTENT) as usize];
		let mut source_texels = vec![[1.0, 1.0, 1.0, 1.0]; (EXTENT * EXTENT) as usize];
		for along in 4..=11 {
			edge_texels[index(along, line)] = edge(true, false);
		}
		edge_texels[index(4, line)] = edge(true, true);
		edge_texels[index(12, line)] = edge(false, true);
		for along in 0..EXTENT {
			for across in line..EXTENT {
				source_texels[index(along, across)] = [0.0, 0.0, 0.0, 1.0];
			}
		}

		let area_bytes = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search_bytes = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let mut source = texture_2d(EXTENT, EXTENT, &source_texels);
		let mut edges = texture_2d(EXTENT, EXTENT, &edge_texels);
		let mut area = vm_lookup_texture(&area_bytes, AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT, 2);
		let mut search = vm_lookup_texture(&search_bytes, SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT, 1);
		let mut result = empty_image(EXTENT, EXTENT);
		for origin in [[0, 0], [16, 0], [0, 16], [16, 16]] {
			run_resolve_tile(&mut source, &mut edges, &mut area, &mut search, &mut result, origin, [EXTENT, EXTENT]);
		}
		result
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
		run_edge_tile(&mut source, &mut edges, [0, 0], [5, 1]);
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
		run_edge_tile(&mut source, &mut edges, [0, 0], [5, 1]);
		assert_rgba_close(rgba(&edges, [2, 0]), [1.0 / 3.0, 0.0, 0.0, 0.0], 0.0);

		// Steps are judged as a tone mapper shows them: a dim shadow step is an edge, a wide highlight step is not.
		for (dark, bright, expected) in [(0.01, 0.04, 1.0 / 3.0), (8.0, 10.0, 0.0)] {
			let step = [dark, dark, bright, bright, bright].map(|value| [value, value, value, 1.0]);
			let mut source = texture_2d(5, 1, &step);
			let mut edges = empty_image(5, 1);
			run_edge_tile(&mut source, &mut edges, [0, 0], [5, 1]);
			assert_rgba_close(rgba(&edges, [2, 0]), [expected, 0.0, 0.0, 0.0], 0.0);
		}

		let north_only = [
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
		];
		let mut source = texture_2d(3, 3, &north_only);
		let mut edges = empty_image(3, 3);
		run_edge_tile(&mut source, &mut edges, [0, 0], [3, 3]);
		assert_rgba_close(rgba(&edges, [1, 1]), [2.0 / 3.0, 0.0, 0.0, 0.0], 0.0);

		let both = [
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
			[1.0, 1.0, 1.0, 1.0],
		];
		let mut source = texture_2d(3, 3, &both);
		let mut edges = empty_image(3, 3);
		run_edge_tile(&mut source, &mut edges, [0, 0], [3, 3]);
		assert_rgba_close(rgba(&edges, [1, 1]), [1.0, 0.0, 0.0, 0.0], 0.0);
	}

	#[test]
	fn smaa_reference_luts_keep_their_canonical_payloads() {
		let area = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");

		assert_eq!(fnv1a(&area), 0x247a58bbba65292d);
		assert_eq!(fnv1a(&search), 0x21c1fcf0aa631065);
	}

	#[test]
	fn smaa_fused_resolve_besl_vm_blends_a_diagonal_staircase() {
		let mut edge_texels = [[0.0, 0.0, 0.0, 0.0]; 81];
		for offset in -3_i32..=3 {
			let x = (4 + offset) as usize;
			let y = (4 - offset) as usize;
			edge_texels[y * 9 + x] = packed_edge(true, true);
		}
		let mut source_texels = [[1.0, 1.0, 1.0, 1.0]; 81];
		source_texels[4 * 9 + 4] = [0.0, 0.0, 0.0, 1.0];
		let area_bytes = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search_bytes = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let mut source = texture_2d(9, 9, &source_texels);
		let mut edges = texture_2d(9, 9, &edge_texels);
		let mut area = vm_lookup_texture(&area_bytes, AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT, 2);
		let mut search = vm_lookup_texture(&search_bytes, SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT, 1);
		let mut result = empty_image(9, 9);
		// One tile covers the whole 9x9 image.
		run_resolve_tile(&mut source, &mut edges, &mut area, &mut search, &mut result, [0, 0], [9, 9]);
		let resolved = rgba(&result, [4, 4]);

		assert!(
			resolved[0] > 0.0 && resolved[1] > 0.0 && resolved[2] > 0.0,
			"The fused reference diagonal lookup must blend the black center toward its white neighbors."
		);
		assert_eq!(resolved[3], 1.0);
	}

	#[test]
	fn smaa_fused_resolve_besl_vm_uses_reference_areas_for_an_orthogonal_line() {
		let mut edge_texels = [[0.0, 0.0, 0.0, 0.0]; 81];
		for x in 1..=7 {
			edge_texels[4 * 9 + x] = packed_edge(false, true);
		}
		edge_texels[4 * 9 + 1] = packed_edge(true, true);
		edge_texels[4 * 9 + 8] = packed_edge(true, false);
		let mut source_texels = [[1.0, 1.0, 1.0, 1.0]; 81];
		source_texels[4 * 9 + 4] = [0.0, 0.0, 0.0, 1.0];
		let area_bytes = decode_lookup_texture(AREA_TEXTURE, AREA_TEXTURE_BYTE_COUNT, "area texture");
		let search_bytes = decode_lookup_texture(SEARCH_TEXTURE, SEARCH_TEXTURE_BYTE_COUNT, "search texture");
		let mut source = texture_2d(9, 9, &source_texels);
		let mut edges = texture_2d(9, 9, &edge_texels);
		let mut area = vm_lookup_texture(&area_bytes, AREA_TEXTURE_WIDTH, AREA_TEXTURE_HEIGHT, 2);
		let mut search = vm_lookup_texture(&search_bytes, SEARCH_TEXTURE_WIDTH, SEARCH_TEXTURE_HEIGHT, 1);
		let mut result = empty_image(9, 9);
		// One tile covers the whole 9x9 image.
		run_resolve_tile(&mut source, &mut edges, &mut area, &mut search, &mut result, [0, 0], [9, 9]);
		let resolved = rgba(&result, [4, 4]);

		assert!(
			resolved[0] > 0.0 && resolved[1] > 0.0 && resolved[2] > 0.0,
			"The fused reference area lookup must blend the black center toward its white neighbors."
		);
		assert_eq!(resolved[3], 1.0);
	}

	/// Verifies pixels beside a tile border resolve exactly as they do inside a tile, so the weights a tile computes
	/// for its bottom look-ahead row and right look-ahead column match the ones the next tile computes for itself.
	#[test]
	fn smaa_fused_resolve_besl_vm_matches_across_tile_borders() {
		for vertical in [false, true] {
			// Row or column 16 starts the second tile, so the pixels just before it read the first tile's look-ahead
			// weights. At 8 both sides of the boundary lie inside the first tile.
			let border = resolve_orthogonal_line(16, vertical);
			let interior = resolve_orthogonal_line(8, vertical);
			let pixel = |across: u32, along: u32| if vertical { [across, along] } else { [along, across] };
			let mut blended = false;
			for along in 0..32 {
				for offset in 0..2 {
					let border_texel = rgba(&border, pixel(15 + offset, along));
					assert_rgba_close(border_texel, rgba(&interior, pixel(7 + offset, along)), 0.0);
					blended |= border_texel[0] > 0.0 && border_texel[0] < 1.0;
				}
			}
			assert!(
				blended,
				"Expected the orthogonal line to blend the pixels beside it (vertical: {vertical}). The most likely cause is a fixture whose edges no longer form a line the reference areas weight."
			);
		}
	}

	#[test]
	fn smaa_fused_resolve_besl_vm_copies_unblended_partial_tiles() {
		let mut source_texels = [[0.0, 0.0, 0.0, 1.0]; 17 * 9];
		for y in 0..9 {
			for x in 0..17 {
				source_texels[y * 17 + x] = [x as f32 / 16.0, y as f32 / 8.0, (x + y) as f32 / 24.0, 1.0];
			}
		}
		let mut source = texture_2d(17, 9, &source_texels);
		let mut edges = empty_image(17, 9);
		let mut area = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut search = texture_2d(1, 1, &[[0.0, 0.0, 0.0, 1.0]]);
		let mut result = empty_image(17, 9);
		// The second tile holds only the last column, so both partial tiles run.
		for origin in [[0, 0], [16, 0]] {
			run_resolve_tile(&mut source, &mut edges, &mut area, &mut search, &mut result, origin, [17, 9]);
		}
		for y in 0..9 {
			for x in 0..17 {
				assert_rgba_close(rgba(&result, [x, y]), source_texels[(y * 17 + x) as usize], 0.0);
			}
		}
	}
}
