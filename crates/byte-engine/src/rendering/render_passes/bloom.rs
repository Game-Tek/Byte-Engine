use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use utils::{Box, Extent};

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
	/// Upsample radius in `x`; the other lanes are unused.
	pub(crate) filter: [f32; 4],
}

/// The `BloomPass` struct creates a reusable pre-tonemap glow stage that can feed later post-processing.
///
/// The pass prefilters the HDR `main` target into a half-resolution pyramid with Jimenez's 13-tap filter, using a
/// luma-weighted average on the first level so single bright texels do not flicker, then climbs back up with a
/// tent blur per level and adds the result onto the scene. Install it after the passes that write scene light and
/// before tone mapping. Toggle it at runtime with the `render.pass.bloom` parameter.
pub struct BloomPass {
	settings: BloomPassSettings,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	parameters: ghi::DynamicBufferHandle<BloomShaderData>,
	extract_pass: simple_compute::Pass,
	downsample_passes: Vec<simple_compute::Pass>,
	upsample_passes: Vec<simple_compute::Pass>,
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
					ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name(name),
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
		let upsample_images = [
			"Bloom Upsample 0",
			"Bloom Upsample 1",
			"Bloom Upsample 2",
			"Bloom Upsample 3",
			"Bloom Upsample 4",
		][..level_count - 1]
			.iter()
			.enumerate()
			.map(|(level, name)| pyramid_target(name, level))
			.collect::<Vec<_>>();

		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Bloom Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		let extract_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Extract", "byte-engine/rendering/bloom/extract.pipeline"),
		)
		.expect(
			"Failed to create bloom extract shader. The most likely cause is an incompatible bloom extract shader interface.",
		);
		let downsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Downsample", "byte-engine/rendering/bloom/downsample.pipeline"),
		)
		.expect(
			"Failed to create bloom downsample shader. The most likely cause is an incompatible bloom downsample shader interface.",
		);
		let upsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Upsample", "byte-engine/rendering/bloom/upsample.pipeline"),
		)
		.expect(
			"Failed to create bloom upsample shader. The most likely cause is an incompatible bloom upsample shader interface.",
		);
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Bloom Composite", "byte-engine/rendering/bloom/composite.pipeline"),
		)
		.expect(
			"Failed to create bloom composite shader. The most likely cause is an incompatible bloom composite shader interface.",
		);

		let extract_pass = extract_pipeline
			.bind(
				render_pass_builder,
				"Bloom Extract Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", downsample_images[0]),
					simple_compute::Resource::buffer("bloom_parameters", parameters),
				],
			)
			.expect("Failed to bind bloom extract resources. The most likely cause is a changed BESL binding contract.");

		let downsample_passes = (1..level_count)
			.map(|index| {
				downsample_pipeline
					.bind(
						render_pass_builder,
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
					.expect(
						"Failed to bind bloom downsample resources. The most likely cause is a changed BESL binding contract.",
					)
			})
			.collect::<Vec<_>>();

		let upsample_passes = (0..level_count.saturating_sub(1))
			.rev()
			.map(|level| {
				let low_resolution_source = if level == level_count - 2 {
					downsample_images[level + 1]
				} else {
					upsample_images[level + 1]
				};
				upsample_pipeline
					.bind(
						render_pass_builder,
						"Bloom Upsample Descriptor Set",
						&[
							simple_compute::Resource::combined_image_sampler(
								"low_resolution_texture",
								low_resolution_source,
								sampler,
								ghi::Layouts::Read,
							),
							simple_compute::Resource::combined_image_sampler(
								"high_resolution_texture",
								downsample_images[level],
								sampler,
								ghi::Layouts::Read,
							),
							simple_compute::Resource::image("result_texture", upsample_images[level]),
							simple_compute::Resource::buffer("bloom_parameters", parameters),
						],
					)
					.expect(
						"Failed to bind bloom upsample resources. The most likely cause is a changed BESL binding contract.",
					)
			})
			.collect::<Vec<_>>();

		let bloom_source = if level_count == 1 {
			downsample_images[0]
		} else {
			upsample_images[0]
		};
		let composite_pass = composite_pipeline
			.bind(
				render_pass_builder,
				"Bloom Composite Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler(
						"bloom_texture",
						bloom_source,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("result_texture", output),
					simple_compute::Resource::buffer("bloom_parameters", parameters),
				],
			)
			.expect("Failed to bind bloom composite resources. The most likely cause is a changed BESL binding contract.");
		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			settings,
			bypass_pass,
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
		parameters.filter = [self.settings.radius.max(0.0), 0.0, 0.0, 0.0];
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
		let downsample_passes = self
			.downsample_passes
			.iter_mut()
			.map(|pass| pass.ready(frame))
			.collect::<Option<Vec<_>>>()?;
		let upsample_passes = self
			.upsample_passes
			.iter_mut()
			.map(|pass| pass.ready(frame))
			.collect::<Option<Vec<_>>>()?;
		let composite_pass = self.composite_pass.ready(frame)?;
		let extent = sink.extent();

		self.write_parameters(frame);

		let downsample_passes = frame_allocator.alloc_slice_copy(&downsample_passes);
		let upsample_passes = frame_allocator.alloc_slice_copy(&upsample_passes);
		let level_count = self.level_count;

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Bloom"),
					|command_buffer| {
						extract_pass.record(command_buffer, bloom_extent(extent, 0));

						for (index, pass) in downsample_passes.iter().enumerate() {
							pass.record(command_buffer, bloom_extent(extent, index + 1));
						}

						if level_count > 1 {
							for (level, pass) in (0..level_count - 1).rev().zip(upsample_passes.iter()) {
								pass.record(command_buffer, bloom_extent(extent, level));
							}
						}

						composite_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}

	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
}

