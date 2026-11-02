use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute, simple_tile},
	},
};

const MAX_BLOOM_LEVELS: u32 = 6;

/// The `BloomPassSettings` struct defines the intent and shaping controls for a reusable HDR bloom stage.
///
/// Pass it to [`crate::application::graphics::setup_bloom_render_pass`] or [`BloomPass::with_settings`].
#[derive(Clone, Copy, Debug)]
pub struct BloomPassSettings {
	/// Share of each pixel's light that moves into the glow's tail, from `0.0`, which passes the scene through, to
	/// `1.0`. The glow adds no light: what the tail gains, the pixel it came from loses.
	pub scatter: f32,
	/// How fast the tail falls off with distance, as the power `p` of about `1 / distance^p`. `2.0` reaches furthest,
	/// with every pyramid level carrying the same share; `3.0` keeps the glow close to its source.
	pub tail_exponent: f32,
	/// Share of each red, green, and blue channel's light that moves into its halo, from `0.0` to one less the
	/// scatter. Film scatters red light back from its base, so `[0.02, 0.006, 0.0]` gives a faint red-orange fringe;
	/// the default of zero in every channel turns halation off. Each channel's halo comes from that channel's light.
	pub halation: [f32; 3],
	/// Radius of each channel's halo as a share of the sink's height. The halo centers on the pyramid level whose
	/// texels span the radius, so it keeps its size relative to the frame, in steps of one level.
	pub halation_radius: [f32; 3],
	/// Scene-linear brightness, after exposure, that a texel is held to before it spreads. A highlight's tail is as
	/// bright as the highlight, so a low ceiling starves it. `4096.0` is twelve stops above display white and keeps a
	/// mirror reflection of the sun, at the half-float limit, from hazing the whole frame.
	pub max_brightness: f32,
	/// Width of each upsample blur in texels of the level being blurred. Larger values spread the glow further.
	pub radius: f32,
	/// Pyramid depth, from `1` to `6`. Each level halves the resolution and doubles the glow's reach.
	pub levels: u32,
}

impl Default for BloomPassSettings {
	fn default() -> Self {
		Self {
			scatter: 0.06,
			tail_exponent: 2.0,
			halation: [0.0; 3],
			halation_radius: [0.005; 3],
			max_brightness: 4096.0,
			radius: 1.0,
			levels: 6,
		}
	}
}

impl BloomPassSettings {
	fn resolved_level_count(self) -> usize {
		self.levels.clamp(1, MAX_BLOOM_LEVELS) as usize
	}
}

/// The `BloomShaderData` struct mirrors the `BloomParameters` layout shared by the bloom extract, upsample, and
/// composite shaders.
///
/// [`crate::rendering::render_passes::lens_flare::LensFlarePass`] reuses the extract and composite shaders, so it
/// fills this layout too.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct BloomShaderData {
	/// Threshold, soft knee, composite strength, and brightness ceiling. Bloom thresholds nothing; the lens flare
	/// thresholds its highlights and adds itself with its intensity.
	pub(crate) prefilter: [f32; 4],
	/// Upsample radius in `x`; the other lanes are unused.
	pub(crate) filter: [f32; 4],
	/// A climb's per-channel weight for its same-resolution level, its share of the glow.
	pub(crate) same_weight: [f32; 4],
	/// A climb's per-channel weight for the blurred levels below it: one, except the deepest climb, which gives the
	/// coarsest level its share.
	pub(crate) low_weight: [f32; 4],
	/// The share of each channel's light the composite moves into the glow, the sum of every level's weight.
	pub(crate) scatter: [f32; 4],
}

impl BloomShaderData {
	/// Returns the shader data of the climb that builds `level`, of `level_count`, from the per-level `weights` that
	/// [`level_weights`] returns. The climb that builds level `0` is the composite's, and the prefilter reads it too.
	fn climb(settings: BloomPassSettings, weights: &LevelWeights, level: usize, level_count: usize) -> Self {
		let low = if level_count == 1 {
			// A one-level pyramid has nothing below its top, and the composite reads a stand-in there.
			[0.0; 3]
		} else if level == level_count - 2 {
			weights[level_count - 1]
		} else {
			[1.0; 3]
		};
		let scatter = weights[..level_count].iter().fold([0.0; 3], |sum, weight| {
			[sum[0] + weight[0], sum[1] + weight[1], sum[2] + weight[2]]
		});
		let rgb = |color: [f32; 3]| [color[0], color[1], color[2], 0.0];
		Self {
			prefilter: [0.0, 0.0, 0.0, settings.max_brightness.max(0.0)],
			filter: [settings.radius.max(0.0), 0.0, 0.0, 0.0],
			same_weight: rgb(weights[level]),
			low_weight: rgb(low),
			scatter: rgb(scatter),
		}
	}
}

/// Each pyramid level's per-channel share of the glow.
type LevelWeights = [[f32; 3]; MAX_BLOOM_LEVELS as usize];

/// Returns each level's per-channel weight for a sink `sink_height` pixels tall: the scatter spread over the tail,
/// level `k` in proportion to `r^k` with `r = 2^(2 - p)`, plus each channel's halation spread over a bump centered on
/// the level whose texels span its radius. Each spread sums to one before its share scales it, so a channel's weights
/// sum to its scatter plus its halation.
fn level_weights(settings: BloomPassSettings, level_count: usize, sink_height: u32) -> LevelWeights {
	let scatter = settings.scatter.clamp(0.0, 1.0);
	let ratio = 2f32.powf(2.0 - settings.tail_exponent);
	let tail_sum: f32 = (0..level_count).map(|level| ratio.powi(level as i32)).sum();
	let last_level = (level_count - 1) as f32;
	let mut weights = [[0.0; 3]; MAX_BLOOM_LEVELS as usize];
	for channel in 0..3 {
		let halation = settings.halation[channel].clamp(0.0, 1.0 - scatter);
		// Level `k`'s texels span `2^(k + 1)` pixels. Holding the center inside the pyramid keeps a level within half a
		// level of it, so the bump never sums to zero.
		let radius = settings.halation_radius[channel] * sink_height as f32;
		let center = (radius / 2.0).max(f32::MIN_POSITIVE).log2().clamp(0.0, last_level);
		let halo = |level: usize| (-2.0 * (level as f32 - center).powi(2)).exp();
		let halo_sum: f32 = (0..level_count).map(halo).sum();
		for (level, weight) in weights[..level_count].iter_mut().enumerate() {
			weight[channel] = scatter * ratio.powi(level as i32) / tail_sum + halation * halo(level) / halo_sum;
		}
	}
	weights
}

