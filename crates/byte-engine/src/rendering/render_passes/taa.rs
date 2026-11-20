//! Temporal antialiasing: resolves detail finer than a pixel by blending jittered frames, each reprojected through the
//! scene's velocity, while rejecting history that no longer matches the scene so moving objects leave no trails.
//!
//! [`TaaPass`] asks its sink for [jitter](crate::rendering::Sink::with_jitter), which makes the visibility pipeline
//! write the `Velocity` and `Reactive` targets the pass reads. Install it with
//! [`crate::application::graphics::setup_taa_render_pass`] in place of SMAA. See the temporal antialiasing design note.

use std::{cell::Cell, rc::Rc};

use ghi::{context::ContextCreate as _, frame::Frame as _};
use utils::Extent;

use crate::{
	configuration::{Accepts, ConfigurationPort, Read, Setting},
	rendering::{
		DebugViews, ShownDebugViews, Sink,
		debug_views::DEBUG_VIEW_TARGETS_BINDING,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_tile},
	},
};

/// The `TaaPassSettings` struct defines how strongly the history weighs against each new frame.
///
/// Pass it to [`crate::application::graphics::setup_taa_render_pass`]. The `render.taa.` parameters change it at
/// runtime.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TaaPassSettings {
	/// The history's least weight, which a moving pixel gets when its color changed, and any pixel when it moves fast.
	/// Lower follows change faster and leaves less trail, and shows more of each frame's aliasing.
	pub feedback_min: f32,
	/// The history's greatest weight, which a pixel gets while it holds still. Higher is smoother and slower to follow
	/// change.
	pub feedback_max: f32,
	/// How many standard deviations of the 3 × 3 neighborhood the history may stray from its mean before it's clipped.
	/// Lower rejects stale history sooner, and lets more flicker through.
	pub variance_gamma: f32,
}

impl Default for TaaPassSettings {
	/// Returns INSIDE's feedback range and Salvi's variance gamma.
	fn default() -> Self {
		Self {
			feedback_min: 0.88,
			feedback_max: 0.97,
			variance_gamma: 1.0,
		}
	}
}

impl TaaPassSettings {
	/// The runtime settings, named inside the controls' configuration port.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[
		Setting {
			name: "feedback-min",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.feedback_min)),
			apply: |settings, read| {
				let feedback_min = read.number() as f32;
				if feedback_min > settings.feedback_max {
					return Err(format!(
						"The feedback minimum {feedback_min} is above the maximum {}. The most likely cause is a minimum set before lowering the maximum. Set `render.taa.feedback-max` first, or pick a minimum of at most {}.",
						settings.feedback_max, settings.feedback_max
					));
				}
				Ok(Self { feedback_min, ..settings })
			},
		},
		Setting {
			name: "feedback-max",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.feedback_max)),
			apply: |settings, read| {
				let feedback_max = read.number() as f32;
				if feedback_max > 1.0 || feedback_max < settings.feedback_min {
					return Err(format!(
						"The feedback maximum {feedback_max} is outside {} through 1. The most likely cause is a maximum below the current minimum or above 1. Pick a maximum from {} through 1.",
						settings.feedback_min, settings.feedback_min
					));
				}
				Ok(Self { feedback_max, ..settings })
			},
		},
		Setting {
			name: "variance-gamma",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.variance_gamma)),
			apply: |settings, read| {
				let variance_gamma = read.number() as f32;
				if variance_gamma <= 0.0 {
					return Err(
						"The variance gamma 0 clips every history to the neighborhood's mean. The most likely cause is a typo. Pick a gamma above 0, such as 1."
							.to_string(),
					);
				}
				Ok(Self { variance_gamma, ..settings })
			},
		},
	];
}

/// The `TaaControls` struct shares one set of settings between every sink's [`TaaPass`], so a runtime parameter
/// changes the antialiasing of every window at once.
///
/// Create it with [`Self::new`] from a [`ConfigurationPort`] registered for `render.taa.`, and hand the same `Rc` to
/// each pass.
pub struct TaaControls {
	settings: Cell<TaaPassSettings>,
	configuration: ConfigurationPort,
}

