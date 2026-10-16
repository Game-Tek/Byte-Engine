//! Camera motion blur: smears the HDR scene along the camera's own turn and zoom, as a shutter open for part of the
//! frame would.
//!
//! The pass derives each pixel's velocity from the current and previous frame's view and projection matrices alone,
//! as if every pixel were infinitely far away, so it needs no depth and no per-surface velocities. Camera translation
//! and moving objects do not blur; they need a velocity target written by the scene pipeline, which can later replace
//! the matrix-derived velocity in the same blur. See [`MotionBlurPass`].

use std::{cell::Cell, rc::Rc};

use ghi::{context::ContextCreate as _, frame::Frame as _};
use math::{Matrix, inverse};
use maths_rs::Vec4f;
use utils::Extent;

use crate::{
	configuration::{ConfigurationPort, ConfigurationValue},
	core::Entity,
	rendering::{
		Sink, View,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute},
		render_passes::blit::ImageBypassPass,
	},
	time::MediaTime,
};

/// Configuration namespace of the runtime motion blur controls; see [`MotionBlurPassSettings::with_parameter`].
/// The most taps one pixel gathers; the blur shader loops to this many.
const MAX_TAPS: u32 = 32;

/// The `MotionBlurPassSettings` struct defines how long the shutter stays open and how far the blur may reach.
///
/// Pass it to [`crate::application::graphics::setup_motion_blur_render_pass`] or [`MotionBlurControls::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlurPassSettings {
	/// How long the shutter stays open each frame. The blur covers that fraction of the camera's motion since the
	/// previous frame, so at a steady frame rate it is the shutter angle: `1/120` s is a 180° shutter at 60 Hz. A
	/// shutter longer than the frame covers the whole motion.
	pub shutter: MediaTime,
	/// The longest blur, in pixels. A fast turn smears no further, which bounds the cost of every pixel's taps.
	pub max_blur_radius: f32,
	/// The most taps averaged along the blur, from `1` to `32`; a blur gathers one tap per pixel of its length up to
	/// this. More taps smooth longer blurs; fewer are cheaper. The `render.motion-blur.taps` parameter changes it at
	/// runtime.
	pub taps: u32,
}

impl Default for MotionBlurPassSettings {
	fn default() -> Self {
		Self {
			shutter: MediaTime::from_micros(8_333),
			max_blur_radius: 32.0,
			taps: 8,
		}
	}
}

impl MotionBlurPassSettings {
	/// Applies one runtime parameter, named inside the controls' configuration port, returning the updated
	/// settings and the effective value, or leaves them unchanged. `taps` takes a whole number from `1` to `32`.
	pub(crate) fn with_parameter(
		self,
		parameter: &str,
		value: &ConfigurationValue,
	) -> Result<(Self, ConfigurationValue), String> {
		match parameter {
			"taps" => {
				let taps = value.to_u32().filter(|taps| (1..=MAX_TAPS).contains(taps)).ok_or(
					"Motion blur taps were not set. The most likely cause is that the value is not a whole number from 1 to 32.",
				)?;
				Ok((Self { taps, ..self }, ConfigurationValue::Integer(i64::from(taps))))
			}
			_ => Err(
				"Motion blur parameter was not set. The most likely cause is that the parameter name is unsupported."
					.to_string(),
			),
		}
	}
}

/// The `MotionBlurControls` struct shares one set of settings between every sink's [`MotionBlurPass`], so a
/// runtime parameter changes the blur of every window at once.
///
/// Create it with [`Self::new`] from a [`ConfigurationPort`] registered for the prefix the application gives
/// motion blur, and hand the same `Rc` to each pass.
pub struct MotionBlurControls {
	settings: Cell<MotionBlurPassSettings>,
	configuration: ConfigurationPort,
}

impl MotionBlurControls {
	/// Creates controls that start at `settings` and apply the runtime updates queued on `configuration`.
	pub fn new(settings: MotionBlurPassSettings, configuration: ConfigurationPort) -> Rc<Self> {
		Rc::new(Self {
			settings: Cell::new(settings),
			configuration,
		})
	}

	/// Applies the queued runtime updates and returns the settings every pass draws with this frame.
	pub fn settings(&self) -> MotionBlurPassSettings {
		let mut settings = self.settings.get();
		self.configuration
			.drain(&mut settings, MotionBlurPassSettings::with_parameter);
		self.settings.set(settings);
		settings
	}
}