/// The `BloomPass` struct creates a reusable pre-tonemap glow stage that can feed later post-processing.
///
/// It models the lens's point spread function: it moves a share of every pixel's light into a long, faint tail and
/// adds none, so highlights glow over darker neighbors instead of growing brighter. Film halation, a halo of a set
/// radius per channel, rides the same pyramid as per-channel level weights, so it costs no extra pass. The pass prefilters the HDR `main`
/// target into a half-resolution pyramid with Jimenez's 13-tap filter, using a brightness-weighted average on the
/// first level so single bright texels do not flicker, then climbs back up with a tent blur per level, weighting each
/// level by its share of the tail and halo. The last climb and the blend into the scene share one dispatch, so the top level never
/// reaches memory. The prefilter and the composite, the two steps at the sink's half and full resolution, are tile
/// shaders, so where the platform runs tile functions their targets stay losslessly compressed. The smaller levels run
/// as compute dispatches in one encoder: each tile pass is a render pass of its own, which costs more than a small level
/// saves. Install it after the passes that write scene light and before tone mapping. Toggle it at runtime with the
/// `render.pass.bloom` parameter.
pub struct BloomPass {
	settings: BloomPassSettings,
	/// One buffer per climb, indexed by the level it builds. The prefilter reads level `0`'s, the composite's.
	parameters: Vec<ghi::DynamicBufferHandle<BloomShaderData>>,
	extract_pass: simple_tile::Pass,
	downsample_passes: Vec<simple_compute::Pass>,
	upsample_passes: Vec<simple_compute::Pass>,
	/// Builds the top level from the one below it and adds it onto the scene.
	composite_pass: simple_tile::Pass,
	/// The number of pyramid levels, each a render target the renderer sizes with the sink.
	level_count: usize,
}

impl Entity for BloomPass {}

impl BloomPass {
	/// Creates a bloom pass with the default glow shaping parameters.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, BloomPassSettings::default())
	}

	/// Creates a bloom pass with caller-supplied settings and remaps `main` for downstream passes.
	// Keep the paired downsample and upsample resource graph together so level indices remain symmetric.
	#[allow(clippy::too_many_lines)]
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: BloomPassSettings) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		// The builder gives the replacement for `main` the uses a tile pass writes it with.
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("Bloom Output"));

		let level_count = settings.resolved_level_count();
		// The prefilter writes the first level as a tile output and compute dispatches write the rest.
		let output_uses = simple_tile::output_uses(render_pass_builder.context());
		// Each pyramid level halves the previous one, starting at half the sink resolution.
		let mut pyramid_target = |name, level: usize| -> ghi::BaseImageHandle {
			let write_uses = if level == 0 { output_uses } else { ghi::Uses::Storage };
			render_pass_builder
				.create_scaled_render_target(
					ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Image | write_uses).name(name),
					level_divisor(level),
				)
				.into()
		};
		let downsample_images = [
			"Bloom Downsample 0",
			"Bloom Downsample 1",
			"Bloom Downsample 2",
			"Bloom Downsample 3",
			"Bloom Downsample 4",
			"Bloom Downsample 5",
		][..level_count]
			.iter()
			.enumerate()
			.map(|(level, name)| pyramid_target(name, level))
			.collect::<Vec<_>>();
		// Every level between the top and the coarsest is climbed into its own image. The coarsest level has nothing
		// below it, and the composite builds the top level in workgroup memory.
		let upsample_images = ["Bloom Upsample 1", "Bloom Upsample 2", "Bloom Upsample 3", "Bloom Upsample 4"]
			[..level_count.saturating_sub(2)]
			.iter()
			.enumerate()
			.map(|(index, name)| pyramid_target(name, index + 1))
			.collect::<Vec<_>>();
		// The image that holds a level's finished glow: the coarsest level as downsampled, the others as climbed.
		let finished = |level: usize| {
			if level == level_count - 1 {
				downsample_images[level]
			} else {
				upsample_images[level - 1]
			}
		};

		let context = render_pass_builder.context();
		// Every level but the coarsest is climbed, and a one-level pyramid still composites its top.
		let parameters = (0..level_count.saturating_sub(1).max(1))
			.map(|_| {
				context.build_dynamic_buffer(
					ghi::buffer::Builder::new(ghi::Uses::Storage)
						.name("Bloom Parameters")
						.device_accesses(ghi::DeviceAccesses::HostToDevice),
				)
			})
			.collect::<Vec<_>>();
		let sampler = context.build_sampler(ghi::sampler::Builder::new());

		let pyramid_formats = [crate::rendering::SCENE_COLOR_FORMAT];
		let extract_pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Bloom Extract",
				"byte_engine/rendering/bloom/extract.pipeline",
				&pyramid_formats,
			),
		);
		let downsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Downsample", "byte_engine/rendering/bloom/downsample.pipeline"),
		);
		let upsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Upsample", "byte_engine/rendering/bloom/upsample.pipeline"),
		);
		let composite_pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Bloom Upsample Composite",
				"byte_engine/rendering/bloom/upsample-composite.pipeline",
				&[output.format()],
			),
		);

		let extract_pass = extract_pipeline.bind(
			"Bloom Extract Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("bloom_parameters", parameters[0]),
			],
			&[simple_tile::Output::new("result", downsample_images[0])],
		);

		let downsample_passes = (1..level_count)
			.map(|index| {
				downsample_pipeline.bind(
					"Bloom Downsample Descriptor Set",
					&[
						simple_compute::Resource::combined_image_sampler(
							"source_texture",
							downsample_images[index - 1],
							sampler,
							ghi::Layouts::Read,
						),
						simple_compute::Resource::image("result_texture", downsample_images[index]),
					],
				)
			})
			.collect::<Vec<_>>();

		let upsample_passes = (1..level_count.saturating_sub(1))
			.rev()
			.map(|level| {
				upsample_pipeline.bind(
					"Bloom Upsample Descriptor Set",
					&[
						simple_compute::Resource::combined_image_sampler(
							"low_resolution_texture",
							finished(level + 1),
							sampler,
							ghi::Layouts::Read,
						),
						simple_compute::Resource::combined_image_sampler(
							"high_resolution_texture",
							downsample_images[level],
							sampler,
							ghi::Layouts::Read,
						),
						simple_compute::Resource::image("result_texture", upsample_images[level - 1]),
						simple_compute::Resource::buffer("bloom_parameters", parameters[level]),
					],
				)
			})
			.collect::<Vec<_>>();

		// A one-level pyramid has nothing below its top; its climb weights the level bound there by zero.
		let below_top = if level_count == 1 { downsample_images[0] } else { finished(1) };
		let composite_pass = composite_pipeline.bind(
			"Bloom Upsample Composite Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler("low_resolution_texture", below_top, sampler, ghi::Layouts::Read),
				simple_tile::Resource::combined_image_sampler(
					"high_resolution_texture",
					downsample_images[0],
					sampler,
					ghi::Layouts::Read,
				),
				simple_tile::Resource::buffer("bloom_parameters", parameters[0]),
			],
			&[simple_tile::Output::new("result", output)],
		);

		Self {
			settings,
			parameters,
			extract_pass,
			downsample_passes,
			upsample_passes,
			composite_pass,
			level_count,
		}
	}

	/// Writes each climb's weights for a sink `sink_height` pixels tall into its per-frame parameter buffer before
	/// dispatch. The halation radius is a share of the sink's height, so the weights follow the sink's size.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame, sink_height: u32) {
		let weights = level_weights(self.settings, self.level_count, sink_height);
		for (level, parameters) in self.parameters.iter().enumerate() {
			*frame.get_mut_dynamic_buffer_slice(*parameters) =
				BloomShaderData::climb(self.settings, &weights, level, self.level_count);
		}
	}
}

