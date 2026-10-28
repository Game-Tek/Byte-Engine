use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute},
	},
};

const MAX_BLOOM_LEVELS: u32 = 6;

/// The `BloomPassSettings` struct defines the intent and shaping controls for a reusable HDR bloom stage.
///
/// Pass it to [`crate::application::graphics::setup_bloom_render_pass`] or [`BloomPass::with_settings`].
#[derive(Clone, Copy, Debug)]
pub struct BloomPassSettings {
	/// Scene-linear brightness, after exposure, above which light starts to glow. `1.0` glows only what the
	/// display cannot show.
	pub threshold: f32,
	/// Fraction of the threshold over which the glow fades in, from `0.0` for a hard cut to `1.0`.
	pub soft_knee: f32,
	/// Scale of the glow added onto the scene. `0.0` passes the scene through.
	pub intensity: f32,
	/// Scene-linear brightness, after exposure, that a texel is held to before it glows. Without it, a mirror
	/// reflection of the sun, at the half-float limit, blooms across the whole frame. `64.0` is six stops above
	/// display white.
	pub max_brightness: f32,
	/// Width of each upsample blur in texels of the level being blurred. Larger values spread the glow further.
	pub radius: f32,
	/// Pyramid depth, from `1` to `6`. Each level halves the resolution and doubles the glow's reach.
	pub levels: u32,
}

