//! Color correction of the scene-linear picture: exposure, white balance, a tone curve, a split tone, and saturation.
//!
//! Exposure and white balance are linear, so they fold into one matrix built here, and the split tone's direction is
//! computed here once; the tone curve, split tone, and saturation run in the shader, in ACEScct. Install it with [`crate::application::graphics::setup_color_correction_render_pass`].

use ghi::context::{Context as _, ContextCreate as _};

use crate::{
	core::Entity,
	rendering::{
		Sink,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_tile},
	},
};

/// The `ColorCorrectionPassSettings` struct describes the correction and look a scene needs before tone mapping: how
/// bright, which light reads as white, how its shadows, highlights, and contrast fall, which hues its highlights and
/// shadows lean toward, and how strong and how dense its color is.
///
/// Pass it to [`crate::application::graphics::setup_color_correction_render_pass`]. The defaults leave the picture
/// unchanged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorCorrectionPassSettings {
	/// Stops added to the camera's exposure. `1` doubles the light and `-1` halves it.
	pub exposure: f32,
	/// The color temperature, in kelvins, of the light that shows as white, from 1,000 to 15,000. Lower values turn the
	/// picture bluer and higher values warmer.
	pub temperature: f32,
	/// How far the white lies from the Planckian locus, from `-1` to `1`. Positive values turn the picture magenta and
	/// negative values green.
	pub tint: f32,
	/// Brightens (`1`) or deepens (`-1`) the shadows, from -1 to 1. Black, mid grey, and brighter tones stay in place, so
	/// this recovers shadow detail that a LUT or output transform crushes without lifting black.
	pub shadows: f32,
	/// Brightens (`1`) or pulls down (`-1`) the highlights between mid grey and scene 64, from -1 to 1. Mid grey and
	/// brighter tones stay in place.
	pub highlights: f32,
	/// The Oklab hue, in degrees, that the highlights lean toward; the shadows lean toward the opposite hue. About `55`
	/// gives orange highlights and teal shadows.
	pub split_hue: f32,
	/// How far the split tone moves greys toward its hues, from 0 to 1. Black, mid grey, the brightest highlights, and
	/// already saturated colors, such as skin, keep most or all of their color.
	pub split_amount: f32,
	/// Shifts the split tone toward the shadows (`-1`, only shadows) or the highlights (`1`, only highlights).
	pub split_balance: f32,
	/// The slope of the S-curve at mid grey, above 0. Above `1` it deepens shadows and lifts highlights; mid grey stays.
	pub contrast: f32,
	/// The color strength, at or above 0. `0` is grey, `1` keeps the color, and higher values strengthen it.
	pub saturation: f32,
	/// Where saturation takes its light from, from 0 to 1. At `0` stronger color brightens its main channel; at `1` it
	/// only darkens the other channels, as more dye does in film.
	pub density: f32,
}

impl Default for ColorCorrectionPassSettings {
	fn default() -> Self {
		Self {
			exposure: 0.0,
			temperature: REFERENCE_TEMPERATURE as f32,
			tint: 0.0,
			shadows: 0.0,
			highlights: 0.0,
			split_hue: 0.0,
			split_amount: 0.0,
			split_balance: 0.0,
			contrast: 1.0,
			saturation: 1.0,
			density: 0.5,
		}
	}
}

/// The `ColorCorrectionShaderData` struct mirrors `ColorCorrectionParameters` in `color-correction/correct.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct ColorCorrectionShaderData {
	to_working: [[f32; 4]; 3],
	from_working: [[f32; 4]; 3],
	look: [f32; 4],
	tone: [f32; 4],
	split: [f32; 4],
}

type Matrix3 = [[f64; 3]; 3];

/// The temperature, in kelvins, that names D65 at a tint of 0, so a setting of it changes nothing.
const REFERENCE_TEMPERATURE: f64 = 6504.0;
/// The CIE 1960 *uv* distance from the locus that a tint of 1 moves the white.
const TINT_SCALE: f64 = 0.05;
/// The temperatures Krystek's approximation of the Planckian locus covers.
const TEMPERATURE_RANGE: std::ops::RangeInclusive<f32> = 1000.0..=15000.0;

