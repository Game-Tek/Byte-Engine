use ghi::{
	command_buffer::CommonCommandBufferMode as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use maths_rs::Vec3f;

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute},
		render_passes::bloom::BloomShaderData,
		renderer::scaled_extent,
	},
};

const MAX_GHOSTS: u32 = 8;
/// The prefilter runs at half the sink resolution, like the first bloom level.
const HALF_RESOLUTION: u32 = 2;
/// Ghosts are soft, so the flare gathers them at quarter resolution.
const QUARTER_RESOLUTION: u32 = 4;

/// The `LensFlarePassSettings` struct defines the look of the screen-space lens flare stage.
///
/// Pass it to [`crate::application::graphics::setup_lens_flare_render_pass`] or [`LensFlarePass::with_settings`].
#[derive(Clone, Copy, Debug)]
pub struct LensFlarePassSettings {
	/// Scene-linear brightness, after exposure, above which light starts to flare. The prefilter averages each
	/// source with its neighbors first, so a small specular highlight far above display white arrives near `2.0`;
	/// raise the threshold above that to flare only large sources such as the sun.
	pub threshold: f32,
	/// Fraction of the threshold over which the flare fades in, from `0.0` for a hard cut to `1.0`.
	pub soft_knee: f32,
	/// Scene-linear brightness, after exposure, that a texel is held to before it flares. It stops a source at the
	/// half-float limit from washing the whole frame with ghosts.
	pub max_brightness: f32,
	/// Scale of the flare added onto the scene. `0.0` passes the scene through.
	pub intensity: f32,
	/// Ghosts cast by each bright source, from `1` to `8`.
	pub ghost_count: u32,
	/// Distance between ghosts, as a fraction of the distance from the mirrored source to the frame center.
	pub ghost_spacing: f32,
	/// Radius of the halo ring in UV. A source near the frame center is ringed at this distance.
	pub halo_width: f32,
	/// Strength of the halo relative to one ghost. `0.0` removes the halo.
	pub halo_intensity: f32,
	/// UV distance between the red and blue taps of each flare sample. Larger values fringe ghosts with color.
	pub chromatic_distortion: f32,
	/// Linear color the flare is multiplied by, standing in for the lens coatings. The default is the cool blue of
	/// anamorphic lenses; use white for an untinted flare.
	pub tint: Vec3f,
	/// Horizontal squeeze ratio of the emulated lens. `1.0` is a spherical lens; anamorphic lenses are commonly
	/// `1.33`, `1.5`, or `2.0`. Larger values widen the halo into an ellipse and spread each ghost horizontally.
	/// Values below `1.0` are treated as `1.0`.
	pub squeeze: f32,
}

impl Default for LensFlarePassSettings {
	fn default() -> Self {
		Self {
			threshold: 1.0,
			soft_knee: 0.5,
			max_brightness: 64.0,
			intensity: 0.05,
			ghost_count: 4,
			ghost_spacing: 0.35,
			halo_width: 0.45,
			halo_intensity: 0.5,
			chromatic_distortion: 0.004,
			tint: Vec3f::new(0.35, 0.6, 1.0),
			squeeze: 1.0,
		}
	}
}

impl LensFlarePassSettings {
	fn resolved_ghost_count(self) -> u32 {
		self.ghost_count.clamp(1, MAX_GHOSTS)
	}
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LensFlareShaderData {
	ghosts: [f32; 4],
	tint: [f32; 4],
	anamorphic: [f32; 4],
}

/// The `LensFlarePass` struct adds the ghosts and halo that bright light casts inside a camera lens.
///
/// The pass works in screen space, so every bright source on screen flares and a source hidden behind geometry
/// does not. It reuses the bloom prefilter and downsample shaders to find bright light at quarter resolution,
/// gathers ghosts and a halo mirrored through the frame center, then tints the result and adds it onto the HDR
/// scene. Install it after the passes that write scene light and before bloom and tone mapping, so the ghosts
/// pick up a little glow. Toggle it at runtime with the `render.pass.lens-flare` parameter.
pub struct LensFlarePass {
	settings: LensFlarePassSettings,
	bypass_pass: crate::rendering::render_passes::blit::ImageBypassPass,
	prefilter_parameters: ghi::DynamicBufferHandle<BloomShaderData>,
	feature_parameters: ghi::DynamicBufferHandle<LensFlareShaderData>,
	extract_pass: simple_compute::Pass,
	downsample_pass: simple_compute::Pass,
	features_pass: simple_compute::Pass,
	composite_pass: simple_compute::Pass,
}

impl Entity for LensFlarePass {}

impl LensFlarePass {
	/// Creates a lens flare pass with the default blue-tinted look.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		Self::with_settings(render_pass_builder, LensFlarePassSettings::default())
	}