/// The `MotionBlurShaderData` struct mirrors the `MotionBlurParameters` layout of `motion-blur.besl`.
///
/// Everything the same for every pixel is computed here so the shader does not repeat it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurShaderData {
	/// Maps a pixel's integer coordinate to the previous frame's pixel coordinates, scaled by the shutter's fraction
	/// of the frame; see [`pixel_to_previous_pixel`].
	pixel_to_previous_pixel: ghi::pod::Mat4f,
	/// One texel in UV, the shutter's fraction of the frame, and the blur radius in pixels.
	texel: [f32; 4],
	/// Shifts the shader's interleaved gradient noise each frame.
	noise_offset: f32,
	/// The most taps one pixel gathers.
	taps: u32,
	_padding: [u32; 2],
}

/// The `MotionBlurPass` struct creates a reusable camera motion blur stage for the HDR scene.
///
/// Install it after the passes that write scene light and the fog, and before lens flare, bloom, and tone mapping,
/// so light glows over the blurred image. Toggle it at runtime with the `render.pass.motion-blur` parameter and
/// change its taps with `render.motion-blur.taps`. The pass keeps the view it last drew for each sink, so the
/// first frame after it starts, after a bypass, or after a resize passes the scene through.
pub struct MotionBlurPass {
	controls: Rc<MotionBlurControls>,
	parameters: ghi::DynamicBufferHandle<MotionBlurShaderData>,
	blur_pass: simple_compute::Pass,
	/// Forwards `main` unchanged while there is nothing to blur from or the blur shader is still compiling.
	main_copy: ImageBypassPass,
	/// The view and extent of the frame drawn before this one, which the next frame's velocities start from.
	previous: Option<(View, Extent)>,
	/// Counts the frames drawn, which cycles the sample jitter so it does not sit still on a still image.
	frame: u32,
}

impl Entity for MotionBlurPass {}

impl MotionBlurPass {
	/// Creates a motion blur pass drawing with `controls`, shared with the passes of the other sinks, and remaps
	/// `main` for downstream passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, controls: Rc<MotionBlurControls>) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Storage | ghi::Uses::Image).name("Motion Blur"),
		);
		// The builder's copy of the incoming `main` into `output` doubles as this pass's forwarding copy.
		let main_copy = render_pass_builder.take_main_copy().expect(
			"Main copy is missing. The most likely cause is that the pass did not call `RenderPassBuilder::create_main_render_target` first.",
		);

		let pipeline = simple_compute::Pipeline::compile(
			render_pass_builder,
			simple_compute::Descriptor::new("Motion Blur", "byte-engine/rendering/motion-blur.pipeline"),
		);
		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Motion Blur Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// The taps land between texels, so the sampler's linear filter blends them; its clamp keeps a blur that
		// leaves the image on the edge texels.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let blur_pass = pipeline.bind(
			"Motion Blur Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
				simple_compute::Resource::image("result", output),
				simple_compute::Resource::buffer("parameters", parameters),
			],
		);

		Self {
			controls,
			parameters,
			blur_pass,
			main_copy,
			previous: None,
			frame: 0,
		}
	}

	/// Remembers this frame's view as the one the next frame moves from, and returns the view this frame moves
	/// from when it was drawn at the same extent.
	fn take_previous_view(&mut self, sink: &Sink) -> Option<View> {
		let previous = self.previous.replace((sink.view(), sink.extent()));
		previous.and_then(|(view, extent)| (extent == sink.extent()).then_some(view))
	}
}