/// Linear sRGB to CIE XYZ, for a D65 white.
const SRGB_TO_XYZ: Matrix3 = [
	[0.4124564, 0.3575761, 0.1804375],
	[0.2126729, 0.7151522, 0.0721750],
	[0.0193339, 0.1191920, 0.9503041],
];
/// CIE XYZ to CAT16 cone responses (Li et al., 2017).
const XYZ_TO_CAT16: Matrix3 = [
	[0.401288, 0.650173, -0.051461],
	[-0.250268, 1.204414, 0.045854],
	[-0.002079, 0.048952, 0.953127],
];
/// Linear sRGB to Oklab's cone responses (Ottosson, 2020).
const SRGB_TO_OKLAB_LMS: Matrix3 = [
	[0.4122214708, 0.5363325363, 0.0514459929],
	[0.2119034982, 0.6806995451, 0.1073969566],
	[0.0883024619, 0.2817188376, 0.6299787005],
];
/// Oklab's cube-rooted cone responses to L, a, and b.
const OKLAB_LMS_TO_LAB: Matrix3 = [
	[0.2104542553, 0.7936177850, -0.0040720468],
	[1.9779984951, -2.4285922050, 0.4505937099],
	[0.0259040371, 0.7827717662, -0.8086757660],
];
/// The luminance of each ACEScg channel.
const ACESCG_LUMINANCE: [f64; 3] = [0.2722287168, 0.6740817658, 0.0536895174];
/// The Oklab chroma that a split amount of 1 gives a grey at a mask's peak.
const SPLIT_CHROMA: f64 = 0.05;
/// ACEScct's steps per stop.
const ACESCCT_STOPS: f64 = 17.52;

/// Linear sRGB to ACEScg, D65 adapted to D60. Must match `linear_srgb_to_acescg` in `layouts/acescct.besl`.
const SRGB_TO_ACESCG: Matrix3 = [
	[0.6130974024, 0.3395231462, 0.0473794514],
	[0.0701937225, 0.9163538791, 0.0134523985],
	[0.0206155929, 0.1095697729, 0.8698146342],
];

fn multiply(a: &Matrix3, b: &Matrix3) -> Matrix3 {
	std::array::from_fn(|row| std::array::from_fn(|column| (0..3).map(|k| a[row][k] * b[k][column]).sum()))
}

fn apply(m: &Matrix3, v: [f64; 3]) -> [f64; 3] {
	std::array::from_fn(|row| (0..3).map(|k| m[row][k] * v[k]).sum())
}

/// Inverts `m` by its adjugate. The matrices here are fixed, well-conditioned color transforms.
fn invert(m: &Matrix3) -> Matrix3 {
	let cofactor = |row: usize, column: usize| {
		let (r0, r1) = ((row + 1) % 3, (row + 2) % 3);
		let (c0, c1) = ((column + 1) % 3, (column + 2) % 3);
		m[r0][c0] * m[r1][c1] - m[r0][c1] * m[r1][c0]
	};
	let determinant: f64 = (0..3).map(|column| m[0][column] * cofactor(0, column)).sum();
	// The adjugate is the transposed cofactor matrix.
	std::array::from_fn(|row| std::array::from_fn(|column| cofactor(column, row) / determinant))
}

/// Returns the CIE 1960 *uv* of the Planckian white of `temperature` kelvins, and the locus's tangent there.
///
/// The locus is Krystek's 1985 rational approximation. Its tangent comes from the quotient rule.
fn planckian(temperature: f64) -> ([f64; 2], [f64; 2]) {
	// Each coordinate is (a + b T + c T²) / (1 + d T + e T²); returns it and its derivative in T.
	let rational = |[a, b, c, d, e]: [f64; 5]| {
		let t = temperature;
		let (numerator, denominator) = (a + b * t + c * t * t, 1.0 + d * t + e * t * t);
		let derivative = ((b + 2.0 * c * t) * denominator - numerator * (d + 2.0 * e * t)) / (denominator * denominator);
		(numerator / denominator, derivative)
	};
	let (u, du) = rational([0.860117757, 1.54118254e-4, 1.28641212e-7, 8.42420235e-4, 7.08145163e-7]);
	let (v, dv) = rational([0.317398726, 4.22806245e-5, 4.20481691e-8, -2.89741816e-5, 1.61456053e-7]);
	([u, v], [du, dv])
}