impl Default for BloomPassSettings {
	fn default() -> Self {
		Self {
			threshold: 1.0,
			soft_knee: 0.5,
			intensity: 0.08,
			max_brightness: 64.0,
			radius: 1.0,
			levels: 5,
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
	/// Threshold, soft knee, composite intensity, and brightness ceiling.
	pub(crate) prefilter: [f32; 4],
	/// Upsample radius in `x`, and in `y` the weight of the level below the top, zero when the pyramid has one level;
	/// the other lanes are unused.
	pub(crate) filter: [f32; 4],
}

/// The `BloomPass` struct creates a reusable pre-tonemap glow stage that can feed later post-processing.
///
/// The pass prefilters the HDR `main` target into a half-resolution pyramid with Jimenez's 13-tap filter, using a
/// luma-weighted average on the first level so single bright texels do not flicker, then climbs back up with a
/// tent blur per level. The last climb and the addition onto the scene share one dispatch, so the top level never
/// reaches memory. Install it after the passes that write scene light and before tone mapping. Toggle it at runtime
/// with the `render.pass.bloom` parameter.
pub struct BloomPass {
	settings: BloomPassSettings,
	parameters: ghi::DynamicBufferHandle<BloomShaderData>,
	extract_pass: simple_compute::Pass,
	downsample_passes: Vec<simple_compute::Pass>,
	upsample_passes: Vec<simple_compute::Pass>,
	/// Builds the top level from the one below it and adds it onto the scene.
	composite_pass: simple_compute::Pass,
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
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("Bloom Output"),
		);

		let level_count = settings.resolved_level_count();
		// Each pyramid level halves the previous one, starting at half the sink resolution.
		let mut pyramid_target = |name, level: usize| -> ghi::BaseImageHandle {
			render_pass_builder
				.create_scaled_render_target(
					ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Storage | ghi::Uses::Image)
						.name(name),
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
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Bloom Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = context.build_sampler(ghi::sampler::Builder::new());

		let extract_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Extract", "byte-engine/rendering/bloom/extract.pipeline"),
		);
		let downsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Downsample", "byte-engine/rendering/bloom/downsample.pipeline"),
		);
		let upsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Upsample", "byte-engine/rendering/bloom/upsample.pipeline"),
		);
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new(
				"Bloom Upsample Composite",
				"byte-engine/rendering/bloom/upsample-composite.pipeline",
			),
		);

		let extract_pass = extract_pipeline.bind(
			"Bloom Extract Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
				simple_compute::Resource::image("result_texture", downsample_images[0]),
				simple_compute::Resource::buffer("bloom_parameters", parameters),
			],
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
						simple_compute::Resource::buffer("bloom_parameters", parameters),
					],
				)
			})
			.collect::<Vec<_>>();

		// A one-level pyramid has nothing below its top; the composite weights the level it binds there by zero.
		let below_top = if level_count == 1 { downsample_images[0] } else { finished(1) };
		let composite_pass = composite_pipeline.bind(
			"Bloom Upsample Composite Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
				simple_compute::Resource::combined_image_sampler(
					"low_resolution_texture",
					below_top,
					sampler,
					ghi::Layouts::Read,
				),
				simple_compute::Resource::combined_image_sampler(
					"high_resolution_texture",
					downsample_images[0],
					sampler,
					ghi::Layouts::Read,
				),
				simple_compute::Resource::image("result_texture", output),
				simple_compute::Resource::buffer("bloom_parameters", parameters),
			],
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

	/// Writes the static bloom controls into the per-frame parameter buffer before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame) {
		let parameters = frame.get_mut_dynamic_buffer_slice(self.parameters);

		parameters.prefilter = [
			self.settings.threshold.max(0.0),
			self.settings.soft_knee.clamp(0.0, 1.0),
			self.settings.intensity.max(0.0),
			self.settings.max_brightness.max(0.0),
		];
		let below_top_weight = if self.level_count > 1 { 1.0 } else { 0.0 };
		parameters.filter = [self.settings.radius.max(0.0), below_top_weight, 0.0, 0.0];
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

		self.write_parameters(frame);

		let level_count = self.level_count;

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("Bloom"),
					|command_buffer| {
						extract_pass.record(command_buffer, block_lanes(bloom_extent(extent, 0)));

						for (index, pass) in downsample_passes.iter().enumerate() {
							pass.record(command_buffer, bloom_extent(extent, index + 1));
						}

						for (level, pass) in (1..level_count.saturating_sub(1)).rev().zip(upsample_passes.iter()) {
							pass.record(command_buffer, bloom_extent(extent, level));
						}

						composite_pass.record(command_buffer, block_lanes(extent));
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

/// Texels each lane of the extract and the upsample composite writes along each axis, matching `BLOOM_LANE_WIDTH` in
/// `bloom/extract.besl` and `bloom/upsample-composite.besl`.
const BLOCK_LANE_WIDTH: u32 = 2;

/// Returns the lanes that dispatch a shader whose lanes each write a 2x2 block of a destination `extent` texels across:
/// the extract and the upsample composite. Every pass that records one of them, such as
/// [`crate::rendering::render_passes::lens_flare::LensFlarePass`] with the extract, sizes its dispatch with this.
pub(crate) fn block_lanes(extent: Extent) -> Extent {
	Extent::rectangle(
		extent.width().div_ceil(BLOCK_LANE_WIDTH),
		extent.height().div_ceil(BLOCK_LANE_WIDTH),
	)
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ExecutableProgram, ResourceSlot, Texture, Value, WorkgroupState};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, lane_config, rgba, run_at, texture_2d};

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

	/// Runs the `width` by `height` workgroup whose first lane is at `origin`.
	fn run_workgroup(
		program: &ExecutableProgram,
		descriptors: &mut DescriptorBindings<'_>,
		origin: [u32; 2],
		width: u32,
		height: u32,
	) {
		let configs = (0..width * height)
			.map(|lane| lane_config(lane).with_thread_id([origin[0] + lane % width, origin[1] + lane / width]))
			.collect::<Vec<_>>();
		program.run_workgroup(descriptors, &configs).expect(
			"Failed to execute a bloom workgroup with the BESL VM. The most likely cause is missing runtime support or invalid workgroup memory synchronization.",
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
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_workgroup_state(&mut workgroup);
		run_workgroup(&program, &mut descriptors, [0, 0], 8, 8);
		drop(descriptors);
		rgba(&result, [0, 0])
	}

	/// Runs the prefilter over a whole `size` by `size` source, every workgroup of it, without threshold or ceiling.
	/// Each lane filters a 2x2 block of destination texels, so an 8x8 workgroup covers a 16x16 tile.
	fn extract_image(size: u32, source_texels: &[[f32; 4]]) -> Texture {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_EXTRACT_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, 0.0, NO_CEILING]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut source = texture_2d(size, size, source_texels);
		let mut result = empty_image(size / 2, size / 2);
		let lanes = block_lanes(Extent::square(size / 2));
		for y in (0..lanes.height()).step_by(8) {
			for x in (0..lanes.width()).step_by(8) {
				let mut workgroup = WorkgroupState::new();
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut source);
				descriptors.bind_image(ResourceSlot::new(1), &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				descriptors.bind_workgroup_state(&mut workgroup);
				run_workgroup(&program, &mut descriptors, [x, y], 8, 8);
			}
		}
		result
	}

	/// Verifies that the prefilter is the same centered filter at every destination texel: the glow around a bright
	/// block is mirror-symmetric, and moving the block by whole destination texels moves the glow unchanged, whether
	/// the filter's footprint stays inside one lane's 2x2 texels, spans several lanes, or straddles the next workgroup's
	/// tile.
	#[test]
	fn bloom_extract_besl_vm_filters_alike_across_workgroups() {
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

		// Each lane filters two destination texels along each axis, and texel 16 starts the second workgroup's tile, so
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

	/// Verifies that the prefilter holds back a single bright texel below the block's plain average.
	#[test]
	fn bloom_extract_besl_vm_suppresses_fireflies() {
		let firefly = [
			[4.0, 4.0, 4.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
			[0.0, 0.0, 0.0, 1.0],
		];
		let uniform = [[1.0, 1.0, 1.0, 1.0]; 4];

		let firefly_result = extract(&firefly, 0.0, 0.0, NO_CEILING);
		let uniform_result = extract(&uniform, 0.0, 0.0, NO_CEILING);

		assert_rgba_close(uniform_result, [1.0, 1.0, 1.0, 1.0], 1e-5);
		// Both blocks average to one, so the luma weighting is what pulls the firefly down.
		assert!(firefly_result[0] > 0.0 && firefly_result[0] < 0.75, "{firefly_result:?}");
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

	/// Runs one upsample step from a `low_size` level onto a `high_size` level with the given radius, returning every
	/// result texel in row-major order.
	fn upsample(
		low_size: u32,
		low_texels: &[[f32; 4]],
		high_size: u32,
		high_texels: &[[f32; 4]],
		radius: f32,
	) -> Vec<[f32; 4]> {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_UPSAMPLE_BESL));
		let parameter_slot = ResourceSlot::new(3);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("filter", 0, Value::Vec4F([radius, 0.0, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
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
			let climbed = upsample(LOW_SIZE, &low, HIGH_SIZE, &high, radius);
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

	/// Verifies that upsampling adds the blurred lower level onto the same-resolution level.
	#[test]
	fn bloom_upsample_besl_vm_combines_both_levels() {
		let result = upsample(1, &[[0.1, 0.2, 0.3, 0.0]], 1, &[[0.4, 0.5, 0.6, 0.0]], 1.0);

		assert_rgba_close(result[0], [0.5, 0.7, 0.9, 1.0], 1e-6);
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

		let spread = upsample(2, &low, 2, &high, 1.0);
		let sharp = upsample(2, &low, 2, &high, 0.0);

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
		descriptors.bind_image(ResourceSlot::new(2), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
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

	/// Runs the fused top-level upsample and composite over a `size` by `size` scene, one 16x16 workgroup at a time.
	/// Each lane writes a 2x2 block of pixels, so a workgroup covers 32x32 of them. The top level is half the scene's
	/// size and the level below it a quarter, both rounded down.
	fn upsample_composite(
		size: u32,
		scene_texels: &[[f32; 4]],
		low_texels: &[[f32; 4]],
		high_texels: &[[f32; 4]],
		below_top_weight: f32,
		intensity: f32,
	) -> Texture {
		let program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_UPSAMPLE_COMPOSITE_BESL));
		let parameter_slot = ResourceSlot::new(4);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, intensity, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		parameters
			.write_indexed("filter", 0, Value::Vec4F([1.0, below_top_weight, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut scene = texture_2d(size, size, scene_texels);
		let mut low = texture_2d(size / 4, size / 4, low_texels);
		let mut high = texture_2d(size / 2, size / 2, high_texels);
		let mut result = empty_image(size, size);
		let lanes = block_lanes(Extent::square(size));
		for y in (0..lanes.height()).step_by(16) {
			for x in (0..lanes.width()).step_by(16) {
				let mut workgroup = WorkgroupState::new();
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut low);
				descriptors.bind_texture(ResourceSlot::new(1), &mut high);
				descriptors.bind_texture(ResourceSlot::new(2), &mut scene);
				descriptors.bind_image(ResourceSlot::new(3), &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				descriptors.bind_workgroup_state(&mut workgroup);
				run_workgroup(&program, &mut descriptors, [x, y], 16, 16);
			}
		}
		result
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

	/// Verifies that the fused pass adds exactly what the upsample pass followed by the plain composite adds: the top
	/// level built from both levels, filtered bilinearly to the scene's resolution, across workgroup boundaries. The odd
	/// size leaves a partial workgroup and 2x2 blocks that straddle the image's edge, and halves to a top level that is
	/// not exactly half the scene.
	#[test]
	fn bloom_upsample_composite_besl_vm_matches_separate_upsample_and_composite() {
		for size in [64, 39] {
			assert_upsample_composite_matches_separate_passes(size);
		}
	}

	/// Compares the fused pass with the upsample pass followed by the plain composite over a `size` by `size` scene.
	fn assert_upsample_composite_matches_separate_passes(size: u32) {
		let scene = varied_texels(size * size, 0.0);
		let low = varied_texels((size / 4) * (size / 4), 1.0);
		let high = varied_texels((size / 2) * (size / 2), 2.0);
		let intensity = 0.5;

		let fused = upsample_composite(size, &scene, &low, &high, 1.0, intensity);

		// The separate passes: climb into a top-level image, then sample it from the scene's texels.
		let upsample_program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_UPSAMPLE_BESL));
		let upsample_parameter_slot = ResourceSlot::new(3);
		let mut upsample_parameters = buffer(&upsample_program, upsample_parameter_slot);
		upsample_parameters
			.write_indexed("filter", 0, Value::Vec4F([1.0, 0.0, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut low_texture = texture_2d(size / 4, size / 4, &low);
		let mut high_texture = texture_2d(size / 2, size / 2, &high);
		let mut top_level = empty_image(size / 2, size / 2);
		for texel in 0..(size / 2) * (size / 2) {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut low_texture);
			descriptors.bind_texture(ResourceSlot::new(1), &mut high_texture);
			descriptors.bind_image(ResourceSlot::new(2), &mut top_level);
			descriptors.bind_buffer(upsample_parameter_slot, &mut upsample_parameters);
			run_at(&upsample_program, &mut descriptors, [texel % (size / 2), texel / (size / 2)]);
		}
		let composite_program =
			crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(*BLOOM_COMPOSITE_BESL));
		let composite_parameter_slot = ResourceSlot::new(3);
		let mut composite_parameters = buffer(&composite_program, composite_parameter_slot);
		composite_parameters
			.write_indexed("prefilter", 0, Value::Vec4F([0.0, 0.0, intensity, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut scene_texture = texture_2d(size, size, &scene);
		let mut separate = empty_image(size, size);
		for texel in 0..size * size {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut scene_texture);
			descriptors.bind_texture(ResourceSlot::new(1), &mut top_level);
			descriptors.bind_image(ResourceSlot::new(2), &mut separate);
			descriptors.bind_buffer(composite_parameter_slot, &mut composite_parameters);
			run_at(&composite_program, &mut descriptors, [texel % size, texel / size]);
		}

		for texel in 0..size * size {
			let coordinate = [texel % size, texel / size];
			assert_rgba_close(rgba(&fused, coordinate), rgba(&separate, coordinate), 1e-4);
		}
	}

	/// Verifies that a one-level pyramid adds its only level and nothing from the level bound below it, and that zero
	/// intensity passes the scene through.
	#[test]
	fn bloom_upsample_composite_besl_vm_weights_the_level_below_and_passes_through_at_zero() {
		let scene = [[0.2, 0.3, 0.4, 0.6]; 16 * 16];
		let low = [[9.0, 9.0, 9.0, 1.0]; 4 * 4];
		let high = [[0.5, 0.25, 0.125, 1.0]; 8 * 8];

		let one_level = upsample_composite(16, &scene, &low, &high, 0.0, 2.0);
		let passthrough = upsample_composite(16, &scene, &low, &high, 1.0, 0.0);

		assert_rgba_close(rgba(&one_level, [7, 9]), [1.2, 0.8, 0.65, 1.0], 1e-6);
		assert_rgba_close(rgba(&passthrough, [7, 9]), [0.2, 0.3, 0.4, 0.6], 1e-6);
	}

	/// Verifies that the fused pass holds a scene texel at the half-float limit finite when the glow is added.
	#[test]
	fn bloom_upsample_composite_besl_vm_holds_the_sum_to_the_half_float_range() {
		let scene = [[65504.0, 1.0, 1.0, 1.0]; 16 * 16];
		let low = [[0.0, 0.0, 0.0, 1.0]; 4 * 4];
		let high = [[100.0, 100.0, 100.0, 1.0]; 8 * 8];

		let result = upsample_composite(16, &scene, &low, &high, 1.0, 1.0);

		assert_rgba_close(rgba(&result, [3, 12]), [65504.0, 101.0, 101.0, 1.0], 1e-2);
	}

	/// Lowers every bloom program through the platform shader compiler. The VM tests above prove the programs
	/// link and behave as BESL; defects the backend mishandles, such as helper functions over vector arguments,
	/// only surface when the real platform compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn bloom_besl_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

		for (name, source) in [
			("bloom_extract", *BLOOM_EXTRACT_BESL),
			("bloom_downsample", BLOOM_DOWNSAMPLE_BESL),
			("bloom_upsample", *BLOOM_UPSAMPLE_BESL),
			("bloom_composite", *BLOOM_COMPOSITE_BESL),
			("bloom_upsample_composite", *BLOOM_UPSAMPLE_COMPOSITE_BESL),
		] {
			let mut root = besl::parse(source).expect("bloom shader should parse");
			root.add(vec![crate::rendering::common_shader_generator::common_shader_scope()]);
			let root = besl::lex(root).expect("bloom shader should link");
			let settings = ShaderGenerationSettings::compute(Extent::rectangle(8, 8)).name(name.to_string());

			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}
	}
}