impl RenderPass for MotionBlurPass {
	fn name(&self) -> &'static str {
		"motion-blur"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let settings = self.controls.settings();
		let previous = self.take_previous_view(sink);
		self.frame = self.frame.wrapping_add(1);
		let fraction = shutter_fraction(settings.shutter, sink.frame_interval());
		// Without a frame to move from, or a shutter that is open, there is nothing to blur.
		let Some(previous) = previous.filter(|_| fraction > 0.0) else {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		let Some(command) = self.blur_pass.prepare(frame, sink, frame_allocator) else {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		*frame.get_mut_dynamic_buffer_slice(self.parameters) =
			motion_blur_shader_data(&settings, previous, sink, fraction, self.frame);
		Some(command)
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// A bypassed frame still becomes the one the next frame moves from, so re-enabling never blurs from a
		// view several frames old, and its parameters are applied so they report their result. This pass took
		// the builder's forwarding copy, so it forwards `main` itself.
		self.controls.settings();
		self.take_previous_view(sink);
		self.main_copy.prepare(frame, sink, frame_allocator)
	}
}

/// Returns the fraction of the motion since the previous frame a shutter open for `shutter` covers, from `0` for a
/// closed shutter or an unknown frame interval to `1` for a shutter open the whole frame or longer.
fn shutter_fraction(shutter: MediaTime, frame_interval: MediaTime) -> f32 {
	let interval = frame_interval.as_seconds_f32();
	if interval > 0.0 {
		(shutter.as_seconds_f32() / interval).clamp(0.0, 1.0)
	} else {
		0.0
	}
}

/// Builds one sink's blur constants for a frame that moves from `previous` to the sink's view.
fn motion_blur_shader_data(
	settings: &MotionBlurPassSettings,
	previous: View,
	sink: &Sink,
	fraction: f32,
	frame: u32,
) -> MotionBlurShaderData {
	let extent = sink.extent();
	let taps = settings.taps.clamp(1, MAX_TAPS);
	MotionBlurShaderData {
		pixel_to_previous_pixel: pixel_to_previous_pixel(sink.view(), previous, extent, fraction).into(),
		texel: [
			1.0 / extent.width() as f32,
			1.0 / extent.height() as f32,
			fraction,
			settings.max_blur_radius.max(0.0),
		],
		// The noise shifts by this much per frame and repeats after 64 frames.
		noise_offset: 5.588_238 * (frame % 64) as f32,
		taps,
		_padding: [0; 2],
	}
}

/// Returns the matrix that takes a pixel's integer coordinate in a `current` view of `extent` to the pixel
/// coordinates the `previous` view saw its view direction at, before the division by w, scaled by `fraction`.
///
/// The pixel's direction in the current camera's axes is rotated into the previous camera's axes and projected with
/// the previous projection. Dropping w before the rotation makes the direction a point at infinity, so the cameras'
/// positions play no part: only their turn and zoom move pixels. The clip-to-pixel scale and `fraction` are folded
/// in, so the shader divides once and has the blur's share of the motion. The z row is zero: depth plays no part.
fn pixel_to_previous_pixel(current: View, previous: View, extent: Extent, fraction: f32) -> Matrix {
	// Takes current view space to previous view space; without the translation column, only the rotation.
	let mut rotation = previous.view() * inverse(current.view());
	rotation.set_column(3, Vec4f::new(0.0, 0.0, 0.0, 1.0));
	let to_infinity = Matrix::from((
		Vec4f::new(1.0, 0.0, 0.0, 0.0),
		Vec4f::new(0.0, 1.0, 0.0, 0.0),
		Vec4f::new(0.0, 0.0, 1.0, 0.0),
		Vec4f::new(0.0, 0.0, 0.0, 0.0),
	));
	// Pixel x is (ndc.x + 1) / 2 of the width, and pixel y counts down from the top.
	let (width, height) = (
		extent.width() as f32 * 0.5 * fraction,
		extent.height() as f32 * 0.5 * fraction,
	);
	let clip_to_pixel = Matrix::from((
		Vec4f::new(width, 0.0, 0.0, width),
		Vec4f::new(0.0, -height, 0.0, height),
		Vec4f::new(0.0, 0.0, 0.0, 0.0),
		Vec4f::new(0.0, 0.0, 0.0, 1.0),
	));
	clip_to_pixel * previous.projection() * rotation * to_infinity * inverse(current.projection()) * super::pixel_to_ndc(extent)
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ResourceSlot, Value};
	use math::{Degrees, Point, UnitVector, Vector};

	use super::*;
	use crate::rendering::shader_vm_test::{assert_rgba_close, buffer, empty_image, rgba, run_at, texture_2d};

	const MOTION_BLUR_BESL: &str = include_str!("../../../assets/rendering/motion-blur.besl");
	const FOV: f32 = 60.0;

	/// A camera at the origin looking along `direction` with a square 60° view.
	fn view(direction: UnitVector) -> View {
		View::new_perspective(Degrees::new(FOV), 1.0, 0.1, 100.0, Point::origin(), direction)
	}

	/// A camera turned `yaw` degrees from the z axis toward the x axis.
	fn yawed(yaw: f32) -> View {
		let (sine, cosine) = yaw.to_radians().sin_cos();
		view(
			Vector::new(sine, 0.0, cosine)
				.normalized()
				.expect("A unit direction normalizes."),
		)
	}