/// Returns the CIE XYZ of D65, the white that equal sRGB channels show.
fn d65() -> [f64; 3] {
	apply(&SRGB_TO_XYZ, [1.0; 3])
}

/// Returns the CIE XYZ, at luminance 1, of the white the settings `temperature` and `tint` name.
///
/// It is the Planckian white moved by the *uv* offset that takes the reference temperature's white to D65, so the
/// default settings name D65 exactly, then moved `tint` along the locus's normal toward higher *v*, the green side.
/// D65 lies a little green of the locus; without the offset, every balanced white would show slightly magenta.
fn white(temperature: f64, tint: f64) -> [f64; 3] {
	let uv = |[x, y, z]: [f64; 3]| {
		let denominator = x + 15.0 * y + 3.0 * z;
		[4.0 * x / denominator, 6.0 * y / denominator]
	};
	let (reference, _) = planckian(REFERENCE_TEMPERATURE);
	let target = uv(d65());
	let ([u, v], [du, dv]) = planckian(temperature);
	let length = du.hypot(dv);
	let u = u + target[0] - reference[0] + TINT_SCALE * tint * dv / length;
	let v = v + target[1] - reference[1] - TINT_SCALE * tint * du / length;

	let denominator = 2.0 * u - 8.0 * v + 4.0;
	let (x, y) = (3.0 * u / denominator, 2.0 * v / denominator);
	[x / y, 1.0, (1.0 - x - y) / y]
}

/// Builds the linear sRGB matrix that makes light of the settings' white show as white.
///
/// It is a von Kries scale in CAT16 cone space from that white to D65, scaled after so a neutral grey
/// keeps its luminance.
fn white_balance(temperature: f64, tint: f64) -> Matrix3 {
	let source = apply(&XYZ_TO_CAT16, white(temperature, tint));
	let destination = apply(&XYZ_TO_CAT16, d65());
	let scale: Matrix3 = std::array::from_fn(|row| {
		std::array::from_fn(|column| if row == column { destination[row] / source[row] } else { 0.0 })
	});
	let cone = multiply(&invert(&XYZ_TO_CAT16), &multiply(&scale, &XYZ_TO_CAT16));
	let adaptation = multiply(&invert(&SRGB_TO_XYZ), &multiply(&cone, &SRGB_TO_XYZ));

	let grey_luminance = apply(&SRGB_TO_XYZ, apply(&adaptation, [1.0; 3]))[1];
	adaptation.map(|row| row.map(|value| value / grey_luminance))
}

/// Returns the ACEScct offsets that tint a grey toward Oklab hue `hue` degrees, at chroma [`SPLIT_CHROMA`].
///
/// It takes the Oklab color with mid grey's lightness, that chroma, and that hue to ACEScg, reads each channel's offset
/// from grey in ACEScct, and removes their luminance-weighted mean, so the tint leaves a grey's brightness in place to
/// first order. The negated offsets tint toward the opposite hue.
fn split_direction(hue: f64) -> [f64; 3] {
	let grey = 0.18f64;
	let lab_to_lms = invert(&OKLAB_LMS_TO_LAB);
	let grey_lightness = apply(&OKLAB_LMS_TO_LAB, apply(&SRGB_TO_OKLAB_LMS, [grey; 3]).map(f64::cbrt))[0];
	let (sin, cos) = hue.to_radians().sin_cos();
	let lms = apply(&lab_to_lms, [grey_lightness, SPLIT_CHROMA * cos, SPLIT_CHROMA * sin]).map(|value| value.powi(3));
	let srgb = apply(&invert(&SRGB_TO_OKLAB_LMS), lms);
	let working = apply(&SRGB_TO_ACESCG, srgb);
	let grey_working = apply(&SRGB_TO_ACESCG, [grey; 3]);

	let offsets: [f64; 3] = std::array::from_fn(|channel| (working[channel] / grey_working[channel]).log2() / ACESCCT_STOPS);
	let mean: f64 = (0..3).map(|channel| ACESCG_LUMINANCE[channel] * offsets[channel]).sum();
	offsets.map(|offset| offset - mean)
}