/// Returns the resolution divisor of a pyramid level: level 0 is half the sink resolution.
fn level_divisor(level: usize) -> u32 {
	2 << level
}

/// Returns the extent of a pyramid level, matching the size the renderer gives its render target.
fn bloom_extent(extent: Extent, level: usize) -> Extent {
	crate::rendering::renderer::scaled_extent(extent, level_divisor(level))
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Value};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const BLOOM_EXTRACT_BESL: &str = include_str!("../../../assets/rendering/bloom/extract.besl");
	const BLOOM_DOWNSAMPLE_BESL: &str = include_str!("../../../assets/rendering/bloom/downsample.besl");
	const BLOOM_UPSAMPLE_BESL: &str = include_str!("../../../assets/rendering/bloom/upsample.besl");
	const BLOOM_COMPOSITE_BESL: &str = include_str!("../../../assets/rendering/bloom/composite.besl");

	/// A prefilter ceiling above any test input, so it does not take part.
	const NO_CEILING: f32 = 65504.0;

	/// Runs the prefilter over a 2x2 source into one texel.
	fn extract(source_texels: &[[f32; 4]; 4], threshold: f32, soft_knee: f32, max_brightness: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(BLOOM_EXTRACT_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("prefilter", 0, Value::Vec4F([threshold, soft_knee, 0.0, max_brightness]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut source = texture_2d(2, 2, source_texels);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut source);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		rgba(&result, [0, 0])
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

	/// Runs one upsample step with the given radius, returning every result texel in row-major order.
	fn upsample(size: u32, low_texels: &[[f32; 4]], high_texels: &[[f32; 4]], radius: f32) -> Vec<[f32; 4]> {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(BLOOM_UPSAMPLE_BESL));
		let parameter_slot = ResourceSlot::new(3);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed("filter", 0, Value::Vec4F([radius, 0.0, 0.0, 0.0]))
			.expect("Failed to initialize bloom parameters. The most likely cause is a changed production buffer layout.");
		let mut low = texture_2d(size, size, low_texels);
		let mut high = texture_2d(size, size, high_texels);
		let mut result = empty_image(size, size);
		for y in 0..size {
			for x in 0..size {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut low);
				descriptors.bind_texture(ResourceSlot::new(1), &mut high);
				descriptors.bind_image(ResourceSlot::new(2), &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}
		(0..size * size)
			.map(|index| rgba(&result, [index % size, index / size]))
			.collect()
	}

	/// Verifies that upsampling adds the blurred lower level onto the same-resolution level.
	#[test]
	fn bloom_upsample_besl_vm_combines_both_levels() {
		let result = upsample(1, &[[0.1, 0.2, 0.3, 0.0]], &[[0.4, 0.5, 0.6, 0.0]], 1.0);

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

		let spread = upsample(2, &low, &high, 1.0);
		let sharp = upsample(2, &low, &high, 0.0);

		assert!(spread[3][0] > 0.0, "{spread:?}");
		assert!(spread[3][0] < spread[0][0], "{spread:?}");
		assert_rgba_close(sharp[3], [0.0, 0.0, 0.0, 1.0], 1e-6);
	}

	/// Runs the composite over 1x1 inputs with the given intensity.
	fn composite(scene_color: [f32; 4], bloom_color: [f32; 4], intensity: f32) -> [f32; 4] {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(BLOOM_COMPOSITE_BESL));
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

	/// Lowers every bloom program through the platform shader compiler. The VM tests above prove the programs
	/// link and behave as BESL; defects the backend mishandles, such as helper functions over vector arguments,
	/// only surface when the real platform compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn bloom_besl_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;

		for (name, source) in [
			("bloom_extract", BLOOM_EXTRACT_BESL),
			("bloom_downsample", BLOOM_DOWNSAMPLE_BESL),
			("bloom_upsample", BLOOM_UPSAMPLE_BESL),
			("bloom_composite", BLOOM_COMPOSITE_BESL),
		] {
			let mut root = besl::parse(source).expect("bloom shader should parse");
			root.add(vec![crate::rendering::common_shader_generator::CommonShaderScope::new()]);
			let root = besl::lex(root).expect("bloom shader should link");
			let settings = ShaderGenerationSettings::compute(Extent::rectangle(8, 8)).name(name.to_string());

			PlatformShaderCompiler::new()
				.generate(&settings, &root)
				.await
				.unwrap_or_else(|error| panic!("{name} should compile for the platform shader language: {error}"));
		}
	}

	#[test]
	fn bloom_level_count_is_clamped() {
		let settings = BloomPassSettings {
			levels: MAX_BLOOM_LEVELS + 4,
			..Default::default()
		};

		assert_eq!(settings.resolved_level_count(), MAX_BLOOM_LEVELS as usize);
	}

	#[test]
	fn bloom_extent_stays_non_zero() {
		let extent = bloom_extent(Extent::rectangle(1, 1), 4);

		assert_eq!(extent.width(), 1);
		assert_eq!(extent.height(), 1);
	}
}
