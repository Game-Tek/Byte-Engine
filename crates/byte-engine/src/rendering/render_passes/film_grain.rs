//! Film grain that imitates scanned 35 mm, 16 mm, and 8 mm stock, applied to the display-encoded picture.
//!
//! The grain model follows cinegrain2 by mr-berndt, as published under the MIT license up to commit `fd9e8f3`; see
//! `THIRD_PARTY_NOTICES.md`. White noise is shaped like the scans' grain, a narrow Gaussian blur minus a wide one, and
//! changes the picture's density multiplicatively, with a five-zone equalizer over luma. Install it with
//! [`crate::application::graphics::setup_film_grain_render_pass`].

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_tile},
		render_passes::blit::ImageBypassPass,
	},
};

/// The `FilmGrainPassSettings` struct sets how strong and how coarse the grain is, and how its strength follows the
/// picture's brightness.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilmGrainPassSettings {
	/// The grain strength. `1` matches the scans of real stock on a 50 % grey card; `0` turns the grain off.
	pub level: f32,
	/// The grain size: the narrow blur's standard deviation in pixels of a 2160-pixel-tall picture, scaled with the
	/// sink's height. About `0.4` looks like 35 mm, `1.2` like 16 mm, and `2.7` like 8 mm.
	pub size: f32,
	/// The equalizer gains of the Black, Shadow, Mid, High, and White zones, at luma 0, 0.13, 0.30, 0.60, and 1. `0`
	/// keeps the grain as strong as at mid grey, `-1` removes it, and `3` makes it four times as strong.
	pub zone_gains: [f32; 5],
}

impl Default for FilmGrainPassSettings {
	/// The 35 mm look: fine grain that thins in the highlights, as on the scans.
	fn default() -> Self {
		Self {
			level: 1.0,
			size: 0.37,
			zone_gains: [0.0, 0.65, 0.0, -0.65, -2.0],
		}
	}
}

/// The `FilmGrainShaderData` struct mirrors `FilmGrainParameters` in `film-grain/layouts/grain.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct FilmGrainShaderData {
	zone_weights: [f32; 4],
	white_weight: f32,
	narrow_sigma: f32,
	narrow_radius: u32,
	wide_sigma: f32,
	wide_radius: u32,
	normalization: f32,
	amplitude: f32,
	seed: u32,
}

/// The picture height, in pixels, at which `size` is measured and the scans were taken.
const REFERENCE_HEIGHT: f32 = 2160.0;
/// How many times wider the wide blur is than the narrow one, fitted to the scans' power spectra.
const WIDE_SCALE: f32 = 8.4;
/// The wide part's share of the difference of blurs. Must match `GRAIN_WIDE_SHARE` in the shader layer.
const WIDE_SHARE: f32 = 0.95;
/// The variance, in square pixels, that averaging 4x4 blocks and filtering them back up adds to the wide blur.
const WIDE_FLOOR: f32 = 3.92;
/// Below this standard deviation, in quarter-resolution cells, the wide blur changes nothing visible and is skipped.
const MIN_WIDE_SIGMA: f32 = 0.3;
const MAX_NARROW_RADIUS: u32 = 8;
const MAX_WIDE_RADIUS: u32 = 16;
/// The smallest narrow standard deviation, in pixels, so the weights stay finite.
const MIN_NARROW_SIGMA: f32 = 0.05;
/// The density exponent per unit-variance grain at level 1 on a 50 % grey card: cinegrain2's grain scale of 0.405
/// times its level reference of 0.210.
const LEVEL_SCALE: f32 = 0.405 * 0.210;