impl TaaControls {
	/// Creates controls that start at `settings` and apply the runtime updates queued on `configuration`.
	pub fn new(settings: TaaPassSettings, configuration: ConfigurationPort) -> Rc<Self> {
		configuration.declare(TaaPassSettings::SETTINGS.iter().map(|setting| setting.parameter(settings)));
		Rc::new(Self {
			settings: Cell::new(settings),
			configuration,
		})
	}

	/// Applies the queued runtime updates and returns the settings every pass resolves with this frame.
	pub fn settings(&self) -> TaaPassSettings {
		let mut settings = self.settings.get();
		self.configuration.drain(&mut settings, TaaPassSettings::SETTINGS);
		self.settings.set(settings);
		settings
	}
}

/// The `TaaShaderData` struct mirrors the `TaaParameters` layout of `taa/resolve.besl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct TaaShaderData {
	pub(crate) feedback_min: f32,
	pub(crate) feedback_max: f32,
	pub(crate) variance_gamma: f32,
	pub(crate) exposure_ratio: f32,
	pub(crate) reset: u32,
	pub(crate) debug_view_first_slot: u32,
	/// The shader's `bool` members, one bit per view of [`TAA_DEBUG_VIEWS`].
	pub(crate) debug_views: u8,
	_padding: [u8; 3],
}

/// The debug views the pass declares, in the order `taa/resolve.besl` numbers them from `0`.
const TAA_DEBUG_VIEWS: [&str; 4] = ["velocity", "disocclusion", "reactive", "taa-frames"];

/// The format of the history, which keeps the blend's precision across frames and the frame count in alpha.
const HISTORY_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;

/// The `TaaPass` struct resolves a jittered sink's frames into one antialiased image with as little ghosting as the
/// scene's velocity allows.
pub struct TaaPass {
	controls: Rc<TaaControls>,
	parameters: ghi::BufferHandle<TaaShaderData>,
	resolve_pass: simple_tile::Pass,
	debug_views: DebugViews,
	/// The extent and exposure of the frame this pass resolved last, or `None` when it didn't resolve the frame before
	/// this one, whose history then holds nothing usable.
	previous: Option<(Extent, f32)>,
}

impl TaaPass {
	/// Creates a TAA pass for one sink, resolving with `controls`, shared with the passes of the other sinks, and
	/// remaps `main` for downstream passes.
	///
	/// # Panics
	///
	/// Panics when the sink has no `Velocity` target, because its scene pipeline isn't the visibility pipeline.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, controls: Rc<TaaControls>) -> Self {
		let source = render_pass_builder.read_from("main");
		let velocity = render_pass_builder.read_from("Velocity");
		let reactive = render_pass_builder.read_from("Reactive");
		let main_format = render_pass_builder.format_of("main");
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("TAA Output"));
		let output_uses = simple_tile::output_uses(&*render_pass_builder.context());
		let history = render_pass_builder.create_history_target(
			ghi::image::Builder::new(HISTORY_FORMAT, ghi::Uses::Image | output_uses).name("TAA History"),
			1,
		);

		let pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"TAA Resolve",
				"byte_engine/rendering/taa/resolve.pipeline",
				&[output.format(), HISTORY_FORMAT],
			),
		);
		let debug_views = render_pass_builder.debug_views(&TAA_DEBUG_VIEWS);
		let context = render_pass_builder.context();
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("TAA Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		// The history's Catmull-Rom taps land between texels and blend through the linear filter; every other input is
		// fetched texel by texel.
		let linear = context.build_sampler(ghi::sampler::Builder::new());
		let nearest = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		let read = ghi::Layouts::Read;
		let resolve_pass = pipeline.bind(
			"TAA Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("scene_texture", source, nearest, read),
				simple_tile::Resource::combined_image_sampler("velocity", velocity, nearest, read),
				simple_tile::Resource::previous_frame("previous_velocity", velocity, nearest),
				simple_tile::Resource::combined_image_sampler("reactive", reactive, nearest, read),
				simple_tile::Resource::previous_frame("history", history, linear),
				simple_tile::Resource::buffer("parameters", parameters),
				simple_tile::Resource::image_array(DEBUG_VIEW_TARGETS_BINDING, debug_views.targets()),
			],
			&[
				simple_tile::Output::new("result", output),
				simple_tile::Output::new("history_output", ghi::BaseImageHandle::from(history)),
			],
		);

		Self {
			controls,
			parameters,
			resolve_pass,
			debug_views,
			previous: None,
		}
	}
}

