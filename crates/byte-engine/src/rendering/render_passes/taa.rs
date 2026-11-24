//! Temporal antialiasing: resolves detail finer than a pixel by blending jittered frames, each reprojected through the
//! scene's velocity, while rejecting history that no longer matches the scene so moving objects leave no trails.
//!
//! [`TaaPass`] asks its sink for [jitter](crate::rendering::Sink::with_jitter), which makes the visibility pipeline
//! write the `Velocity` and `Reactive` targets the pass reads. Install it with
//! [`crate::application::graphics::setup_taa_render_pass`] in place of SMAA. See the temporal antialiasing design note.
//!
//! The pass blends with one [`TemporalTechnique`]: the engine's own resolve shader, or a vendor temporal upscaler, such
//! as MetalFX, which also renders the scene below the window's resolution; see the
//! [render scale](/docs/reference/rendering#render-scale) specification.

use std::cell::Cell;

use ghi::{
	command_buffer::CommandBufferRecording as _,
	context::{Context as _, ContextCreate as _},
	frame::Frame as _,
};
use utils::Extent;

use crate::{
	configuration::{Accepts, ConfigurationPort, Read, Setting},
	rendering::{
		DebugViews, ShownDebugViews, Sink,
		debug_views::DEBUG_VIEW_TARGETS_BINDING,
		render_pass::{RenderFeature, RenderPass, RenderPassBuilder, RenderPassReturn, allocate_render_command, simple_tile},
	},
};

/// The `TemporalTechnique` enum names how the temporal antialiasing pass blends frames, so an application can trade the
/// engine's own resolve for a vendor upscaler that also renders the scene at a lower resolution.
///
/// Set it through [`TaaPassSettings::technique`] or the `render.taa.technique` parameter.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TemporalTechnique {
	/// The engine's own resolve shader, at the window's resolution.
	#[default]
	Engine,
	/// MetalFX's temporal scaler, on Metal devices that support it, which upscales by [`TaaPassSettings::scale`].
	MetalFx,
}

impl TemporalTechnique {
	/// The parameter words, in variant order.
	const NAMES: &'static [&'static str] = &["engine", "metalfx"];
}

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
	/// How the pass blends frames. A technique the device lacks resolves with [`TemporalTechnique::Engine`].
	pub technique: TemporalTechnique,
	/// The fraction of the window's pixel size, on each axis, the scene renders at while an upscaling technique runs,
	/// above 0 and at most 1. Held to the range the technique supports; [`TemporalTechnique::Engine`] always renders at
	/// 1.
	pub scale: f32,
}

impl Default for TaaPassSettings {
	/// Returns INSIDE's feedback range and Salvi's variance gamma.
	fn default() -> Self {
		Self {
			feedback_min: 0.88,
			feedback_max: 0.97,
			variance_gamma: 1.0,
			technique: TemporalTechnique::Engine,
			scale: 1.0,
		}
	}
}

impl TaaPassSettings {
	/// The runtime settings, named inside the feature's configuration port.
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
				Ok(Self {
					feedback_min,
					..settings
				})
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
				Ok(Self {
					feedback_max,
					..settings
				})
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
				Ok(Self {
					variance_gamma,
					..settings
				})
			},
		},
		Setting {
			name: "technique",
			accepts: Accepts::Choice(TemporalTechnique::NAMES),
			value: |settings| Read::Choice(settings.technique as usize),
			apply: |settings, read| {
				let technique = match read.choice() {
					0 => TemporalTechnique::Engine,
					_ => TemporalTechnique::MetalFx,
				};
				Ok(Self { technique, ..settings })
			},
		},
		Setting {
			name: "scale",
			accepts: Accepts::Number { min: 0.0 },
			value: |settings| Read::Number(f64::from(settings.scale)),
			apply: |settings, read| {
				let scale = read.number() as f32;
				if scale <= 0.0 || scale > 1.0 {
					return Err(format!(
						"The render scale {scale} is outside the range above 0 through 1. The most likely cause is a scale written as an upscale factor, such as 1.5 for 1.5x. Pass its inverse, such as 0.67."
					));
				}
				Ok(Self { scale, ..settings })
			},
		},
	];
}