impl RenderPass for BloomPass {
	fn name(&self) -> &'static str {
		"bloom"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extract_pass = self.extract_pass.ready(frame)?;
		let downsample_passes = ready_in(&mut self.downsample_passes, frame, frame_allocator)?;
		let upsample_passes = ready_in(&mut self.upsample_passes, frame, frame_allocator)?;
		let composite_pass = self.composite_pass.ready(frame)?;
		let extent = sink.extent();

		self.write_parameters(frame, extent.height());

		let level_count = self.level_count;

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("Bloom"),
					|command_buffer| {
						// The tile passes run over their output's extent, and the compute dispatches over one lane per texel.
						extract_pass.record(command_buffer, bloom_extent(extent, 0));

						for (index, pass) in downsample_passes.iter().enumerate() {
							pass.record(command_buffer, bloom_extent(extent, index + 1));
						}

						for (level, pass) in (1..level_count.saturating_sub(1)).rev().zip(upsample_passes.iter()) {
							pass.record(command_buffer, bloom_extent(extent, level));
						}

						composite_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}
}

/// Returns the resolution divisor of a pyramid level: level 0 is half the sink resolution.
fn level_divisor(level: usize) -> u32 {
	2 << level
}

/// Readies every pass into the frame allocator, or returns `None` while any of their pipelines is unavailable.
fn ready_in<'a>(
	passes: &mut [simple_compute::Pass],
	frame: &mut ghi::implementation::Frame,
	frame_allocator: &'a bumpalo::Bump,
) -> Option<&'a [simple_compute::ReadyPass]> {
	let mut ready = bumpalo::collections::Vec::with_capacity_in(passes.len(), frame_allocator);
	for pass in passes {
		ready.push(pass.ready(frame)?);
	}
	Some(ready.into_bump_slice())
}