/// Builds the grain constants for a sink `height` pixels tall, for the frame numbered `frame`.
///
/// Everything that is the same for every pixel is computed here: the blur widths, the scale that brings the difference
/// of blurs to unit variance, and the equalizer weights divided by the weight at mid grey, so `level` keeps its meaning
/// whatever the zones do.
fn film_grain_shader_data(settings: &FilmGrainPassSettings, height: u32, frame: u32) -> FilmGrainShaderData {
	let narrow_sigma = (settings.size * height as f32 / REFERENCE_HEIGHT).max(MIN_NARROW_SIGMA);
	let narrow_radius = ((2.5 * narrow_sigma).ceil() as u32).min(MAX_NARROW_RADIUS);

	// The wide blur runs on quarter-resolution cells, which already hold WIDE_FLOOR of its variance.
	let wide_pixels = WIDE_SCALE * narrow_sigma;
	let wide_sigma = (wide_pixels * wide_pixels - WIDE_FLOOR).max(0.0).sqrt() * 0.25;
	let (wide_sigma, wide_radius) = if wide_sigma < MIN_WIDE_SIGMA {
		(1.0, 0)
	} else {
		(wide_sigma, ((2.5 * wide_sigma).ceil() as u32).min(MAX_WIDE_RADIUS))
	};

	let [black, shadow, mid, high, white] = settings.zone_gains.map(|gain| 1.0 + gain);
	let mid_grey = zone_weight_at_mid_grey(mid, high).max(0.05);

	FilmGrainShaderData {
		zone_weights: [black / mid_grey, shadow / mid_grey, mid / mid_grey, high / mid_grey],
		white_weight: white / mid_grey,
		narrow_sigma,
		narrow_radius,
		wide_sigma,
		wide_radius,
		normalization: unit_variance_scale(narrow_sigma, narrow_radius),
		amplitude: settings.level.max(0.0) * LEVEL_SCALE * downscale_retention(settings.size, height),
		// Golden-ratio steps spread consecutive frames over the whole seed range.
		seed: frame.wrapping_mul(0x9E37_79B9),
	}
}

/// Returns the equalizer weight at luma 0.5, which lies between the Mid zone at 0.30 and the High zone at 0.60.
fn zone_weight_at_mid_grey(mid: f32, high: f32) -> f32 {
	let t: f32 = (0.5 - 0.30) / 0.30;
	let eased = t * t * (3.0 - 2.0 * t);
	(mid + (high - mid) * eased).max(0.0)
}

/// Returns the scale that brings the difference of blurs over noise of variance 1/3 to unit variance.
///
/// The narrow part's variance is exact, from its discrete separable weights. The wide part and the cross term use
/// continuous Gaussians, with the wide width including the variance the quarter-resolution grid adds.
fn unit_variance_scale(narrow_sigma: f32, narrow_radius: u32) -> f32 {
	let (sum, squares) = (1..=narrow_radius).fold((1.0f32, 1.0f32), |(sum, squares), distance| {
		let distance = distance as f32;
		let weight = (-0.5 * distance * distance / (narrow_sigma * narrow_sigma)).exp();
		(sum + 2.0 * weight, squares + 2.0 * weight * weight)
	});
	// The 1-D variance factor of the normalized kernel; the 2-D kernel is separable, so it squares.
	let narrow = squares / (sum * sum);
	let wide = (WIDE_SCALE * narrow_sigma).max(WIDE_FLOOR.sqrt());
	let variance = narrow * narrow - 2.0 * WIDE_SHARE / (std::f32::consts::TAU * (narrow_sigma * narrow_sigma + wide * wide))
		+ WIDE_SHARE * WIDE_SHARE / (2.0 * std::f32::consts::TAU * wide * wide);
	(variance / 3.0).max(1e-8).sqrt().recip()
}

/// Returns the share of the 2160-pixel grain's standard deviation that a picture `height` pixels tall can show.
///
/// The grain imitated is the film's own as a 2160-pixel scan shows it. Shrinking that scan to fewer pixels averages
/// fine grain away and keeps coarse grain, so this is the standard deviation of one downscaled sample of the
/// 2160-pixel grain field, through a Hermite downscaling kernel widened by the scale factor, averaged over 8 sample
/// phases. The field is separable, so the 1-D variance is the 2-D field's standard deviation factor. It is 1 at 2160
/// pixels and taller.
fn downscale_retention(size: f32, height: u32) -> f32 {
	const MAX_TAPS: usize = 24;
	const MAX_RADIUS: usize = MAX_NARROW_RADIUS as usize;

	let factor = REFERENCE_HEIGHT / height.max(1) as f32;
	if factor <= 1.0 {
		return 1.0;
	}
	let factor = factor.min(0.5 * (MAX_TAPS - 2) as f32);
	let sigma = size.max(MIN_NARROW_SIGMA);
	let radius = ((2.5 * sigma).ceil() as usize).min(MAX_RADIUS);

	// The 2160-pixel kernel's autocorrelation, normalized so lag 0 is 1.
	let weight = |distance: usize| (-0.5 * (distance * distance) as f32 / (sigma * sigma)).exp();
	let mut correlation = [0.0f32; 2 * MAX_RADIUS + 1];
	for (lag, value) in correlation.iter_mut().enumerate().take(2 * radius + 1) {
		*value = (-(radius as isize)..=radius as isize)
			.filter(|tap| (tap + lag as isize).unsigned_abs() <= radius)
			.map(|tap| weight(tap.unsigned_abs()) * weight((tap + lag as isize).unsigned_abs()))
			.sum();
	}
	let zero_lag = correlation[0];
	correlation.iter_mut().for_each(|value| *value /= zero_lag);

	let hermite = |x: f32| {
		let x = x.abs();
		if x < 1.0 { (2.0 * x - 3.0) * x * x + 1.0 } else { 0.0 }
	};
	let variance: f32 = (0..8)
		.map(|phase| {
			let center = (phase as f32 + 0.5) / 8.0;
			let first = (center - factor).floor() as isize + 1;
			let mut taps = [0.0f32; MAX_TAPS];
			for (index, tap) in taps.iter_mut().enumerate() {
				*tap = hermite(((first + index as isize) as f32 - center) / factor);
			}
			let total: f32 = taps.iter().sum();
			let mut sample = 0.0;
			for (t, a) in taps.iter().enumerate() {
				for (u, b) in taps.iter().enumerate() {
					let lag = t.abs_diff(u);
					if lag <= 2 * radius {
						sample += a * b * correlation[lag];
					}
				}
			}
			sample / (total * total)
		})
		.sum();
	(variance / 8.0).clamp(0.0, 1.0)
}