/// The `Taa` struct holds the settings every sink's [`TaaPass`] resolves with, so a runtime parameter changes the
/// antialiasing of every window at once.
///
/// Create it with [`Self::new`] from a [`ConfigurationPort`] registered for `render.taa.`, and register it with
/// [`crate::rendering::Renderer::add_render_pass`], or with [`crate::application::graphics::setup_taa_render_pass`].
pub struct Taa {
	settings: TaaPassSettings,
	configuration: ConfigurationPort,
}

impl Taa {
	/// Creates the feature that starts at `settings` and applies the runtime updates queued on `configuration`.
	pub fn new(settings: TaaPassSettings, configuration: ConfigurationPort) -> Self {
		configuration.declare(TaaPassSettings::SETTINGS.iter().map(|setting| setting.parameter(settings)));
		Self { settings, configuration }
	}

	/// Returns the settings every pass resolves with this frame.
	pub fn settings(&self) -> TaaPassSettings {
		self.settings
	}
}

impl RenderFeature for Taa {
	type Pass = TaaPass;

	fn name(&self) -> &'static str {
		"taa"
	}

	fn create_pass(&mut self, builder: &mut RenderPassBuilder) -> TaaPass {
		TaaPass::new(builder)
	}

	fn update(&mut self) {
		self.configuration.drain(&mut self.settings, TaaPassSettings::SETTINGS);
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
	parameters: ghi::BufferHandle<TaaShaderData>,
	resolve_pass: simple_tile::Pass,
	debug_views: DebugViews,
	/// The images and formats a vendor upscaler reads and writes, or `None` when the device has none.
	upscaler_images: Option<UpscalerImages>,
	/// The vendor upscaler built for the current render and output extents.
	upscaler: Option<BuiltUpscaler>,
	/// Whether the pass already warned that the device lacks the requested technique, so it warns once.
	warned_unsupported: Cell<bool>,
	/// The frame this pass resolved last, or `None` when it didn't resolve the frame before this one, whose history
	/// then holds nothing usable.
	previous: Option<PreviousFrame>,
}

/// The `PreviousFrame` struct records what the frame before this one resolved with, so the pass can tell when its
/// history no longer matches.
#[derive(Clone, Copy, Debug, PartialEq)]
struct PreviousFrame {
	render_extent: Extent,
	output_extent: Extent,
	exposure_scale: f32,
	/// The exposure scale of the frame the upscaler's history started on, which its pre-exposure is relative to.
	history_exposure: f32,
	technique: TemporalTechnique,
}

/// The `UpscalerImages` struct holds what a vendor upscaler needs from the sink's targets, read once when the pass is
/// built.
#[derive(Clone, Copy, Debug)]
struct UpscalerImages {
	color: ghi::BaseImageHandle,
	color_format: ghi::Formats,
	depth: ghi::BaseImageHandle,
	motion: ghi::BaseImageHandle,
	reactive: ghi::BaseImageHandle,
	output: ghi::BaseImageHandle,
	output_format: ghi::Formats,
	/// The input-to-output scales the device's upscaler takes.
	scales: (f32, f32),
}

/// The `BuiltUpscaler` struct pairs a vendor upscaler with the extents it was built for, since it can't change them.
#[derive(Clone, Copy, Debug)]
struct BuiltUpscaler {
	handle: ghi::upscaler::TemporalUpscalerHandle,
	render_extent: Extent,
	output_extent: Extent,
}