	/// Creates a lens flare pass with caller-supplied settings and remaps `main` for downstream passes.
	// Keep the four-stage resource graph together so each stage's inputs sit next to the stage that writes them.
	#[allow(clippy::too_many_lines)]
	pub fn with_settings(render_pass_builder: &mut RenderPassBuilder, settings: LensFlarePassSettings) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("Lens Flare Output"),
		);

		// Render targets, so the renderer sizes them with the sink and each stage can be captured by name.
		let flare_target = |name| {
			ghi::image::Builder::new(crate::rendering::SCENE_COLOR_FORMAT, ghi::Uses::Storage | ghi::Uses::Image)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let half_image =
			render_pass_builder.create_scaled_render_target(flare_target("Lens Flare Half Resolution"), HALF_RESOLUTION);
		let quarter_image =
			render_pass_builder.create_scaled_render_target(flare_target("Lens Flare Quarter Resolution"), QUARTER_RESOLUTION);
		let flare_image =
			render_pass_builder.create_scaled_render_target(flare_target("Lens Flare Features"), QUARTER_RESOLUTION);

		let context = render_pass_builder.context();
		let prefilter_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lens Flare Prefilter Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let feature_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Lens Flare Feature Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp),
		);

		// The bloom prefilter, downsample, and composite shaders do exactly what the flare needs around its own
		// feature shader, so the flare binds them to its own images and parameters.
		let extract_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Lens Flare Extract", "byte-engine/rendering/bloom/extract.pipeline"),
		)
		.expect(
			"Failed to create the lens flare extract shader. The most likely cause is an incompatible bloom extract shader interface.",
		);
		let downsample_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Lens Flare Downsample", "byte-engine/rendering/bloom/downsample.pipeline"),
		)
		.expect(
			"Failed to create the lens flare downsample shader. The most likely cause is an incompatible bloom downsample shader interface.",
		);
		let features_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Lens Flare Features", "byte-engine/rendering/lens-flare/features.pipeline"),
		)
		.expect(
			"Failed to create the lens flare features shader. The most likely cause is an incompatible lens flare shader interface.",
		);
		let composite_pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Lens Flare Composite", "byte-engine/rendering/bloom/composite.pipeline"),
		)
		.expect(
			"Failed to create the lens flare composite shader. The most likely cause is an incompatible bloom composite shader interface.",
		);

		let extract_pass = extract_pipeline
			.bind(
				render_pass_builder,
				"Lens Flare Extract Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", half_image),
					simple_compute::Resource::buffer("bloom_parameters", prefilter_parameters),
				],
			)
			.expect("Failed to bind lens flare extract resources. The most likely cause is a changed BESL binding contract.");
		let downsample_pass = downsample_pipeline
			.bind(
				render_pass_builder,
				"Lens Flare Downsample Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("source_texture", half_image, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", quarter_image),
				],
			)
			.expect(
				"Failed to bind lens flare downsample resources. The most likely cause is a changed BESL binding contract.",
			);
		let features_pass = features_pipeline
			.bind(
				render_pass_builder,
				"Lens Flare Features Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler(
						"source_texture",
						quarter_image,
						sampler,
						ghi::Layouts::Read,
					),
					simple_compute::Resource::image("result_texture", flare_image),
					simple_compute::Resource::buffer("lens_flare_parameters", feature_parameters),
				],
			)
			.expect("Failed to bind lens flare feature resources. The most likely cause is a changed BESL binding contract.");
		let composite_pass = composite_pipeline
			.bind(
				render_pass_builder,
				"Lens Flare Composite Descriptor Set",
				&[
					simple_compute::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
					simple_compute::Resource::combined_image_sampler("bloom_texture", flare_image, sampler, ghi::Layouts::Read),
					simple_compute::Resource::image("result_texture", output),
					simple_compute::Resource::buffer("bloom_parameters", prefilter_parameters),
				],
			)
			.expect("Failed to bind lens flare composite resources. The most likely cause is a changed BESL binding contract.");
		let bypass_pass = crate::rendering::render_passes::blit::ImageBypassPass::new(render_pass_builder, source, output);

		Self {
			settings,
			bypass_pass,
			prefilter_parameters,
			feature_parameters,
			extract_pass,
			downsample_pass,
			features_pass,
			composite_pass,
		}
	}

	/// Writes the flare controls into the per-frame parameter buffers before dispatch.
	fn write_parameters(&self, frame: &mut ghi::implementation::Frame) {
		let settings = self.settings;

		let prefilter = frame.get_mut_dynamic_buffer_slice(self.prefilter_parameters);
		prefilter.prefilter = [
			settings.threshold.max(0.0),
			settings.soft_knee.clamp(0.0, 1.0),
			settings.intensity.max(0.0),
			settings.max_brightness.max(0.0),
		];
		prefilter.filter = [0.0; 4];

		let features = frame.get_mut_dynamic_buffer_slice(self.feature_parameters);
		features.ghosts = [
			settings.resolved_ghost_count() as f32,
			settings.ghost_spacing,
			settings.halo_width.max(0.0),
			settings.halo_intensity.max(0.0),
		];
		features.tint = [
			settings.tint.x.max(0.0),
			settings.tint.y.max(0.0),
			settings.tint.z.max(0.0),
			settings.chromatic_distortion.max(0.0),
		];
		features.anamorphic = [settings.squeeze.max(1.0), 0.0, 0.0, 0.0];
	}
}