/// Builds the shader constants: exposure and white balance folded into the matrix into ACEScg, its way back, the tone
/// curve's settings, the split tone's strengths and direction, and the look the shader applies in between.
fn color_correction_shader_data(settings: &ColorCorrectionPassSettings) -> ColorCorrectionShaderData {
	let exposure = 2f64.powf(settings.exposure as f64);
	let balance = white_balance(settings.temperature as f64, settings.tint as f64);
	let to_working = multiply(&SRGB_TO_ACESCG, &balance).map(|row| row.map(|value| value * exposure));
	let from_working = invert(&SRGB_TO_ACESCG);
	let rows = |m: Matrix3| m.map(|row| [row[0] as f32, row[1] as f32, row[2] as f32, 0.0]);

	ColorCorrectionShaderData {
		to_working: rows(to_working),
		from_working: rows(from_working),
		look: [settings.contrast, settings.saturation, settings.density, 0.0],
		tone: [
			settings.shadows,
			settings.highlights,
			settings.split_amount * (1.0 + settings.split_balance).min(1.0),
			settings.split_amount * (1.0 - settings.split_balance).min(1.0),
		],
		split: {
			let [r, g, b] = split_direction(settings.split_hue as f64);
			[r as f32, g as f32, b as f32, 0.0]
		},
	}
}

/// Panics with a fix when `settings` holds a value the correction cannot use.
fn validate(settings: &ColorCorrectionPassSettings) {
	let unit = |value: f32| (-1.0..=1.0).contains(&value);
	let checks: [(&str, f32, bool, &str); 11] = [
		("exposure", settings.exposure, true, "any finite number of stops"),
		(
			"temperature",
			settings.temperature,
			TEMPERATURE_RANGE.contains(&settings.temperature),
			"1,000 to 15,000 kelvins",
		),
		("tint", settings.tint, unit(settings.tint), "-1 to 1"),
		("shadows", settings.shadows, unit(settings.shadows), "-1 to 1"),
		("highlights", settings.highlights, unit(settings.highlights), "-1 to 1"),
		("split_hue", settings.split_hue, true, "any finite number of degrees"),
		(
			"split_amount",
			settings.split_amount,
			(0.0..=1.0).contains(&settings.split_amount),
			"0 to 1",
		),
		(
			"split_balance",
			settings.split_balance,
			unit(settings.split_balance),
			"-1 to 1",
		),
		(
			"contrast",
			settings.contrast,
			settings.contrast > 0.0,
			"a value above 0, with 1 for no change",
		),
		(
			"saturation",
			settings.saturation,
			settings.saturation >= 0.0,
			"0 or above, with 1 for no change",
		),
		("density", settings.density, (0.0..=1.0).contains(&settings.density), "0 to 1"),
	];
	for (name, value, valid, range) in checks {
		assert!(
			value.is_finite() && valid,
			"Invalid color correction {name} {value}. The most likely cause is a setting outside the range the correction defines. Set ColorCorrectionPassSettings::{name} to {range}."
		);
	}
}

/// The `ColorCorrectionPass` struct lets an application set a scene's exposure, white balance, tone curve, split tone,
/// and saturation before tone mapping, so a LUT or tone mapper receives a balanced picture with the look it needs.
///
/// Install it with [`crate::application::graphics::setup_color_correction_render_pass`] after every pass that writes
/// scene light, such as bloom, and before a LUT or tone mapper. Toggle it at runtime with `render.pass.color-correction`.
pub struct ColorCorrectionPass {
	pass: simple_tile::Pass,
	_parameters: ghi::BufferHandle<ColorCorrectionShaderData>,
}

impl Entity for ColorCorrectionPass {}

impl ColorCorrectionPass {
	/// Creates the correction with `settings` and remaps `main` to its scene-linear output for later passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, settings: ColorCorrectionPassSettings) -> Self {
		validate(&settings);

		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("Color Correction"));

		let pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Color Correction",
				"byte_engine/rendering/color-correction/correct.pipeline",
				&[output.format()],
			),
		);

		let context = render_pass_builder.context();
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let parameters = context.build_buffer::<ColorCorrectionShaderData>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Color Correction Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		*context.get_mut_buffer_slice(parameters) = color_correction_shader_data(&settings);

		let pass = pipeline.bind(
			"Color Correction",
			&[
				simple_tile::Resource::combined_image_sampler("source_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::buffer("parameters", parameters),
			],
			&[simple_tile::Output::new("result_texture", output)],
		);

		Self {
			pass,
			_parameters: parameters,
		}
	}
}