impl TaaPass {
	/// Creates a TAA pass for one sink, which resolves with the settings of its [`Taa`] feature, and remaps `main` for
	/// downstream passes.
	///
	/// # Panics
	///
	/// Panics when the sink has no `Velocity` target, because its scene pipeline isn't the visibility pipeline.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let source = render_pass_builder.read_from("main");
		let velocity = render_pass_builder.read_from("Velocity");
		let reactive = render_pass_builder.read_from("Reactive");
		let depth = render_pass_builder.read_from("Depth");
		let main_format = render_pass_builder.format_of("main");
		// The output follows the window's extent, which an upscaling technique writes; the engine's resolve renders at
		// that extent too, since it doesn't upscale. A vendor upscaler writes it as a storage image.
		let output = render_pass_builder.create_output_main_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Image | ghi::Uses::Storage).name("TAA Output"),
		);
		// A vendor upscaler needs an image it can write, which a pass that writes the window directly doesn't have.
		let upscaler_images = match (
			render_pass_builder.context().temporal_upscaler_scale_range(),
			ghi::ImageOrSwapchain::from(output),
		) {
			(Some(scales), ghi::ImageOrSwapchain::Image(output_image)) => Some(UpscalerImages {
				color: source.into(),
				color_format: main_format,
				depth: depth.into(),
				motion: velocity.into(),
				reactive: reactive.into(),
				output: output_image,
				output_format: output.format(),
				scales: (*scales.start(), *scales.end()),
			}),
			_ => None,
		};
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
			parameters,
			resolve_pass,
			debug_views,
			upscaler_images,
			upscaler: None,
			warned_unsupported: Cell::new(false),
			previous: None,
		}
	}

	/// Returns the technique the pass resolves with under `settings`: the requested one, or the engine's own when the
	/// device lacks it, which it warns about once.
	fn technique(&self, settings: &TaaPassSettings) -> TemporalTechnique {
		let technique = resolved_technique(settings.technique, self.upscaler_images.is_some());
		if technique != settings.technique && !self.warned_unsupported.replace(true) {
			log::warn!(
				"Temporal antialiasing can't use the `metalfx` technique on this device and resolves with `engine` instead. The most likely cause is a GPU or backend without MetalFX's Metal 4 temporal scaler. Set `render.taa.technique` to `engine`. See {}",
				crate::online_docs_url("reference/rendering#temporal-antialiasing")
			);
		}
		technique
	}

	/// Returns the upscaler for this frame's extents, building it again when they changed, or `None` when the device
	/// rejected it.
	fn upscaler(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		images: UpscalerImages,
	) -> Option<BuiltUpscaler> {
		let (render_extent, output_extent) = (sink.render_extent(), sink.output_extent());
		if let Some(upscaler) = self
			.upscaler
			.filter(|upscaler| (upscaler.render_extent, upscaler.output_extent) == (render_extent, output_extent))
		{
			return Some(upscaler);
		}
		let builder = ghi::upscaler::Builder::new(
			render_extent,
			output_extent,
			[images.color_format, ghi::Formats::Depth32, VELOCITY_FORMAT],
			images.output_format,
		)
		.name("TAA MetalFX")
		.reactive_format(ghi::Formats::R8UNORM)
		.depth_reversed(true);
		match frame.build_temporal_upscaler(builder) {
			Ok(handle) => {
				self.upscaler = Some(BuiltUpscaler {
					handle,
					render_extent,
					output_extent,
				});
				self.upscaler
			}
			Err(error) => {
				// Without an upscaler the pass can't resolve at all, so it stops offering one.
				log::error!("{error}");
				self.upscaler_images = None;
				None
			}
		}
	}
}

/// The format of `Velocity`, which a vendor upscaler reads its motion from.
const VELOCITY_FORMAT: ghi::Formats = ghi::Formats::RGBA16F;

/// Returns the technique the pass resolves with: `requested`, unless it needs a vendor upscaler the device lacks.
fn resolved_technique(requested: TemporalTechnique, has_upscaler: bool) -> TemporalTechnique {
	match requested {
		TemporalTechnique::MetalFx if !has_upscaler => TemporalTechnique::Engine,
		technique => technique,
	}
}

/// Returns the render scale `technique` renders at: 1 for the engine's resolve, and `requested` held to the device's
/// `scales` for an upscaler.
fn technique_render_scale(technique: TemporalTechnique, requested: f32, (min, max): (f32, f32)) -> f32 {
	match technique {
		TemporalTechnique::Engine => 1.0,
		TemporalTechnique::MetalFx => requested.clamp(min, max),
	}
}