impl RenderPass for LensFlarePass {
	fn name(&self) -> &'static str {
		"lens-flare"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let extract_pass = self.extract_pass.ready(frame)?;
		let downsample_pass = self.downsample_pass.ready(frame)?;
		let features_pass = self.features_pass.ready(frame)?;
		let composite_pass = self.composite_pass.ready(frame)?;
		let extent = sink.extent();
		let half_extent = scaled_extent(extent, HALF_RESOLUTION);
		let quarter_extent = scaled_extent(extent, QUARTER_RESOLUTION);
		self.write_parameters(frame);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer, _| {
				command_buffer.region(
					|label| label.write_str("Lens Flare"),
					|command_buffer| {
						extract_pass.record(command_buffer, half_extent);
						downsample_pass.record(command_buffer, quarter_extent);
						features_pass.record(command_buffer, quarter_extent);
						composite_pass.record(command_buffer, extent);
					},
				);
			},
		))
	}

	crate::rendering::render_pass::forward_to_inner_pass!(bypass = bypass_pass);
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Value};

	use super::*;
	use crate::rendering::shader_vm_test::{buffer, empty_image, rgba, run_at, texture_2d};

	const LENS_FLARE_FEATURES_BESL: &str = include_str!("../../../assets/rendering/lens-flare/features.besl");

	/// The controls a features test varies; everything else keeps a neutral value.
	#[derive(Clone, Copy)]
	struct Features {
		ghost_count: f32,
		ghost_spacing: f32,
		halo_width: f32,
		halo_intensity: f32,
		tint: [f32; 3],
		distortion: f32,
		squeeze: f32,
	}

	/// One mirrored ghost, no halo, no tint, and no dispersion.
	const SINGLE_GHOST: Features = Features {
		ghost_count: 1.0,
		ghost_spacing: 0.0,
		halo_width: 0.0,
		halo_intensity: 0.0,
		tint: [1.0, 1.0, 1.0],
		distortion: 0.0,
		squeeze: 1.0,
	};

	/// The halo of a centered source and nothing else.
	const HALO_ONLY: Features = Features {
		ghost_count: 0.0,
		halo_width: 0.25,
		halo_intensity: 1.0,
		..SINGLE_GHOST
	};

	/// Runs the features shader over a `width` by `height` source, returning every result texel in row-major order.
	fn features(width: u32, height: u32, source_texels: &[[f32; 4]], controls: Features) -> Vec<[f32; 4]> {
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(LENS_FLARE_FEATURES_BESL));
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write_indexed(
				"ghosts",
				0,
				Value::Vec4F([
					controls.ghost_count,
					controls.ghost_spacing,
					controls.halo_width,
					controls.halo_intensity,
				]),
			)
			.expect("Failed to initialize lens flare parameters. The most likely cause is a changed production buffer layout.");
		parameters
			.write_indexed(
				"tint",
				0,
				Value::Vec4F([controls.tint[0], controls.tint[1], controls.tint[2], controls.distortion]),
			)
			.expect("Failed to initialize lens flare parameters. The most likely cause is a changed production buffer layout.");
		parameters
			.write_indexed("anamorphic", 0, Value::Vec4F([controls.squeeze, 0.0, 0.0, 0.0]))
			.expect("Failed to initialize lens flare parameters. The most likely cause is a changed production buffer layout.");
		let mut source = texture_2d(width, height, source_texels);
		let mut result = empty_image(width, height);
		for y in 0..height {
			for x in 0..width {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(ResourceSlot::new(0), &mut source);
				descriptors.bind_image(ResourceSlot::new(1), &mut result);
				descriptors.bind_buffer(parameter_slot, &mut parameters);
				run_at(&program, &mut descriptors, [x, y]);
			}
		}
		(0..width * height)
			.map(|index| rgba(&result, [index % width, index / width]))
			.collect()
	}

	/// Returns an 8x8 black source with one bright texel at `bright`.
	fn single_source(bright: [u32; 2]) -> Vec<[f32; 4]> {
		(0..64)
			.map(|index| {
				if [index % 8, index / 8] == bright {
					[10.0, 10.0, 10.0, 1.0]
				} else {
					[0.0, 0.0, 0.0, 1.0]
				}
			})
			.collect()
	}

	/// Returns a 16x8 black source with a bright 2x2 block at its center.
	fn centered_source() -> Vec<[f32; 4]> {
		(0..16 * 8)
			.map(|index| {
				let [x, y] = [index % 16, index / 16];
				if (7..=8).contains(&x) && (3..=4).contains(&y) {
					[10.0, 10.0, 10.0, 1.0]
				} else {
					[0.0, 0.0, 0.0, 1.0]
				}
			})
			.collect()
	}

	fn brightest_texel(texels: &[[f32; 4]], width: u32) -> [u32; 2] {
		let index = texels
			.iter()
			.enumerate()
			.max_by(|(_, a), (_, b)| a[1].total_cmp(&b[1]))
			.map(|(index, _)| index as u32)
			.expect("result should not be empty");
		[index % width, index / width]
	}

	/// Verifies that a bright source casts its ghost at the position mirrored through the frame center.
	#[test]
	fn lens_flare_features_besl_vm_mirrors_a_source_through_the_center() {
		let result = features(8, 8, &single_source([2, 1]), SINGLE_GHOST);

		assert_eq!(brightest_texel(&result, 8), [5, 6], "{result:?}");
	}

	/// Verifies that a black scene casts no flare, and that zero ghosts and zero halo contribute nothing.
	#[test]
	fn lens_flare_features_besl_vm_is_black_without_light_or_features() {
		let black = vec![[0.0, 0.0, 0.0, 1.0]; 64];
		let no_features = Features {
			ghost_count: 0.0,
			halo_width: 0.3,
			..SINGLE_GHOST
		};

		for (source, controls) in [(black, SINGLE_GHOST), (single_source([2, 1]), no_features)] {
			let result = features(8, 8, &source, controls);
			assert!(result.iter().all(|texel| texel[..3] == [0.0; 3]), "{result:?}");
		}
	}

	/// Verifies that the tint colors the flare channel by channel, so a blue tint leaves a blue flare.
	#[test]
	fn lens_flare_features_besl_vm_applies_the_tint() {
		let untinted = features(8, 8, &single_source([2, 1]), SINGLE_GHOST);
		let blue = features(
			8,
			8,
			&single_source([2, 1]),
			Features {
				tint: [0.25, 0.5, 1.0],
				..SINGLE_GHOST
			},
		);

		let ghost = 6 * 8 + 5;
		assert!(untinted[ghost][2] > 0.0, "{untinted:?}");
		for channel in 0..3 {
			let expected = untinted[ghost][channel] * [0.25, 0.5, 1.0][channel];
			assert!((blue[ghost][channel] - expected).abs() < 1e-5, "{blue:?}");
		}
	}

	/// Verifies that the halo rings a centered source at the same UV radius horizontally and vertically on a wide
	/// target, so it stays round instead of stretching with the aspect ratio.
	#[test]
	fn lens_flare_features_besl_vm_keeps_the_halo_round_on_a_wide_target() {
		let result = features(16, 8, &centered_source(), HALO_ONLY);

		// A quarter of the frame height away from the center, measured in pixels on both axes.
		let horizontal = result[4 * 16 + 8 + 2][1];
		let vertical = result[(4 + 2) * 16 + 8][1];
		assert!(horizontal > 0.0, "{result:?}");
		assert!(
			(horizontal - vertical).abs() < 0.05 * horizontal,
			"{horizontal} vs {vertical}"
		);
	}

	/// Verifies that a squeeze of two widens the halo into an ellipse twice as wide as it is tall.
	#[test]
	fn lens_flare_features_besl_vm_squeeze_widens_the_halo() {
		let round = features(16, 8, &centered_source(), HALO_ONLY);
		let squeezed = features(
			16,
			8,
			&centered_source(),
			Features {
				squeeze: 2.0,
				..HALO_ONLY
			},
		);

		// The halo radius is two pixels vertically, so a squeeze of two moves the horizontal ring out to four.
		let vertical = (4 + 2) * 16 + 8;
		let wide = 4 * 16 + 8 + 4;
		assert!(squeezed[vertical][1] > 0.0, "{squeezed:?}");
		assert!(squeezed[wide][1] > 0.5 * squeezed[vertical][1], "{squeezed:?}");
		assert!(round[wide][1] < 0.1 * round[vertical][1], "{round:?}");
	}

	/// Verifies that squeeze spreads a ghost sideways without spreading it vertically.
	#[test]
	fn lens_flare_features_besl_vm_squeeze_stretches_ghosts_horizontally() {
		let spherical = features(8, 8, &single_source([2, 1]), SINGLE_GHOST);
		let anamorphic = features(
			8,
			8,
			&single_source([2, 1]),
			Features {
				squeeze: 3.0,
				..SINGLE_GHOST
			},
		);

		// The ghost sits at (5, 6); a squeeze of three spreads it two texels to each side.
		for side in [6 * 8 + 3, 6 * 8 + 7] {
			assert_eq!(spherical[side][1], 0.0, "{spherical:?}");
			assert!(anamorphic[side][1] > 0.0, "{anamorphic:?}");
		}
		assert_eq!(anamorphic[5 * 8 + 5][1], 0.0, "{anamorphic:?}");
	}

	/// Lowers the lens flare features program through the platform shader compiler. The VM tests above prove the
	/// program links and behaves as BESL; defects the backend mishandles only surface when the real platform
	/// compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn lens_flare_besl_program_lowers_to_the_platform_shader_language() {
		use resource_management::shader::ShaderGenerationSettings;
		use resource_management::shader::besl::backends::platform::PlatformShaderCompiler;
		use utils::Extent;

		let mut root = besl::parse(LENS_FLARE_FEATURES_BESL).expect("lens flare shader should parse");
		root.add(vec![crate::rendering::common_shader_generator::CommonShaderScope::new()]);
		let root = besl::lex(root).expect("lens flare shader should link");
		let settings = ShaderGenerationSettings::compute(Extent::rectangle(8, 8)).name("lens_flare_features".to_string());

		PlatformShaderCompiler::new()
			.generate(&settings, &root)
			.await
			.unwrap_or_else(|error| panic!("lens flare features should compile for the platform shader language: {error}"));
	}

	#[test]
	fn lens_flare_ghost_count_is_clamped() {
		for (requested, resolved) in [(0, 1), (MAX_GHOSTS + 4, MAX_GHOSTS)] {
			let settings = LensFlarePassSettings {
				ghost_count: requested,
				..Default::default()
			};
			assert_eq!(settings.resolved_ghost_count(), resolved);
		}
	}
}