/// The `FilmGrainPass` struct gives a finished, display-encoded picture the grain of scanned film stock.
///
/// It is the last image effect before overlays: install it with
/// [`crate::application::graphics::setup_film_grain_render_pass`] after tone mapping, color grading, and
/// anti-aliasing, so none of them smooths the grain away, and before the UI, so the UI stays clean. Each frame draws
/// new grain. One tile pass builds the whole grain in workgroup memory and applies it to `main`, so no grain image
/// reaches memory. Splitting it into passes, compute or tile, costs several times the grain's own work. Toggle it at
/// runtime with `render.pass.film-grain`.
pub struct FilmGrainPass {
	settings: FilmGrainPassSettings,
	apply_pass: simple_tile::Pass,
	/// Forwards `main` unchanged while the grain is off or its shaders are still compiling.
	main_copy: ImageBypassPass,
	parameters: ghi::DynamicBufferHandle<FilmGrainShaderData>,
	frame: u32,
}

impl Entity for FilmGrainPass {}

impl FilmGrainPass {
	/// Creates the grain pass with `settings` and remaps `main` for downstream passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, settings: FilmGrainPassSettings) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("Film Grain"));
		// The builder's copy of the incoming `main` into `output` doubles as this pass's forwarding copy.
		let main_copy = render_pass_builder.take_main_copy().expect(
			"Main copy is missing. The most likely cause is that the pass did not call `RenderPassBuilder::create_main_render_target` first.",
		);

		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Film Grain Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let sampler = context.build_sampler(ghi::sampler::Builder::new());

		let apply_pass = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Film Grain Apply",
				"byte-engine/rendering/film-grain/apply.pipeline",
				&[output.format()],
			),
		)
		.bind(
			"Film Grain Apply Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("parameters", parameters),
			],
			&[simple_tile::Output::new("result", output)],
		);

		Self {
			settings,
			apply_pass,
			main_copy,
			parameters,
			frame: 0,
		}
	}
}