/// Returns the extent of a pyramid level, matching the size the renderer gives its render target.
fn bloom_extent(extent: Extent, level: usize) -> Extent {
	extent.scaled_down(level_divisor(level))
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ExecutableProgram, ResourceSlot, Texture, Value, WorkgroupState, output_slot};

	use super::*;
	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, assert_rgba_close, buffer, empty_image, rgba, run_at, run_pixel, texture_2d, tile_lanes, tiles,
	};

	static BLOOM_EXTRACT_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("bloom/extract.besl"));
	const BLOOM_DOWNSAMPLE_BESL: &str = include_str!("../../../assets/rendering/bloom/downsample.besl");
	static BLOOM_UPSAMPLE_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("bloom/upsample.besl"));
	static BLOOM_COMPOSITE_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("bloom/composite.besl"));
	static BLOOM_UPSAMPLE_COMPOSITE_BESL: std::sync::LazyLock<&str> =
		std::sync::LazyLock::new(|| crate::rendering::shader_vm_test::asset_source("bloom/upsample-composite.besl"));

	/// A prefilter ceiling above any test input, so it does not take part.
	const NO_CEILING: f32 = 65504.0;
	/// Climb weights that add both levels as they are.
	const UNWEIGHTED: ([f32; 3], [f32; 3]) = ([1.0; 3], [1.0; 3]);

	/// The tile and lanes per tile side of the extract, as `bloom/extract.besl.bead` declares them.
	const EXTRACT_TILE: (u32, u32) = (16, 8);
	/// The tile and lanes per tile side of the upsample composite, as `bloom/upsample-composite.besl.bead` declares
	/// them.
	const UPSAMPLE_COMPOSITE_TILE: (u32, u32) = (32, 16);

	/// Runs the lanes of tile `position`, `tile` pixels wide with `width` lanes along each side, over outputs of
	/// `extent`.
	fn run_tile(
		program: &ExecutableProgram,
		descriptors: &mut DescriptorBindings<'_>,
		position: [u32; 2],
		(tile, width): (u32, u32),
		extent: [u32; 2],
	) {
		program.run_workgroup(descriptors, &tile_lanes(position, tile, width, extent)).expect(
			"Failed to execute a bloom tile with the BESL VM. The most likely cause is missing runtime support or invalid workgroup memory synchronization.",
		);
	}

	/// Runs the prefilter over a 2x2 source into one texel.
	fn extract(source_texels: &[[f32; 4]; 4], threshold: f32, soft_knee: f32, max_brightness: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_EXTRACT_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([threshold, soft_knee, 0.0, max_brightness]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut source = texture_2d(2, 2, source_texels);
		let mut result = empty_image(1, 1);
		let mut workgroup = WorkgroupState::new();
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(output_slot(0), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_workgroup_state(&mut workgroup);
		run_tile(&program, &mut descriptors, [0, 0], EXTRACT_TILE, [1, 1]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Runs the prefilter over a whole `size` by `size` source, every tile of it, without threshold or ceiling. Each
	/// lane filters a 2x2 block of destination texels, so a tile's 8x8 lanes cover its 16x16 texels.
	fn extract_image(size: u32, source_texels: &[[f32; 4]]) -> Texture {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_EXTRACT_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, 0.0, NO_CEILING]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut source = texture_2d(size, size, source_texels);
		let mut result = empty_image(size / 2, size / 2);
		let extent = [size / 2, size / 2];
		for position in tiles(EXTRACT_TILE.0, extent) {
			let mut workgroup = WorkgroupState::new();
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut source);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			descriptors.bind_workgroup_state(&mut workgroup);
			run_tile(&program, &mut descriptors, position, EXTRACT_TILE, extent);
		}
		result
	}

	/// Verifies that the prefilter is the same centered filter at every destination texel: the glow around a bright
	/// block is mirror-symmetric, and moving the block by whole destination texels moves the glow unchanged, whether
	/// the filter's footprint stays inside one lane's 2x2 texels, spans several lanes, or straddles the next workgroup's
	/// tile.
	#[test]
	fn bloom_extract_besl_vm_filters_alike_across_tiles() {
		const SIZE: u32 = 48;
		// Glow sampled from two destination texels before the block to two after, in both axes.
		let glow_around = |block: u32| -> Vec<[f32; 4]> {
			let texels = (0..SIZE * SIZE)
				.map(|index| {
					let (x, y) = (index % SIZE / 2, index / SIZE / 2);
					if x == block && y == block {
						[8.0, 4.0, 2.0, 1.0]
					} else {
						[0.5, 0.25, 0.125, 1.0]
					}
				})
				.collect::<Vec<_>>();
			let result = extract_image(SIZE, &texels);
			(0..25)
				.map(|index| rgba(&result, [block + index % 5 - 2, block + index / 5 - 2]))
				.collect()
		};

		// The 13-tap footprint is centered on the block under its destination texel, so the glow mirrors across both
		// axes and the diagonal.
		let reference = glow_around(5);
		for index in 0..25 {
			let (x, y) = (index % 5, index / 5);
			for mirrored in [(4 - x) + y * 5, x + (4 - y) * 5, y + x * 5] {
				assert_rgba_close(reference[mirrored], reference[index], 1e-6);
			}
		}

		// Each lane filters two destination texels along each axis, and texel 16 starts the second tile, so
		// these blocks put the footprint on either side of a lane's first texel and of the tile edge.
		for block in [6, 7, 14, 15, 16, 17] {
			for (actual, expected) in glow_around(block).into_iter().zip(&reference) {
				assert_rgba_close(actual, *expected, 1e-6);
			}
		}
	}

	/// Verifies threshold rejection and soft-knee extraction through the production bloom program.
	#[test]
	fn bloom_extract_besl_vm_applies_threshold_and_soft_knee() {
		for (source_color, expected) in [
			([0.25, 0.2, 0.1, 0.25], [0.0, 0.0, 0.0, 1.0]),
			([2.0, 1.0, 0.5, 0.25], [1.0, 0.5, 0.25, 1.0]),
		] {
			assert_rgba_close(extract(&[source_color; 4], 1.0, 0.5, NO_CEILING), expected, 1e-5);
		}
	}

	/// Verifies that no texel glows above the configured ceiling.
	#[test]
	fn bloom_extract_besl_vm_holds_texels_to_max_brightness() {
		let result = extract(&[[100.0, 100.0, 100.0, 1.0]; 4], 0.0, 0.0, 64.0);

		assert_rgba_close(result, [64.0, 64.0, 64.0, 1.0], 1e-3);
	}

	/// Verifies that a non-finite scene texel cannot poison the glow around it.
	#[test]
	fn bloom_extract_besl_vm_survives_non_finite_input() {
		for bad in [f32::INFINITY, f32::NAN] {
			let source = [
				[bad, bad, bad, 1.0],
				[1.0, 1.0, 1.0, 1.0],
				[1.0, 1.0, 1.0, 1.0],
				[1.0, 1.0, 1.0, 1.0],
			];
			let result = extract(&source, 0.0, 0.0, NO_CEILING);
			assert!(result.iter().all(|channel| channel.is_finite()), "{bad}: {result:?}");
		}
	}

	/// Verifies that the prefilter holds a single bright texel, a firefly, under one half whatever its brightness, a
	/// saturated one as hard as a white one, while a uniform block keeps all its light. The clamp-addressed 2x2 source
	/// repeats the firefly in the filter's edge blocks, so it lands above the third a truly lone texel would.
	#[test]
	fn bloom_extract_besl_vm_suppresses_fireflies() {
		// One texel lit in a block of four.
		let lone = |color: [f32; 3]| {
			[
				[color[0], color[1], color[2], 1.0],
				[0.0, 0.0, 0.0, 1.0],
				[0.0, 0.0, 0.0, 1.0],
				[0.0, 0.0, 0.0, 1.0],
			]
		};
		let uniform = [[1.0, 1.0, 1.0, 1.0]; 4];

		let white = extract(&lone([1024.0, 1024.0, 1024.0]), 0.0, 0.0, NO_CEILING);
		let blue = extract(&lone([0.0, 0.0, 1024.0]), 0.0, 0.0, NO_CEILING);

		assert_rgba_close(extract(&uniform, 0.0, 0.0, NO_CEILING), [1.0, 1.0, 1.0, 1.0], 1e-5);
		assert!(white[0] > 0.0 && white[0] < 0.5, "{white:?}");
		assert!((blue[2] - white[2]).abs() < 1e-4, "{blue:?} {white:?}");
	}

	/// Runs the pyramid downsample over a 2x2 source into one texel.
	fn downsample(source_texels: &[[f32; 4]; 4]) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(BLOOM_DOWNSAMPLE_BESL));
		let mut source = texture_2d(2, 2, source_texels);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies that the downsample filter's weights sum to one and keep a block's energy without luma weighting.
	#[test]
	fn bloom_downsample_besl_vm_preserves_block_energy() {
		let uniform = [[0.25, 0.5, 0.75, 1.0]; 4];
		let firefly = [
			[4.0, 4.0, 4.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
		];

		assert_rgba_close(downsample(&uniform), [0.25, 0.5, 0.75, 1.0], 1e-5);
		assert_rgba_close(downsample(&firefly), [1.0, 1.0, 1.0, 1.0], 1e-5);
	}

	/// Runs one upsample step from a `low_size` level onto a `high_size` level with the given radius and per-channel
	/// weights for the same-resolution and the lower level, returning every result texel in row-major order.
	fn upsample(
		low_size: u32,
		low_texels: &[[f32; 4]],
		high_size: u32,
		high_texels: &[[f32; 4]],
		radius: f32,
		(same_weight, low_weight): ([f32; 3], [f32; 3]),
	) -> Vec<[f32; 4]> {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_UPSAMPLE_BESL));
		let parameter_slot = ResourceSlot::new(3);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("filter", 0, Value::Vec4F([radius, 0.0, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		for (field, weight) in [("same_weight", same_weight), ("low_weight", low_weight)] {
			parameters
				.write_indexed(field, 0, Value::Vec4F([weight[0], weight[1], weight[2], 0.0]))
				.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		}
		let mut low = texture_2d(low_size, low_size, low_texels);
		let mut high = texture_2d(high_size, high_size, high_texels);
		let mut result = empty_image(high_size, high_size);
		for y in 0..high_size {
			for x in 0..high_size {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut low);
				descriptors.bind_texture(ResourceSlot::new(1), &mut high);
				descriptors.bind_image(ResourceSlot::new(2), &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}
		(0..high_size * high_size)
			.map(|index| rgba(&result, [index % high_size, index / high_size]))
			.collect()
	}

	/// Reads `texels`, `size` texels square, at continuous texel coordinates where integers are texel centers, the way
	/// a bilinear, clamp-addressed sampler does.
	fn bilinear(texels: &[[f32; 4]], size: u32, x: f32, y: f32) -> [f32; 3] {
		let texel = |column: f32, row: f32| {
			let clamp = |coordinate: f32| coordinate.clamp(0.0, (size - 1) as f32) as u32;
			texels[(clamp(row) * size + clamp(column)) as usize]
		};
		let (left, top) = (x.floor(), y.floor());
		let (fraction_x, fraction_y) = (x - left, y - top);
		std::array::from_fn(|channel| {
			let upper = texel(left, top)[channel] * (1.0 - fraction_x) + texel(left + 1.0, top)[channel] * fraction_x;
			let lower =
				texel(left, top + 1.0)[channel] * (1.0 - fraction_x) + texel(left + 1.0, top + 1.0)[channel] * fraction_x;
			upper * (1.0 - fraction_y) + lower * fraction_y
		})
	}

	/// Verifies that each climbed texel is the same-resolution texel plus the lower level blurred with a 3x3 tent of
	/// bilinear taps `radius` lower-level texels apart, weighted 4, 2, and 1 out of 16, both for radii whose taps fold
	/// into four and for wider ones, at every phase between the levels and along the clamped borders.
	#[test]
	fn bloom_upsample_besl_vm_climbs_with_a_bilinear_tent() {
		const LOW_SIZE: u32 = 5;
		const HIGH_SIZE: u32 = 10;
		const TENT: [f32; 3] = [0.25, 0.5, 0.25];
		let low = varied_texels(LOW_SIZE * LOW_SIZE, 1.0);
		let high = varied_texels(HIGH_SIZE * HIGH_SIZE, 2.0);

		for radius in [0.0, 0.3, 0.5, 0.75, 1.0, 1.5, 2.0] {
			let climbed = upsample(LOW_SIZE, &low, HIGH_SIZE, &high, radius, UNWEIGHTED);
			for (index, actual) in climbed.into_iter().enumerate() {
				let (x, y) = ((index as u32 % HIGH_SIZE) as f32, (index as u32 / HIGH_SIZE) as f32);
				let scale = LOW_SIZE as f32 / HIGH_SIZE as f32;
				let (center_x, center_y) = ((x + 0.5) * scale - 0.5, (y + 0.5) * scale - 0.5);
				let mut expected = [high[index][0], high[index][1], high[index][2], 1.0];
				for (row, row_weight) in TENT.iter().enumerate() {
					for (column, column_weight) in TENT.iter().enumerate() {
						let tap = bilinear(
							&low,
							LOW_SIZE,
							center_x + (column as f32 - 1.0) * radius,
							center_y + (row as f32 - 1.0) * radius,
						);
						for channel in 0..3 {
							expected[channel] += tap[channel] * row_weight * column_weight;
						}
					}
				}
				assert_rgba_close(actual, expected, 1e-5);
			}
		}
	}

	/// Verifies that upsampling adds the blurred lower level onto the same-resolution level, each weighted per channel.
	#[test]
	fn bloom_upsample_besl_vm_combines_both_levels() {
		let low = [[0.1, 0.2, 0.3, 0.0]];
		let high = [[0.4, 0.5, 0.6, 0.0]];

		let full = upsample(1, &low, 1, &high, 1.0, UNWEIGHTED);
		let weighted = upsample(1, &low, 1, &high, 1.0, ([1.0, 0.5, 0.0], [1.0, 0.5, 2.0]));

		assert_rgba_close(full[0], [0.5, 0.7, 0.9, 1.0], 1e-6);
		assert_rgba_close(weighted[0], [0.5, 0.35, 0.6, 1.0], 1e-6);
	}

	/// Verifies that the tent blur spreads a lower-level texel to its neighbors, and that a zero radius does not.
	#[test]
	fn bloom_upsample_besl_vm_tent_reach_follows_radius() {
		let low = [
			[1.0, 1.0, 1.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
		];
		let high = [[0.0, 0.0, 0.0, 1.0]; 4];

		let spread = upsample(2, &low, 2, &high, 1.0, UNWEIGHTED);
		let sharp = upsample(2, &low, 2, &high, 0.0, UNWEIGHTED);

		assert!(spread[3][0] > 0.0, "{spread:?}");
		assert!(spread[3][0] < spread[0][0], "{spread:?}");
		assert_rgba_close(sharp[3], [0.0, 0.0, 0.0, 1.0], 1e-6);
	}

	/// Runs the composite over 1x1 inputs with the given intensity.
	fn composite(scene_color: [f32; 4], bloom_color: [f32; 4], intensity: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_COMPOSITE_BESL));
		let parameter_slot = ResourceSlot::new(3);
		let mut scene = texture_2d(1, 1, &[scene_color]);
		let mut bloom = texture_2d(1, 1, &[bloom_color]);
		let mut result = empty_image(1, 1);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, intensity, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut scene);
		descriptors.bind_texture(ResourceSlot::new(1), &mut bloom);
		descriptors.bind_image(output_slot(0), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Verifies additive bloom and the zero-intensity passthrough branch.
	#[test]
	fn bloom_composite_besl_vm_preserves_zero_intensity_and_adds_positive_bloom() {
		let scene_color = [0.2, 0.3, 0.4, 0.6];
		let bloom_color = [0.5, 0.25, 0.125, 0.0];

		for (intensity, expected) in [(0.0, scene_color), (2.0, [1.2, 0.8, 0.65, 1.0])] {
			assert_rgba_close(composite(scene_color, bloom_color, intensity), expected, 1e-6);
		}
	}

	/// Verifies that a scene texel at the half-float limit stays finite when the glow is added.
	#[test]
	fn bloom_composite_besl_vm_holds_the_sum_to_the_half_float_range() {
		let result = composite([65504.0, 1.0, 1.0, 1.0], [100.0, 100.0, 100.0, 0.0], 1.0);

		assert_rgba_close(result, [65504.0, 101.0, 101.0, 1.0], 1e-2);
	}

	/// Runs the fused top-level upsample and composite over a `size` by `size` scene, one tile at a time, with the
	/// bloom pass's shader data. Each of a tile's 16x16 lanes writes a 2x2 block of pixels, so a tile covers 32x32 of
	/// them. The top level is half the scene's size and the level below it a quarter, both rounded down.
	fn upsample_composite(
		size: u32,
		scene_texels: &[[f32; 4]],
		low_texels: &[[f32; 4]],
		high_texels: &[[f32; 4]],
		data: BloomShaderData,
	) -> Texture {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_UPSAMPLE_COMPOSITE_BESL));
		let parameter_slot = ResourceSlot::new(4);
		let mut parameters = buffer(&program, parameter_slot);
		for (field, value) in [
			("prefilter", data.prefilter),
			("filter", data.filter),
			("same_weight", data.same_weight),
			("low_weight", data.low_weight),
			("scatter", data.scatter),
		] {
			parameters
				.write_indexed(field, 0, Value::Vec4F(value))
				.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		}
		let mut scene = texture_2d(size, size, scene_texels);
		let mut low = texture_2d(size / 4, size / 4, low_texels);
		let mut high = texture_2d(size / 2, size / 2, high_texels);
		let mut result = empty_image(size, size);
		for position in tiles(UPSAMPLE_COMPOSITE_TILE.0, [size, size]) {
			let mut workgroup = WorkgroupState::new();
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut low);
			descriptors.bind_texture(ResourceSlot::new(1), &mut high);
			descriptors.bind_texture(ResourceSlot::new(2), &mut scene);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_buffer(parameter_slot, &mut parameters);
			descriptors.bind_workgroup_state(&mut workgroup);
			run_tile(&program, &mut descriptors, position, UPSAMPLE_COMPOSITE_TILE, [size, size]);
		}
		result
	}

	/// Returns bloom settings with the given scatter, tail exponent, and halation, without a brightness ceiling.
	fn settings(scatter: f32, tail_exponent: f32, halation: [f32; 3], halation_radius: [f32; 3]) -> BloomPassSettings {
		BloomPassSettings {
			scatter,
			tail_exponent,
			halation,
			halation_radius,
			max_brightness: NO_CEILING,
			..BloomPassSettings::default()
		}
	}

	/// Returns the shader data of every climb of a `levels`-level pyramid on a sink `height` pixels tall, indexed by
	/// the level it builds, as `BloomPass` writes it.
	fn climbs(settings: BloomPassSettings, levels: usize, height: u32) -> Vec<BloomShaderData> {
		let weights = level_weights(settings, levels, height);
		(0..levels.saturating_sub(1).max(1))
			.map(|level| BloomShaderData::climb(settings, &weights, level, levels))
			.collect()
	}

	/// Returns the composite's shader data for a `levels`-level pyramid with `scatter` and no halation.
	fn composite_data(scatter: f32, levels: usize) -> BloomShaderData {
		climbs(settings(scatter, 2.5, [0.0; 3], [0.005; 3]), levels, 64)[0]
	}

	/// Returns `count` distinct, smoothly varying texels, so a misplaced tap or weight changes the result.
	fn varied_texels(count: u32, seed: f32) -> Vec<[f32; 4]> {
		(0..count)
			.map(|index| {
				let value = index as f32 * 0.37 + seed;
				[
					value.sin().abs() * 3.0,
					value.cos().abs() * 2.0,
					(value * 0.5).sin().abs(),
					1.0,
				]
			})
			.collect()
	}

	/// Verifies that the fused pass blends exactly what the upsample pass followed by a bilinear blend would: the top
	/// level built from both levels, filtered bilinearly to the scene's resolution and normalized, across tile
	/// boundaries. The odd size leaves a partial tile and 2x2 blocks that straddle the image's edge, and halves to a top
	/// level that is not exactly half the scene.
	#[test]
	fn bloom_upsample_composite_besl_vm_matches_separate_upsample_and_blend() {
		for size in [64, 39] {
			assert_upsample_composite_matches_separate_passes(size);
		}
	}

	/// Compares the fused pass with the upsample pass followed by a bilinear blend over a `size` by `size` scene, with
	/// weights that differ per channel.
	fn assert_upsample_composite_matches_separate_passes(size: u32) {
		let scene = varied_texels(size * size, 0.0);
		let low = varied_texels((size / 4) * (size / 4), 1.0);
		let high = varied_texels((size / 2) * (size / 2), 2.0);
		let data = climbs(settings(0.25, 2.5, [0.1, 0.05, 0.0], [0.02; 3]), 3, size)[0];
		let rgb = |value: [f32; 4]| [value[0], value[1], value[2]];

		let fused = upsample_composite(size, &scene, &low, &high, data);

		// The separate passes: climb into a top-level image, then sample it from the scene's texels and blend.
		let top_size = size / 2;
		let top_level = upsample(
			size / 4,
			&low,
			top_size,
			&high,
			1.0,
			(rgb(data.same_weight), rgb(data.low_weight)),
		);
		let scale = top_size as f32 / size as f32;
		for texel in 0..size * size {
			let coordinate = [texel % size, texel / size];
			let (x, y) = (coordinate[0] as f32, coordinate[1] as f32);
			let glow = bilinear(&top_level, top_size, (x + 0.5) * scale - 0.5, (y + 0.5) * scale - 0.5);
			let original = scene[texel as usize];
			let expected: [f32; 4] = std::array::from_fn(|channel| {
				if channel == 3 {
					1.0
				} else {
					original[channel] * (1.0 - data.scatter[channel]) + glow[channel]
				}
			});
			assert_rgba_close(rgba(&fused, coordinate), expected, 1e-4);
		}
	}

	/// Verifies that a one-level pyramid blends its only level and nothing from the level bound below it, and that
	/// zero scatter passes the scene through.
	#[test]
	fn bloom_upsample_composite_besl_vm_weights_the_level_below_and_passes_through_at_zero() {
		let scene = [[0.2, 0.3, 0.4, 0.6]; 16 * 16];
		let low = [[9.0, 9.0, 9.0, 1.0]; 4 * 4];
		let high = [[0.5, 0.25, 0.125, 1.0]; 8 * 8];

		let one_level = upsample_composite(16, &scene, &low, &high, composite_data(0.5, 1));
		let passthrough = upsample_composite(16, &scene, &low, &high, composite_data(0.0, 3));

		assert_rgba_close(rgba(&one_level, [7, 9]), [0.35, 0.275, 0.2625, 1.0], 1e-6);
		assert_rgba_close(rgba(&passthrough, [7, 9]), [0.2, 0.3, 0.4, 0.6], 1e-6);
	}

	/// Verifies that the fused pass stores a scene texel that overflowed to infinity as the largest finite half float.
	#[test]
	fn bloom_upsample_composite_besl_vm_holds_the_result_to_the_half_float_range() {
		let scene = [[f32::INFINITY, 1.0, 1.0, 1.0]; 16 * 16];
		let low = [[0.0, 0.0, 0.0, 1.0]; 4 * 4];
		let high = [[100.0, 100.0, 100.0, 1.0]; 8 * 8];

		let result = upsample_composite(16, &scene, &low, &high, composite_data(0.5, 1));

		assert_rgba_close(rgba(&result, [3, 12]), [65504.0, 50.5, 50.5, 1.0], 1e-2);
	}

	/// Runs the pass's whole chain over a `size` by `size` scene, as `BloomPass` records it: the prefilter, the
	/// downsamples, the climbs, and the fused composite over `levels` levels.
	fn bloom(size: u32, scene: &[[f32; 4]], settings: BloomPassSettings, levels: usize) -> Texture {
		let data = climbs(settings, levels, size);
		let texels = |image: &Texture, size: u32| -> Vec<[f32; 4]> {
			(0..size * size)
				.map(|index| rgba(image, [index % size, index / size]))
				.collect()
		};
		let rgb = |value: [f32; 4]| [value[0], value[1], value[2]];
		let level_size = |level: usize| size / level_divisor(level);

		let mut downsampled = vec![texels(&extract_image(size, scene), level_size(0))];
		for level in 1..levels {
			downsampled.push(downsample_image(level_size(level - 1), &downsampled[level - 1]));
		}
		// Climb from the smallest level up to the one below the top, which the fused pass climbs itself.
		let mut finished = downsampled[levels - 1].clone();
		for level in (1..levels.saturating_sub(1)).rev() {
			let weights = (rgb(data[level].same_weight), rgb(data[level].low_weight));
			finished = upsample(
				level_size(level + 1),
				&finished,
				level_size(level),
				&downsampled[level],
				1.0,
				weights,
			);
		}
		let below_top = if levels == 1 { downsampled[0].clone() } else { finished };
		upsample_composite(size, scene, &below_top, &downsampled[0], data[0])
	}

	/// Runs the pyramid downsample over a whole `size` by `size` level into one half its size.
	fn downsample_image(size: u32, texels: &[[f32; 4]]) -> Vec<[f32; 4]> {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(BLOOM_DOWNSAMPLE_BESL));
		let mut source = texture_2d(size, size, texels);
		let half = size / 2;
		let mut result = empty_image(half, half);
		for index in 0..half * half {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut source);
			descriptors.bind_image(ResourceSlot::new(1), &mut result);
			run_at(&program, &mut descriptors, [index % half, index / half]);
		}
		(0..half * half)
			.map(|index| rgba(&result, [index % half, index / half]))
			.collect()
	}

	/// Returns a `size` by `size` scene of `background` with an 8x8 block of `highlight` at its center.
	fn highlight_scene(size: u32, background: f32, highlight: f32) -> Vec<[f32; 4]> {
		let block = size / 2 - 4..size / 2 + 4;
		(0..size * size)
			.map(|index| {
				let value = if block.contains(&(index % size)) && block.contains(&(index / size)) {
					highlight
				} else {
					background
				};
				[value, value, value, 1.0]
			})
			.collect()
	}

	/// Verifies that bloom adds no light: a field of one brightness passes through unchanged at any scatter, tail, and
	/// halation.
	#[test]
	fn bloom_besl_vm_passes_a_uniform_field_through() {
		const SIZE: u32 = 64;
		let scene = vec![[0.5, 2.0, 30.0, 1.0]; (SIZE * SIZE) as usize];

		for settings in [
			settings(0.04, 2.5, [0.0; 3], [0.005; 3]),
			settings(1.0, 2.0, [0.0; 3], [0.005; 3]),
			settings(0.5, 3.0, [0.3, 0.1, 0.0], [0.1, 0.05, 0.2]),
		] {
			let result = bloom(SIZE, &scene, settings, 4);
			for coordinate in [[0, 0], [17, 40], [63, 63]] {
				assert_rgba_close(rgba(&result, coordinate), [0.5, 2.0, 30.0, 1.0], 1e-3);
			}
		}
	}

	/// Verifies that a highlight gives its scatter to darker neighbors as a tail that fades with distance, and grows no
	/// brighter.
	#[test]
	fn bloom_besl_vm_spreads_a_highlight_into_a_falling_tail() {
		const SIZE: u32 = 64;
		let scene = highlight_scene(SIZE, 0.01, 8.0);

		let result = bloom(SIZE, &scene, settings(0.1, 2.5, [0.0; 3], [0.005; 3]), 4);

		assert!(
			rgba(&result, [32, 32])[0] < 8.0,
			"the highlight grew: {:?}",
			rgba(&result, [32, 32])
		);
		// Texels right of the block, two to twenty texels past its edge.
		let tail = [38, 42, 48, 56].map(|x| rgba(&result, [x, 32])[0]);
		assert!(tail[0] > 0.01, "no tail: {tail:?}");
		assert!(
			tail.windows(2).all(|pair| pair[0] > pair[1] && pair[1] > 0.01 * 0.9),
			"{tail:?}"
		);
	}

	/// Verifies that a larger tail exponent keeps the glow closer to its source: brighter beside it, dimmer far away.
	#[test]
	fn bloom_besl_vm_tail_exponent_sets_the_reach() {
		const SIZE: u32 = 64;
		let scene = highlight_scene(SIZE, 0.0, 8.0);

		let wide = bloom(SIZE, &scene, settings(0.1, 2.0, [0.0; 3], [0.005; 3]), 4);
		let tight = bloom(SIZE, &scene, settings(0.1, 3.0, [0.0; 3], [0.005; 3]), 4);

		let (near, far) = ([36, 32], [60, 32]);
		assert!(
			rgba(&tight, near)[0] > rgba(&wide, near)[0],
			"{:?} {:?}",
			rgba(&tight, near),
			rgba(&wide, near)
		);
		assert!(
			rgba(&tight, far)[0] < rgba(&wide, far)[0],
			"{:?} {:?}",
			rgba(&tight, far),
			rgba(&wide, far)
		);
	}

	/// Verifies that halation in one channel fringes a white highlight in that channel alone and leaves the others as
	/// they were.
	#[test]
	fn bloom_besl_vm_halation_fringes_only_its_channel() {
		const SIZE: u32 = 64;
		let scene = highlight_scene(SIZE, 0.01, 8.0);

		// An 8-pixel radius on a 64-pixel sink.
		let result = bloom(SIZE, &scene, settings(0.0, 2.0, [0.2, 0.0, 0.0], [0.125; 3]), 4);

		let fringe = rgba(&result, [40, 32]);
		assert!(fringe[0] > 0.01 * 1.5, "no red fringe: {fringe:?}");
		assert_rgba_close([fringe[1], fringe[2], 0.0, 1.0], [0.01, 0.01, 0.0, 1.0], 1e-6);
	}

	/// Verifies that each channel's halo reaches its own radius: the channel with the smaller radius glows brighter
	/// beside the highlight, and the one with the larger radius further out.
	#[test]
	fn bloom_besl_vm_halation_radius_is_per_channel() {
		const SIZE: u32 = 64;
		let scene = highlight_scene(SIZE, 0.0, 8.0);

		// Red reaches 16 pixels and green 2 on a 64-pixel sink.
		let result = bloom(SIZE, &scene, settings(0.0, 2.0, [0.2, 0.2, 0.0], [0.25, 0.03125, 0.0]), 4);

		let (near, far) = (rgba(&result, [37, 32]), rgba(&result, [52, 32]));
		assert!(near[1] > near[0], "{near:?}");
		assert!(far[0] > far[1], "{far:?}");
	}

	/// Lowers every bloom program through the platform shader compiler. The VM tests above prove the programs
	/// link and behave as BESL; defects the backend mishandles, such as helper functions over vector arguments,
	/// only surface when the real platform compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn bloom_besl_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

		// The downsample and upsample run as compute; the others are tile shaders.
		for (name, source, tile) in [
			("bloom_extract", *BLOOM_EXTRACT_BESL, Some(EXTRACT_TILE)),
			("bloom_downsample", BLOOM_DOWNSAMPLE_BESL, None),
			("bloom_upsample", *BLOOM_UPSAMPLE_BESL, None),
			("bloom_composite", *BLOOM_COMPOSITE_BESL, Some((PIXEL_TILE, PIXEL_TILE))),
			(
				"bloom_upsample_composite",
				*BLOOM_UPSAMPLE_COMPOSITE_BESL,
				Some(UPSAMPLE_COMPOSITE_TILE),
			),
		] {
			let mut root = crate::rendering::shader_vm_test::with_engine_modules(source);
			root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
			let root = besl::lex(root).expect("bloom shader should link");
			let settings = match tile {
				Some((tile, width)) => ShaderGenerationSettings::tile(tile, Extent::square(width)),
				None => ShaderGenerationSettings::compute(Extent::rectangle(8, 8)),
			}
			.name(name.to_string());

			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}
	}
}