/// Builds a vendor upscaler's inputs for `sink` from `images`, resetting its history when `previous` didn't run it at
/// the same extents. Also returns the exposure scale the history is relative to from this frame on.
fn upscaler_inputs(images: UpscalerImages, sink: &Sink, previous: Option<PreviousFrame>) -> (ghi::upscaler::Inputs, f32) {
	let history = previous.filter(|previous| {
		(previous.render_extent, previous.output_extent, previous.technique)
			== (sink.render_extent(), sink.output_extent(), TemporalTechnique::MetalFx)
	});
	// MetalFX divides the color by the pre-exposure and keeps its history in half floats. The full camera exposure
	// of a sunlit scene is about 2⁻¹⁵, which would push the color past their range, so the pre-exposure is relative to
	// the frame the history started on: near 1, and still rescaling the history when the exposure changes.
	let history_exposure = history.map_or(sink.exposure_scale(), |previous| previous.history_exposure);
	let inputs = ghi::upscaler::Inputs {
		color: images.color,
		depth: images.depth,
		motion: images.motion,
		reactive: Some(images.reactive),
		output: images.output,
		content_extent: sink.render_extent(),
		// MetalFX takes the offset as the sink holds it, in pixels with y down; see the design note.
		jitter: sink.jitter(),
		// `Velocity` holds where the pixel moved to; MetalFX wants where it came from.
		motion_scale: [-1.0, -1.0],
		pre_exposure: sink.exposure_scale() / history_exposure,
		reset: history.is_none(),
	};
	(inputs, history_exposure)
}