	/// Returns where [`pixel_to_previous_pixel`] puts the center of `pixel` of a `current` view in a previous view,
	/// as a pixel coordinate of an `extent`-sized image.
	fn previous_pixel(current: View, previous: View, extent: Extent, pixel: [u32; 2]) -> [f32; 2] {
		let previous =
			pixel_to_previous_pixel(current, previous, extent, 1.0) * Vec4f::new(pixel[0] as f32, pixel[1] as f32, 1.0, 1.0);
		assert!(previous.w > 0.0, "{previous:?}");
		[previous.x / previous.w, previous.y / previous.w]
	}

	#[test]
	fn a_still_camera_keeps_every_pixel_where_it_is() {
		let extent = Extent::rectangle(64, 48);
		let still = view(UnitVector::z_axis());
		for pixel in [[0, 0], [63, 47], [10, 30], [32, 24]] {
			let [x, y] = previous_pixel(still, still, extent, pixel);
			assert!(
				(x - (pixel[0] as f32 + 0.5)).abs() < 1e-3 && (y - (pixel[1] as f32 + 0.5)).abs() < 1e-3,
				"{pixel:?}: {x} {y}"
			);
		}
	}

	#[test]
	fn a_turn_moves_the_center_pixel_by_the_tangent_of_the_turn() {
		let extent = Extent::square(100);
		let yaw = 5.0f32;
		let [x, y] = previous_pixel(view(UnitVector::z_axis()), yawed(yaw), extent, [50, 50]);
		// The previous camera looked toward +x, so the current center lay toward −x on its screen, at the turn's
		// tangent over the half view's tangent.
		let expected = 50.5 - yaw.to_radians().tan() / (FOV / 2.0).to_radians().tan() * 50.0;
		assert!(
			(x - expected).abs() < 1e-2 && (y - 50.5).abs() < 1e-2,
			"{x} {y}, expected {expected}"
		);
	}

	#[test]
	fn a_moved_camera_keeps_every_pixel_where_it_is() {
		let extent = Extent::square(64);
		let moved = View::new_perspective(
			Degrees::new(FOV),
			1.0,
			0.1,
			100.0,
			Point::new(3.0, -2.0, 7.0),
			UnitVector::z_axis(),
		);
		let [x, y] = previous_pixel(view(UnitVector::z_axis()), moved, extent, [10, 50]);
		assert!((x - 10.5).abs() < 1e-3 && (y - 50.5).abs() < 1e-3, "{x} {y}");
	}

	#[test]
	fn taps_parameter_takes_whole_numbers_within_the_shader_limit() {
		let settings = MotionBlurPassSettings::default();
		let text = |value: &str| ConfigurationValue::Text(value.to_string());

		let (updated, effective) = settings.with_parameter("taps", &ConfigurationValue::Integer(16)).unwrap();
		assert_eq!((updated.taps, effective), (16, ConfigurationValue::Integer(16)));
		let (updated, effective) = settings.with_parameter("taps", &text("4")).unwrap();
		assert_eq!((updated.taps, effective), (4, ConfigurationValue::Integer(4)));
		for value in [
			ConfigurationValue::Integer(0),
			ConfigurationValue::Integer(33),
			ConfigurationValue::Float(2.5),
			text("many"),
		] {
			assert!(settings.with_parameter("taps", &value).is_err(), "{value:?}");
		}
		assert!(settings.with_parameter("shutter", &ConfigurationValue::Float(0.01)).is_err());
	}

	#[test]
	fn controls_apply_queued_taps_and_report_them() {
		use crate::configuration::{Configuration, ConfigurationUpdateState};

		let configuration = Configuration::new();
		let port = configuration.register("render.motion-blur.");
		let controls = MotionBlurControls::new(MotionBlurPassSettings::default(), port);
		let set = configuration.update("render.motion-blur.taps", ConfigurationValue::Integer(12));
		let rejected = configuration.update("render.motion-blur.taps", ConfigurationValue::Integer(64));

		assert_eq!(controls.settings().taps, 12);
		assert!(
			matches!(configuration.event(set).unwrap().state(), ConfigurationUpdateState::Set { value } if *value == ConfigurationValue::Integer(12))
		);
		assert!(matches!(
			configuration.event(rejected).unwrap().state(),
			ConfigurationUpdateState::NotSet { .. }
		));
	}

	#[test]
	fn shutter_fraction_follows_the_frame_interval() {
		let frame = MediaTime::from_micros(16_667);
		assert_eq!(shutter_fraction(MediaTime::from_micros(8_333), MediaTime::ZERO), 0.0);
		assert_eq!(shutter_fraction(MediaTime::ZERO, frame), 0.0);
		assert_eq!(shutter_fraction(MediaTime::from_micros(100_000), frame), 1.0);
		assert!((shutter_fraction(MediaTime::from_micros(8_333), frame) - 0.5).abs() < 1e-3);
	}