impl RenderPass for FilmGrainPass {
	fn name(&self) -> &'static str {
		"film-grain"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// No grain, or a shader still compiling, forwards `main` unchanged.
		let ready = if self.settings.level > 0.0 {
			self.apply_pass.ready(frame)
		} else {
			None
		};
		let Some(apply) = ready else {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		let extent = sink.extent();
		self.frame = self.frame.wrapping_add(1);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) =
			film_grain_shader_data(&self.settings, extent.height(), self.frame);

		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("Film Grain"),
					|command_buffer| apply.record(command_buffer, extent),
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
		// This pass took the builder's forwarding copy, so it forwards `main` itself.
		self.main_copy.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Texture, Value, WorkgroupState, output_slot};

	use super::*;
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{asset_source, binding_slot, buffer, compile, empty_image, rgba, texture_2d, tile_lanes, tiles},
	};

	/// The tile and lanes per side of the grain pass, as `apply.besl.bead` declares them.
	const TILE: u32 = 16;

	/// Runs the grain shader over a `size` by `size` scene with the constants `data`, as `FilmGrainPass` records it.
	fn grain(size: u32, scene: &[[f32; 4]], data: &FilmGrainShaderData) -> Texture {
		let linked = simple_compute::compile_test_program(asset_source("film-grain/apply.besl"));
		let slot = binding_slot(&linked, "parameters");
		let program = compile(linked);
		let mut parameters = buffer(&program, slot);
		for (name, value) in [
			("zone_weights", Value::Vec4F(data.zone_weights)),
			("white_weight", Value::F32(data.white_weight)),
			("narrow_sigma", Value::F32(data.narrow_sigma)),
			("narrow_radius", Value::U32(data.narrow_radius)),
			("wide_sigma", Value::F32(data.wide_sigma)),
			("wide_radius", Value::U32(data.wide_radius)),
			("normalization", Value::F32(data.normalization)),
			("amplitude", Value::F32(data.amplitude)),
			("seed", Value::U32(data.seed)),
		] {
			parameters.write(name, value).expect(
				"Failed to initialize film grain parameters. The most likely cause is a changed production buffer layout.",
			);
		}

		let mut scene = texture_2d(size, size, scene);
		let mut result = empty_image(size, size);
		for position in tiles(TILE, [size, size]) {
			let mut workgroup = WorkgroupState::new();
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(ResourceSlot::new(0), &mut scene);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_buffer(slot, &mut parameters);
			descriptors.bind_workgroup_state(&mut workgroup);
			program
				.run_workgroup(&mut descriptors, &tile_lanes(position, TILE, TILE, [size, size]))
				.expect(
					"Failed to execute a film grain tile with the BESL VM. The most likely cause is invalid workgroup memory synchronization.",
				);
		}
		result
	}

	/// Verifies that on a 50 % grey card the grain changes density by `amplitude` standard deviations around no
	/// change, which is what makes level 1 as strong as the scans.
	#[test]
	fn grey_card_grain_has_unit_variance_density() {
		const SIZE: u32 = 96;
		// 16 mm grain at the scans' height, so both blurs span several texels of the small fixture.
		let settings = FilmGrainPassSettings {
			size: 1.2,
			..Default::default()
		};
		let data = film_grain_shader_data(&settings, 2160, 7);
		let result = grain(SIZE, &vec![[0.5, 0.5, 0.5, 1.0]; (SIZE * SIZE) as usize], &data);

		let grains: Vec<f32> = (0..SIZE * SIZE)
			.map(|index| (rgba(&result, [index % SIZE, index / SIZE])[1] / 0.5).ln() / data.amplitude)
			.collect();
		let mean = grains.iter().sum::<f32>() / grains.len() as f32;
		let deviation = (grains.iter().map(|grain| (grain - mean).powi(2)).sum::<f32>() / grains.len() as f32).sqrt();
		assert!(
			mean.abs() < 0.1 && (deviation - 1.0).abs() < 0.15,
			"Grey-card grain has mean {mean} and standard deviation {deviation}, not 0 and 1. The most likely cause is a normalization that no longer matches the blurs. Compare `unit_variance_scale` with the shader's kernels."
		);
	}

	/// Verifies that with the default zones, white takes no grain and mid grey does.
	#[test]
	fn default_zones_leave_white_clean() {
		const SIZE: u32 = 16;
		let data = film_grain_shader_data(&FilmGrainPassSettings::default(), 2160, 3);
		let scene: Vec<[f32; 4]> = (0..SIZE * SIZE)
			.map(|index| {
				if index % SIZE < SIZE / 2 {
					[1.0; 4]
				} else {
					[0.5, 0.5, 0.5, 1.0]
				}
			})
			.collect();
		let result = grain(SIZE, &scene, &data);

		for y in 0..SIZE {
			assert_eq!(rgba(&result, [1, y]), [1.0; 4], "White took grain in row {y}.");
		}
		assert!(
			(0..SIZE).any(|y| rgba(&result, [SIZE - 2, y])[1] != 0.5),
			"Mid grey took no grain. The most likely cause is a zero amplitude or zone weight."
		);
	}

	/// Verifies that a picture at the scans' height, or taller, shows all of their grain, and a shorter one shows less,
	/// fine grain losing more than coarse grain.
	#[test]
	fn shorter_pictures_show_less_fine_grain() {
		assert_eq!(downscale_retention(0.37, 2160), 1.0);
		assert_eq!(downscale_retention(0.37, 4320), 1.0);

		let fine = downscale_retention(0.37, 1080);
		let coarse = downscale_retention(2.7, 1080);
		assert!(
			0.0 < fine && fine < coarse && coarse < 1.0,
			"Downscaled grain retention is out of order: fine {fine}, coarse {coarse}. The most likely cause is an autocorrelation computed over the wrong lags. Compare it with cinegrain2's grain amplitude pass."
		);
	}
}