impl RenderPass<Taa> for TaaPass {
	fn prepare<'a>(
		&mut self,
		feature: &mut Taa,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let settings = feature.settings();
		let technique = self.technique(&settings);
		let previous = self.previous.take();
		let mut current = PreviousFrame {
			render_extent: sink.render_extent(),
			output_extent: sink.output_extent(),
			exposure_scale: sink.exposure_scale(),
			history_exposure: sink.exposure_scale(),
			technique,
		};
		let command = match (technique, self.upscaler_images) {
			(TemporalTechnique::MetalFx, Some(images)) => {
				let upscaler = self.upscaler(frame, sink, images)?;
				let (inputs, history_exposure) = upscaler_inputs(images, sink, previous);
				current.history_exposure = history_exposure;
				allocate_render_command(
					frame_allocator,
					move |recording: &mut ghi::implementation::CommandBufferRecording| {
						recording.upscale(upscaler.handle, &inputs)
					},
				)
			}
			_ => {
				let command = self.resolve_pass.prepare(frame, sink, frame_allocator)?;
				// The engine's history is usable only after a frame the engine itself resolved.
				let history = previous
					.filter(|previous| previous.technique == TemporalTechnique::Engine)
					.map(|previous| (previous.output_extent, previous.exposure_scale));
				*frame.map(self.parameters) = taa_shader_data(
					&settings,
					sink,
					history,
					self.debug_views.shown(sink, 0..TAA_DEBUG_VIEWS.len()),
				);
				command
			}
		};
		self.previous = Some(current);
		Some(command)
	}

	fn bypass<'a>(
		&mut self,
		_feature: &mut Taa,
		_frame: &mut ghi::implementation::Frame,
		_sink: &Sink,
		_frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// A bypassed frame writes no history, so the frame after it starts the history over. The renderer forwards
		// `main` on its own.
		self.previous = None;
		None
	}

	fn jitters_view(&self, _feature: &Taa) -> bool {
		true
	}

	fn render_scale(&self, feature: &Taa) -> f32 {
		let settings = feature.settings();
		let scales = self.upscaler_images.map_or((1.0, 1.0), |images| images.scales);
		technique_render_scale(self.technique(&settings), settings.scale, scales)
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
	use crate::rendering::View;
	use crate::rendering::render_pass::simple_compute;
	use crate::rendering::shader_vm_test::{PIXEL_TILE, buffer, compile, empty_image, rgba, run_pixel, slot, texture_2d};

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
			let program = compile(&simple_compute::compile_test_program(RESOLVE_BESL));
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
		let reset = Fixture {
			reset: true,
			..fixture()
		};
		let off_screen = Fixture {
			velocity: [6.0, 0.0, SURFACE_Z],
			..fixture()
		};
		// Something 1 m away stood in front of the surface, 2 m away, in the previous frame.
		let disoccluded = Fixture {
			previous_z: 1.0,
			..fixture()
		};
		let reactive = Fixture {
			reactive: 1.0,
			..fixture()
		};
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
		let fixture = Fixture {
			previous_z: 3.0,
			velocity: [0.0, 0.0, 3.0],
			..Fixture::still(LIGHT, BETWEEN, 100)
		};
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
		let moving = Fixture {
			velocity: [2.0, 0.0, SURFACE_Z],
			..Fixture::still(LIGHT, BETWEEN, 100).checkered(DARK, LIGHT)
		};
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

	/// A device without the requested upscaler resolves with the engine's shader, and the engine's shader renders at
	/// full scale whatever `render.taa.scale` says, as the temporal antialiasing specification gives it.
	#[test]
	fn a_missing_upscaler_falls_back_to_the_engine_at_full_scale() {
		assert_eq!(
			resolved_technique(TemporalTechnique::MetalFx, false),
			TemporalTechnique::Engine
		);
		assert_eq!(
			resolved_technique(TemporalTechnique::MetalFx, true),
			TemporalTechnique::MetalFx
		);
		assert_eq!(resolved_technique(TemporalTechnique::Engine, true), TemporalTechnique::Engine);
		assert_eq!(technique_render_scale(TemporalTechnique::Engine, 0.5, (0.33, 1.0)), 1.0);
	}

	/// An upscaler renders at the requested scale, held to the range the device takes.
	#[test]
	fn an_upscaler_holds_the_scale_to_the_device_range() {
		assert_eq!(technique_render_scale(TemporalTechnique::MetalFx, 0.67, (0.33, 1.0)), 0.67);
		assert_eq!(technique_render_scale(TemporalTechnique::MetalFx, 0.1, (0.33, 1.0)), 0.33);
	}

	/// MetalFX gets motion pointing back to the previous frame, the jitter as the sink holds it, the exposure relative to the frame
	/// its history started on, and a reset whenever the frame before it didn't run MetalFX at the same extents.
	#[test]
	fn upscaler_inputs_follow_the_metalfx_conventions() {
		let image = crate::rendering::renderer::tests::image_handle;
		let images = UpscalerImages {
			color: image(1),
			color_format: ghi::Formats::RGBu11u11u10,
			depth: image(2),
			motion: image(3),
			reactive: image(4),
			output: image(5),
			output_format: ghi::Formats::RGBu11u11u10,
			scales: (0.33, 1.0),
		};
		let sink = Sink::new(View::identity(), Extent::rectangle(1920, 1080), 0)
			.with_render_scale(0.5)
			.with_exposure_scale(2.0)
			.with_jitter([0.25, -0.125])
			.at_output();
		let ran = |technique, render_extent| PreviousFrame {
			render_extent,
			output_extent: Extent::rectangle(1920, 1080),
			exposure_scale: 0.5,
			history_exposure: 0.5,
			technique,
		};

		let (inputs, history_exposure) = upscaler_inputs(
			images,
			&sink,
			Some(ran(TemporalTechnique::MetalFx, Extent::rectangle(960, 540))),
		);
		assert_eq!(inputs.motion_scale, [-1.0, -1.0]);
		assert_eq!(inputs.jitter, [0.25, -0.125]);
		// The exposure doubled twice since the history started.
		assert_eq!((inputs.pre_exposure, history_exposure), (4.0, 0.5));
		assert_eq!(inputs.content_extent, Extent::rectangle(960, 540));
		assert!(!inputs.reset);
		let (reset, history_exposure) = upscaler_inputs(images, &sink, None);
		assert!(reset.reset);
		assert_eq!((reset.pre_exposure, history_exposure), (1.0, 2.0));
		assert!(
			upscaler_inputs(
				images,
				&sink,
				Some(ran(TemporalTechnique::Engine, Extent::rectangle(960, 540)))
			)
			.0
			.reset
		);
		assert!(
			upscaler_inputs(
				images,
				&sink,
				Some(ran(TemporalTechnique::MetalFx, Extent::rectangle(1280, 720)))
			)
			.0
			.reset
		);
	}
}