	/// Blurs `pixel` of `texels`, a `width`-wide one-row image, through the production shader for a frame that
	/// moved from `previous` to `current` with the shutter open for `fraction` of it.
	fn blur_pixel(texels: &[[f32; 4]], current: View, previous: View, fraction: f32, pixel: u32) -> [f32; 4] {
		let width = texels.len() as u32;
		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(MOTION_BLUR_BESL));
		let sink = Sink::new(current, Extent::rectangle(width, 1), 0);
		let data = motion_blur_shader_data(&MotionBlurPassSettings::default(), previous, &sink, fraction, 1);
		let parameter_slot = ResourceSlot::new(2);
		let mut parameters = buffer(&program, parameter_slot);
		for (name, value) in [
			(
				"pixel_to_previous_pixel",
				Value::Mat4F(bytemuck::cast(data.pixel_to_previous_pixel)),
			),
			("texel", Value::Vec4F(data.texel)),
			("noise_offset", Value::F32(data.noise_offset)),
			("taps", Value::U32(data.taps)),
		] {
			parameters.write(name, value).expect(
				"Failed to initialize motion blur parameters. The most likely cause is a changed production buffer layout.",
			);
		}
		let mut scene = texture_2d(width, 1, texels);
		let mut result = empty_image(width, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut scene);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		run_at(&program, &mut descriptors, [pixel, 0]);
		drop(descriptors);
		rgba(&result, [pixel, 0])
	}

	/// A row whose left half is red and right half is blue.
	fn halves() -> Vec<[f32; 4]> {
		(0..32)
			.map(|x| if x < 16 { [1.0, 0.0, 0.0, 1.0] } else { [0.0, 0.0, 1.0, 1.0] })
			.collect()
	}

	#[test]
	fn motion_blur_besl_vm_passes_a_still_camera_through() {
		let still = view(UnitVector::z_axis());
		for pixel in [0, 15, 16, 31] {
			assert_rgba_close(
				blur_pixel(&halves(), still, still, 1.0, pixel),
				halves()[pixel as usize],
				1e-6,
			);
		}
	}

	#[test]
	fn motion_blur_besl_vm_mixes_the_halves_along_a_turn() {
		let current = view(UnitVector::z_axis());
		// A 5° turn of a 60° view moves the middle of a 32-pixel row by about five pixels.
		let turned = yawed(5.0);
		for pixel in [15, 16] {
			let [red, _, blue, alpha] = blur_pixel(&halves(), current, turned, 1.0, pixel);
			assert!(
				red > 0.05 && blue > 0.05 && (red + blue - 1.0).abs() < 1e-4 && alpha == 1.0,
				"{pixel}: {red} {blue}"
			);
		}
		// Far from the edge the blur only mixes one color.
		assert_rgba_close(blur_pixel(&halves(), current, turned, 1.0, 4), [1.0, 0.0, 0.0, 1.0], 1e-4);
	}

	/// Lowers the blur program through the platform shader compiler. The VM tests prove it links and behaves as
	/// BESL; defects the backend mishandles only surface when the real platform compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn motion_blur_besl_program_lowers_to_the_platform_shader_language() {
		use resource_management::shader::{ShaderGenerationSettings, besl::backends::platform::PlatformShaderCompiler};

		let mut root = besl::parse(MOTION_BLUR_BESL).expect("motion blur shader should parse");
		root.add(vec![crate::rendering::common_shader_generator::common_shader_scope()]);
		let root = besl::lex(root).expect("motion blur shader should link");
		let settings = ShaderGenerationSettings::compute(Extent::rectangle(8, 8)).name("motion_blur".to_string());

		PlatformShaderCompiler::new()
			.generate(&settings, &root)
			.await
			.unwrap_or_else(|error| panic!("motion blur should compile for the platform shader language: {error}"));
	}

	#[test]
	fn motion_blur_besl_vm_keeps_the_image_sharp_with_the_shutter_closed() {
		let current = view(UnitVector::z_axis());
		for pixel in [15, 16] {
			assert_rgba_close(
				blur_pixel(&halves(), current, yawed(5.0), 0.0, pixel),
				halves()[pixel as usize],
				1e-6,
			);
		}
	}
}