impl RenderPass for TaaPass {
	fn name(&self) -> &'static str {
		"taa"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let settings = self.controls.settings();
		let previous = self.previous.take();
		let command = self.resolve_pass.prepare(frame, sink, frame_allocator)?;
		*frame.map(self.parameters) = taa_shader_data(
			&settings,
			sink,
			previous,
			self.debug_views.shown(0..TAA_DEBUG_VIEWS.len()),
		);
		self.previous = Some((sink.extent(), sink.exposure_scale()));
		Some(command)
	}

	fn bypass<'a>(
		&mut self,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// A bypassed frame writes no history, so the frame after it starts the history over. The renderer forwards
		// `main` on its own.
		self.controls.settings();
		self.previous = None;
		None
	}

	fn jitters_view(&self) -> bool {
		true
	}
}

/// Builds one sink's resolve constants from `settings`, where `previous` holds the extent and exposure of the frame
/// this pass resolved last, or is `None` when its history holds nothing usable.
fn taa_shader_data(
	settings: &TaaPassSettings,
	sink: &Sink,
	previous: Option<(Extent, f32)>,
	shown: ShownDebugViews,
) -> TaaShaderData {
	let history = previous.filter(|(extent, _)| *extent == sink.extent());
	TaaShaderData {
		feedback_min: settings.feedback_min,
		feedback_max: settings.feedback_max,
		variance_gamma: settings.variance_gamma,
		exposure_ratio: history.map_or(1.0, |(_, exposure)| sink.exposure_scale() / exposure),
		reset: history.is_none() as u32,
		debug_view_first_slot: shown.first_slot,
		debug_views: shown.views,
		_padding: [0; 3],
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, Value, output_slot};
	use math::{Degrees, Point, UnitVector};

	use super::*;
	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{PIXEL_TILE, buffer, compile, empty_image, rgba, run_pixel, slot, texture_2d};
	use crate::rendering::View;

	const RESOLVE_BESL: &str = include_str!("../../../assets/rendering/taa/resolve.besl");
	const SIDE: u32 = 8;
	const PIXEL: [u32; 2] = [4, 4];
	/// The depth of the surface every fixture pixel shows, this frame and the previous one.
	const SURFACE_Z: f32 = 2.0;

	/// The shader declares one `bool` member per debug view, consecutive and in the order the Rust list declares them.
	#[test]
	fn debug_views_are_declared_in_order() {
		let members = ["velocity", "disocclusion", "reactive", "frames"]
			.iter()
			.map(|view| format!("\tdebug_view_{view}: bool,\n"))
			.collect::<String>();
		assert!(RESOLVE_BESL.contains(&members), "{members}");
		assert_eq!(TAA_DEBUG_VIEWS.len(), 4);
	}

	/// The `Fixture` struct holds one resolve's inputs over a square image: the current frame, given per pixel, and
	/// uniform velocity, depths, reactive mask, and history.
	struct Fixture {
		current: Vec<[f32; 4]>,
		/// The motion in pixels and the surface's previous linear depth.
		velocity: [f32; 3],
		reactive: f32,
		/// The previous frame's linear depth at every pixel.
		previous_z: f32,
		/// The history's color and frame count.
		history: ([f32; 3], u32),
		reset: bool,
	}

	impl Fixture {
		/// A still surface whose current frame is `color` at every pixel, over a history of `history` blended for
		/// `frames` frames.
		fn still(color: [f32; 3], history: [f32; 3], frames: u32) -> Self {
			Self {
				current: vec![[color[0], color[1], color[2], 1.0]; (SIDE * SIDE) as usize],
				velocity: [0.0, 0.0, SURFACE_Z],
				reactive: 0.0,
				previous_z: SURFACE_Z,
				history: (history, frames),
				reset: false,
			}
		}

		/// Makes the current frame a checkerboard of `dark` and `light`, with `light` at [`PIXEL`].
		fn checkered(mut self, dark: [f32; 3], light: [f32; 3]) -> Self {
			for (index, texel) in self.current.iter_mut().enumerate() {
				let (x, y) = (index as u32 % SIDE, index as u32 / SIDE);
				let color = if (x + y) % 2 == 0 { light } else { dark };
				*texel = [color[0], color[1], color[2], 1.0];
			}
			self
		}

		/// Runs the resolve at [`PIXEL`] and returns the result and the history it writes.
		fn run(&self) -> ([f32; 4], [f32; 4]) {
			let program = compile(simple_compute::compile_test_program(RESOLVE_BESL));
			let view = View::new_perspective(Degrees::new(60.0), 1.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
			let sink = Sink::new(view, Extent::rectangle(SIDE, SIDE), 0).with_exposure_scale(1.0);
			let previous = (!self.reset).then_some((sink.extent(), 1.0));
			let data = taa_shader_data(&TaaPassSettings::default(), &sink, previous, ShownDebugViews::default());
			let mut parameters = buffer(&program, slot(&program, "parameters"));
			for (name, value) in [
				("feedback_min", Value::F32(data.feedback_min)),
				("feedback_max", Value::F32(data.feedback_max)),
				("variance_gamma", Value::F32(data.variance_gamma)),
				("exposure_ratio", Value::F32(data.exposure_ratio)),
				("reset", Value::U32(data.reset)),
			] {
				parameters.write(name, value).expect("TAA parameters");
			}
			let uniform = |texel: [f32; 4]| texture_2d(SIDE, SIDE, &vec![texel; (SIDE * SIDE) as usize]);
			let mut scene = texture_2d(SIDE, SIDE, &self.current);
			let [vx, vy, vz] = self.velocity;
			let mut velocity = uniform([vx, vy, vz, SURFACE_Z]);
			let mut previous_velocity = uniform([0.0, 0.0, 0.0, self.previous_z]);
			let mut reactive = uniform([self.reactive, 0.0, 0.0, 0.0]);
			let ([hr, hg, hb], frames) = self.history;
			let mut history = uniform([hr, hg, hb, frames as f32 / 255.0]);
			let mut result = empty_image(SIDE, SIDE);
			let mut history_output = empty_image(SIDE, SIDE);
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_texture(slot(&program, "scene_texture"), &mut scene);
			descriptors.bind_texture(slot(&program, "velocity"), &mut velocity);
			descriptors.bind_texture(slot(&program, "previous_velocity"), &mut previous_velocity);
			descriptors.bind_texture(slot(&program, "reactive"), &mut reactive);
			descriptors.bind_texture(slot(&program, "history"), &mut history);
			descriptors.bind_buffer(slot(&program, "parameters"), &mut parameters);
			descriptors.bind_image(output_slot(0), &mut result);
			descriptors.bind_image(output_slot(1), &mut history_output);
			run_pixel(&program, &mut descriptors, PIXEL, PIXEL_TILE, [SIDE, SIDE]);
			drop(descriptors);
			(rgba(&result, PIXEL), rgba(&history_output, PIXEL))
		}
	}

	const DARK: [f32; 3] = [0.2, 0.2, 0.2];
	const LIGHT: [f32; 3] = [0.6, 0.6, 0.6];
	/// A history between the checkerboard's two colors, inside its neighborhood's box.
	const BETWEEN: [f32; 3] = [0.4, 0.4, 0.4];

	fn assert_gray(actual: [f32; 4], expected: f32) {
		for channel in &actual[..3] {
			assert!((channel - expected).abs() < 0.002, "expected {expected}, found {actual:?}");
		}
	}

	/// A history inside the neighborhood's box pulls the pixel toward it, and the count grows by one.
	#[test]
	fn a_matching_history_pulls_the_pixel_toward_it() {
		let (result, history) = Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT).run();
		assert!(result[0] < LIGHT[0] - 0.05 && result[0] > BETWEEN[0], "{result:?}");
		assert!((history[3] * 255.0 - 101.0).abs() < 0.01, "{history:?}");
	}

	/// A history blended over `n` frames weighs at most n / (n + 1), so a young history follows the scene.
	#[test]
	fn a_young_history_weighs_by_its_age() {
		let young = Fixture::still(LIGHT, BETWEEN, 1).checkered(DARK, LIGHT).run().0;
		let old = Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT).run().0;
		assert!(young[0] > old[0] + 0.01, "young {young:?}, old {old:?}");
	}

	/// A reset frame, a frame whose history falls off screen, a disoccluded pixel, and a fully reactive pixel each take
	/// the current color and start the count over.
	#[test]
	fn rejected_history_leaves_the_current_color() {
		let fixture = || Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT);
		let reset = Fixture { reset: true, ..fixture() };
		let off_screen = Fixture { velocity: [6.0, 0.0, SURFACE_Z], ..fixture() };
		// Something 1 m away stood in front of the surface, 2 m away, in the previous frame.
		let disoccluded = Fixture { previous_z: 1.0, ..fixture() };
		let reactive = Fixture { reactive: 1.0, ..fixture() };
		for (name, case) in [("reset", reset), ("off screen", off_screen), ("disoccluded", disoccluded)] {
			let (result, history) = case.run();
			assert_gray(result, LIGHT[0]);
			assert_eq!(history[3], 0.0, "{name}");
		}
		assert_gray(reactive.run().0, LIGHT[0]);
	}

	/// A surface that moved toward the camera since the previous frame, so its previous depth is larger, keeps its
	/// history: only something in front of it drops the history.
	#[test]
	fn a_surface_farther_in_the_previous_frame_keeps_its_history() {
		let fixture = Fixture { previous_z: 3.0, velocity: [0.0, 0.0, 3.0], ..Fixture::still(LIGHT, BETWEEN, 100) };
		let (_, history) = fixture.checkered(DARK, LIGHT).run();
		assert!(history[3] > 0.0, "{history:?}");
	}

	/// A still pixel whose jittered samples swap between two colors every frame, as grass and its shadow do, settles
	/// into the ripple of the maximum feedback, (1 − f) |b − a| / (1 + f), instead of the minimum's, which the luma
	/// difference would pick for samples that never match the history.
	#[test]
	fn a_still_pixel_averages_its_jitter_instead_of_flickering() {
		let mut history = (BETWEEN, 100);
		let mut results = Vec::new();
		for frame in 0..64 {
			let (dark, light) = if frame % 2 == 0 { (DARK, LIGHT) } else { (LIGHT, DARK) };
			let (result, written) = Fixture::still(light, history.0, history.1).checkered(dark, light).run();
			history = ([written[0], written[1], written[2]], (written[3] * 255.0).round() as u32);
			results.push(result[0]);
		}
		let settled = &results[48..];
		let ripple = settled.iter().copied().fold(f32::MIN, f32::max) - settled.iter().copied().fold(f32::MAX, f32::min);
		let feedback = TaaPassSettings::default().feedback_max;
		let bound = (1.0 - feedback) * (LIGHT[0] - DARK[0]) / (1.0 + feedback);
		// The blend runs on colors divided by 1 + luma, which bends the swing a little.
		assert!(ripple < bound * 1.5, "ripple {ripple}, bound {bound}");
	}

	/// A pixel moving at 2 pixels per frame lets a changed color through sooner than a still one, so the luma difference
	/// still guards moving pixels against trails.
	#[test]
	fn a_moving_pixel_follows_a_changed_color_sooner() {
		let still = Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT);
		let moving = Fixture { velocity: [2.0, 0.0, SURFACE_Z], ..Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT) };
		let (still, moving) = (still.run().0, moving.run().0);
		assert!(moving[0] > still[0] + 0.005, "still {still:?}, moving {moving:?}");
	}

	/// A history far brighter than anything around the pixel is clipped to the neighborhood before it blends, so a
	/// bright object that moved away leaves no trail.
	#[test]
	fn a_stale_history_is_clipped_to_the_neighborhood() {
		let (result, _) = Fixture::still(LIGHT, [20.0, 20.0, 20.0], 100).checkered(DARK, LIGHT).run();
		assert!(result[0] <= LIGHT[0] + 0.002, "{result:?}");
		// Around a flat color the box closes on it, and the history can't change the pixel at all.
		let (flat, _) = Fixture::still(LIGHT, [20.0, 20.0, 20.0], 100).run();
		assert_gray(flat, LIGHT[0]);
	}
}