impl RenderPass for ColorCorrectionPass {
	fn name(&self) -> &'static str {
		"color-correction"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		self.pass.prepare(frame, sink, frame_allocator)
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Value, output_slot};

	use super::*;
	use crate::rendering::{
		render_pass::simple_compute,
		shader_vm_test::{PIXEL_TILE, asset_source, binding_slot, buffer, compile, empty_image, rgba, run_pixel, texture_2d},
	};

	/// Runs the correction shader on one pixel of `color` with `settings`, as `ColorCorrectionPass` records it.
	fn correct(settings: ColorCorrectionPassSettings, color: [f32; 3]) -> [f32; 3] {
		validate(&settings);
		let linked = simple_compute::compile_test_program(asset_source("color-correction/correct.besl"));
		let slot = binding_slot(&linked, "parameters");
		let source_slot = binding_slot(&linked, "source_texture");
		let program = compile(linked);
		let data = color_correction_shader_data(&settings);
		let mut parameters = buffer(&program, slot);
		let fields = [
			("to_working_x", data.to_working[0]),
			("to_working_y", data.to_working[1]),
			("to_working_z", data.to_working[2]),
			("from_working_x", data.from_working[0]),
			("from_working_y", data.from_working[1]),
			("from_working_z", data.from_working[2]),
			("look", data.look),
			("tone", data.tone),
			("split", data.split),
		];
		for (name, value) in fields {
			parameters.write(name, Value::Vec4F(value)).expect(
				"Failed to initialize color correction parameters. The most likely cause is a changed production buffer layout.",
			);
		}

		let mut source = texture_2d(1, 1, &[[color[0], color[1], color[2], 0.5]]);
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(source_slot, &mut source);
		descriptors.bind_buffer(slot, &mut parameters);
		descriptors.bind_image(output_slot(0), &mut result);
		run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
		drop(descriptors);
		let [r, g, b, a] = rgba(&result, [0, 0]);
		assert_eq!(a, 0.5, "Color correction changed alpha.");
		[r, g, b]
	}

	fn luminance([r, g, b]: [f32; 3]) -> f32 {
		0.2126729 * r + 0.7151522 * g + 0.0721750 * b
	}

	fn assert_close(actual: [f32; 3], expected: [f32; 3], tolerance: f32, case: &str) {
		assert!(
			actual
				.iter()
				.zip(expected)
				.all(|(a, e)| (a - e).abs() <= tolerance * e.abs().max(1.0)),
			"{case}: expected {expected:?}, got {actual:?}."
		);
	}

	const COLORS: [[f32; 3]; 5] = [
		[0.18; 3],
		[0.8, 0.2, 0.05],
		[0.02, 0.3, 0.6],
		[4.0, 3.0, 2.5],
		[0.0, 0.0, 0.0],
	];

	#[test]
	fn default_settings_leave_the_picture_unchanged() {
		for color in COLORS {
			assert_close(correct(Default::default(), color), color, 1e-4, "Default correction");
		}
	}

	#[test]
	fn exposure_scales_light_by_stops() {
		let settings = ColorCorrectionPassSettings {
			exposure: 1.0,
			..Default::default()
		};
		assert_close(correct(settings, [0.18; 3]), [0.36; 3], 1e-4, "One stop up");
	}

	#[test]
	fn contrast_keeps_mid_grey_and_spreads_the_tones_around_it() {
		for contrast in [0.5, 1.5, 3.0] {
			let settings = ColorCorrectionPassSettings {
				contrast,
				..Default::default()
			};
			assert_close(correct(settings, [0.18; 3]), [0.18; 3], 1e-4, "Mid grey under contrast");
			let shadow = correct(settings, [0.02; 3])[0];
			let highlight = correct(settings, [2.0; 3])[0];
			assert_eq!(
				shadow < 0.02 && highlight > 2.0,
				contrast > 1.0,
				"Contrast {contrast} moved a shadow to {shadow} and a highlight to {highlight}."
			);
			assert!(shadow > 0.0, "Contrast {contrast} clipped a shadow to black.");
		}
	}

	#[test]
	fn saturation_and_density_never_tint_grey() {
		for (saturation, density) in [(0.0, 0.0), (2.0, 0.0), (0.0, 1.0), (2.0, 1.0), (1.5, 0.5)] {
			let settings = ColorCorrectionPassSettings {
				saturation,
				density,
				contrast: 1.3,
				..Default::default()
			};
			for grey in [0.01, 0.18, 5.0] {
				let [r, g, b] = correct(settings, [grey; 3]);
				assert!(
					(r - g).abs() <= 1e-4 * r && (g - b).abs() <= 1e-4 * g,
					"Saturation {saturation} and density {density} turned grey {grey} into {:?}.",
					[r, g, b]
				);
			}
		}
	}

	#[test]
	fn dense_saturation_darkens_and_additive_saturation_brightens() {
		let color = [0.6, 0.25, 0.1];
		let at = |density| {
			correct(
				ColorCorrectionPassSettings {
					saturation: 1.6,
					density,
					..Default::default()
				},
				color,
			)
		};
		let (dense, additive) = (at(1.0), at(0.0));
		assert!(
			luminance(dense) < luminance(color) && luminance(additive) > luminance(dense),
			"Saturation at density 1 gave luminance {} and at density 0 gave {}, from {}. The most likely cause is a pivot that no longer moves to the largest channel.",
			luminance(dense),
			luminance(additive),
			luminance(color)
		);
		assert!(
			dense[2] < color[2] && dense[1] < color[1],
			"Dense saturation raised a minor channel: {dense:?}."
		);
	}

	#[test]
	fn white_balance_neutralizes_its_light_and_keeps_grey_luminance() {
		let warm = ColorCorrectionPassSettings {
			temperature: 3200.0,
			..Default::default()
		};
		// Light of the set temperature, reflected by a white card, shows as neutral.
		let card = apply(&invert(&SRGB_TO_XYZ), white(3200.0, 0.0)).map(|channel| channel as f32 * 0.5);
		let [r, g, b] = correct(warm, card);
		assert!(
			(r - g).abs() < 2e-3 * g && (b - g).abs() < 2e-3 * g,
			"A 3,200 K card showed as {:?}.",
			[r, g, b]
		);

		let grey = correct(warm, [0.18; 3]);
		assert!(grey[2] > grey[0], "A 3,200 K balance did not turn grey bluer: {grey:?}.");
		assert!(
			(luminance(grey) - 0.18).abs() < 1e-3,
			"White balance changed grey's luminance to {}.",
			luminance(grey)
		);

		let magenta = correct(
			ColorCorrectionPassSettings {
				tint: 0.5,
				..Default::default()
			},
			[0.18; 3],
		);
		assert!(
			magenta[1] < magenta[0] && magenta[1] < magenta[2],
			"A positive tint did not turn grey magenta: {magenta:?}."
		);
	}

	fn stops(from: f32, to: f32) -> f32 {
		(to / from).log2()
	}

	#[test]
	fn shadows_move_the_shadows_and_keep_black_and_mid_grey() {
		for (shadows, expected) in [(1.0, 1.67), (-1.0, -1.21)] {
			let settings = ColorCorrectionPassSettings {
				shadows,
				..Default::default()
			};
			let moved = stops(0.008, correct(settings, [0.008; 3])[1]);
			assert!(
				(moved - expected).abs() < 0.05,
				"Shadows {shadows} moved a tone 4.5 stops below mid grey by {moved} stops, not {expected}."
			);
			assert_eq!(correct(settings, [0.0; 3]), [0.0; 3], "Shadows {shadows} moved black.");
			assert_close(correct(settings, [0.18; 3]), [0.18; 3], 1e-4, "Mid grey under shadows");
			assert_close(correct(settings, [2.0; 3]), [2.0; 3], 1e-4, "A highlight under shadows");
		}
	}

	#[test]
	fn highlights_move_the_highlights_and_keep_mid_grey_and_the_brightest_tones() {
		let peak = 0.18 * 2f32.powf(4.25);
		for (highlights, expected) in [(1.0, 1.59), (-1.0, -1.59)] {
			let settings = ColorCorrectionPassSettings {
				highlights,
				..Default::default()
			};
			let moved = stops(peak, correct(settings, [peak; 3])[1]);
			assert!(
				(moved - expected).abs() < 0.05,
				"Highlights {highlights} moved scene {peak} by {moved} stops, not {expected}."
			);
			assert_close(correct(settings, [0.18; 3]), [0.18; 3], 1e-4, "Mid grey under highlights");
			assert_close(correct(settings, [100.0; 3]), [100.0; 3], 1e-4, "Scene 100 under highlights");
		}
	}

	#[test]
	fn tone_controls_keep_a_ramp_in_order() {
		for (shadows, highlights, contrast) in [(1.0, 1.0, 1.5), (-1.0, -1.0, 0.7), (1.0, -1.0, 1.0), (-1.0, 1.0, 2.0)] {
			let settings = ColorCorrectionPassSettings {
				shadows,
				highlights,
				contrast,
				..Default::default()
			};
			let ramp: Vec<f32> = (0..=64)
				.map(|step| correct(settings, [0.0005 * 2f32.powf(step as f32 * 0.25); 3])[1])
				.collect();
			assert!(
				ramp.windows(2).all(|pair| pair[1] >= pair[0]),
				"Shadows {shadows}, highlights {highlights}, and contrast {contrast} reversed a ramp: {ramp:?}."
			);
		}
	}

	#[test]
	fn split_tone_leans_shadows_and_highlights_toward_opposite_hues_and_keeps_brightness() {
		let settings = ColorCorrectionPassSettings {
			split_hue: 55.0,
			split_amount: 1.0,
			..Default::default()
		};
		// The masks peak 3 stops below and 3.25 stops above mid grey.
		let shadow = 0.18 * 2f32.powf(-2.98);
		let highlight = 0.18 * 2f32.powf(3.24);
		let teal = correct(settings, [shadow; 3]);
		let orange = correct(settings, [highlight; 3]);
		assert!(
			teal[2] > teal[0] && teal[1] > teal[0],
			"Orange split tone did not turn a shadow teal: {teal:?}."
		);
		assert!(
			orange[0] > orange[1] && orange[1] > orange[2],
			"Orange split tone did not turn a highlight orange: {orange:?}."
		);
		for (tinted, grey) in [(teal, shadow), (orange, highlight)] {
			let change = stops(grey, luminance(tinted));
			assert!(
				change.abs() < 0.05,
				"The split tone changed a grey's brightness by {change} stops: {tinted:?}."
			);
		}
		for grey in [0.0, 0.18, 64.0] {
			assert_close(
				correct(settings, [grey; 3]),
				[grey; 3],
				1e-4,
				"Split tone at black, mid grey, or 64",
			);
		}
	}

	#[test]
	fn split_tone_spares_saturated_colors_and_follows_its_balance() {
		let tinted = |balance, color: [f32; 3]| {
			let settings = ColorCorrectionPassSettings {
				split_hue: 55.0,
				split_amount: 1.0,
				split_balance: balance,
				..Default::default()
			};
			let plain = correct(Default::default(), color);
			let toned = correct(settings, color);
			// The tint's size, as the change in the red-to-blue ratio, in stops.
			stops(plain[0] / plain[2], toned[0] / toned[2]).abs()
		};
		let highlight = 0.18 * 2f32.powf(3.24);
		let grey = tinted(0.0, [highlight; 3]);
		let saturated = tinted(0.0, [highlight * 3.0, highlight, highlight * 0.3]);
		assert!(
			saturated < 0.5 * grey,
			"A saturated color took a tint of {saturated} stops against a grey's {grey}. The most likely cause is split protection that no longer reads the channel spread."
		);
		assert!(
			tinted(-1.0, [highlight; 3]) < 1e-4,
			"A balance of -1 still tinted the highlights."
		);
		assert!(
			tinted(1.0, [0.18 * 2f32.powf(-2.98); 3]) < 1e-4,
			"A balance of 1 still tinted the shadows."
		);
	}
}
