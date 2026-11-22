//! Motion blur: smears the HDR scene along each pixel's motion, as a shutter open for part of the frame would.
//!
//! The pass reads the `Velocity` target the visibility pipeline writes, so camera turns, camera moves, moving objects,
//! and skinned surfaces all blur. It follows the reconstruction filter of McGuire et al., "A Reconstruction Filter for
//! Plausible Motion Blur" (I3D 2012): a tile max and a neighbor max find the longest blur that can reach each pixel,
//! and a depth-aware gather samples along it. See [`MotionBlurPass`] and the motion blur design note.

use std::{cell::Cell, rc::Rc};

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};
use utils::Extent;

use crate::{
	accessibility::MotionPreference,
	configuration::{Accepts, ConfigurationPort, Read, Setting},
	core::Entity,
	rendering::{
		DebugViews, ShownDebugViews, Sink,
		debug_views::DEBUG_VIEW_TARGETS_BINDING,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute, simple_tile},
		render_passes::blit::ImageBypassPass,
	},
	time::MediaTime,
};

/// The most taps one pixel gathers; the blur shader loops to this many.
const MAX_TAPS: u32 = 32;
/// The side of the square tiles the tile max and neighbor max work in, in pixels, which bounds the longest blur: a
/// blur no longer than a tile reaches at most the next tile. The tile max covers a tile with a 16x16 workgroup.
const TILE_WIDTH: u32 = 32;
/// The lanes along each side of the tile max's workgroup, each of which reads 2x2 pixels.
const TILE_MAX_WORKGROUP_WIDTH: u32 = 16;
/// The most tiles a sink holds, which sizes the tile buffers: about 16.7 million pixels.
const MAX_TILES: u32 = 16_384;

/// The `MotionBlurPassSettings` struct defines how long the shutter stays open and how far the blur may reach.
///
/// Pass it to [`crate::application::graphics::setup_motion_blur_render_pass`] or [`MotionBlurControls::new`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MotionBlurPassSettings {
	/// How long the shutter stays open each frame. The blur covers that fraction of the camera's motion since the
	/// previous frame, so at a steady frame rate it is the shutter angle: `1/120` s is a 180° shutter at 60 Hz. A
	/// shutter longer than the frame covers the whole motion.
	pub shutter: MediaTime,
	/// The longest blur, in pixels from a pixel to either end of its blur, at most 32. A fast motion smears no further,
	/// which bounds how far each pixel's taps reach.
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
	/// The runtime settings, named inside the controls' configuration port.
	pub(crate) const SETTINGS: &'static [Setting<Self>] = &[Setting {
		name: "taps",
		accepts: Accepts::Whole {
			min: 1,
			max: MAX_TAPS,
			step: 1,
		},
		value: |settings| Read::Whole(settings.taps),
		apply: |settings, read| {
			let taps = read.whole();
			Ok(Self { taps, ..settings })
		},
	}];
}

/// The `MotionBlurControls` struct shares one set of settings between every sink's [`MotionBlurPass`], so a
/// runtime parameter changes the blur of every window at once.
///
/// Create it with [`Self::new`] from a [`ConfigurationPort`] registered for the prefix the application gives
/// motion blur, and hand the same `Rc` to each pass.
pub struct MotionBlurControls {
	settings: Cell<MotionBlurPassSettings>,
	configuration: ConfigurationPort,
	/// The player's motion preference, which the application shares with every set of controls.
	motion: Rc<Cell<MotionPreference>>,
}

impl MotionBlurControls {
	/// Creates controls that start at `settings`, apply the runtime updates queued on `configuration`, and draw no blur
	/// while `motion` is [`MotionPreference::Reduced`]. Pass the application's shared preference as `motion`.
	pub fn new(
		settings: MotionBlurPassSettings,
		configuration: ConfigurationPort,
		motion: Rc<Cell<MotionPreference>>,
	) -> Rc<Self> {
		configuration.declare(
			MotionBlurPassSettings::SETTINGS
				.iter()
				.map(|setting| setting.parameter(settings)),
		);
		Rc::new(Self {
			settings: Cell::new(settings),
			configuration,
			motion,
		})
	}

	/// Applies the queued runtime updates and returns the settings every pass draws with this frame. Under reduced
	/// motion the shutter is closed, which every pass draws as no blur, while the stored settings stay for when motion
	/// is full again.
	pub fn settings(&self) -> MotionBlurPassSettings {
		let mut settings = self.settings.get();
		self.configuration.drain(&mut settings, MotionBlurPassSettings::SETTINGS);
		self.settings.set(settings);
		match self.motion.get() {
			MotionPreference::Full => settings,
			MotionPreference::Reduced => MotionBlurPassSettings {
				shutter: MediaTime::ZERO,
				..settings
			},
		}
	}
}

/// The `MotionBlurShaderData` struct mirrors the `MotionBlurParameters` layout of `motion_blur/layouts/parameters.besl`.
///
/// Everything the same for every pixel is computed here so the shaders do not repeat it.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MotionBlurShaderData {
	/// The shutter's fraction of the frame; see [`shutter_fraction`].
	shutter_fraction: f32,
	/// The longest blur, in pixels from a pixel to either end, at most [`TILE_WIDTH`].
	max_radius: f32,
	/// How many tiles make one row and one column of the tile buffers.
	tiles: [u32; 2],
	/// Shifts the gather's interleaved gradient noise each frame.
	noise_offset: f32,
	/// The most taps one pixel gathers.
	taps: u32,
	/// The pool slot the first shown debug view writes; see [`ShownDebugViews`].
	debug_view_first_slot: u32,
	/// The shader's `bool` members, one bit per view of [`MOTION_BLUR_DEBUG_VIEWS`].
	debug_views: u8,
	_padding: [u8; 3],
}

/// The debug views the pass declares, in the order `motion_blur/layouts/parameters.besl` numbers them from `0`.
const MOTION_BLUR_DEBUG_VIEWS: [&str; 3] = ["velocity", "blur-length", "neighbor-max"];

/// The `MotionBlurPass` struct creates a reusable motion blur stage for the HDR scene.
///
/// Install it after the passes that write scene light and the fog, and before lens flare, bloom, and tone mapping,
/// so light glows over the blurred image. It reads the `Velocity` target, so it needs the visibility pipeline, which
/// writes that target while this pass is enabled. Toggle it at runtime with the `render.pass.motion-blur` parameter
/// and change its taps with `render.motion-blur.taps`.
pub struct MotionBlurPass {
	controls: Rc<MotionBlurControls>,
	parameters: ghi::BufferHandle<MotionBlurShaderData>,
	tile_max_pass: simple_compute::Pass,
	neighbor_max_pass: simple_compute::Pass,
	gather_pass: simple_tile::Pass,
	/// The GPU counters around the tile max, the neighbor max, and the gather.
	counters: [ghi::CounterHandle; 3],
	/// Which debug views to write this frame.
	debug_views: DebugViews,
	/// Forwards `main` unchanged while the shutter is closed or the shaders are still compiling.
	main_copy: ImageBypassPass,
	/// Counts the frames drawn, which cycles the sample jitter so it does not sit still on a still image.
	frame: u32,
}

impl Entity for MotionBlurPass {}

impl MotionBlurPass {
	/// Creates a motion blur pass drawing with `controls`, shared with the passes of the other sinks, and remaps
	/// `main` for downstream passes.
	///
	/// Panics when the sink has no `Velocity` target, because its scene pipeline isn't the visibility pipeline.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, controls: Rc<MotionBlurControls>) -> Self {
		let source = render_pass_builder.read_from("main");
		let velocity: ghi::BaseImageHandle = render_pass_builder
			.try_read_from("Velocity")
			.expect(
			"Motion blur has no `Velocity` target to read. The most likely cause is a sink whose scene pipeline isn't the visibility pipeline, which writes it. Install the visibility pipeline before motion blur, or leave motion blur out.",
			)
			.into();
		let main_format = render_pass_builder.format_of("main");
		// The blur's tile output, which the builder gives the uses that needs.
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Image).name("Motion Blur"));
		// The builder's copy of the incoming `main` into `output` doubles as this pass's forwarding copy.
		let main_copy = render_pass_builder.take_main_copy().expect(
			"Main copy is missing. The most likely cause is that the pass did not call `RenderPassBuilder::create_main_render_target` first.",
		);

		let compute = |render_pass_builder: &mut RenderPassBuilder, label, id| {
			simple_compute::Pipeline::compile(render_pass_builder, simple_compute::Descriptor::new(label, id))
		};
		let tile_max = compute(
			render_pass_builder,
			"Motion Blur Tile Max",
			"byte_engine/rendering/motion_blur/tile-max.pipeline",
		);
		let neighbor_max = compute(
			render_pass_builder,
			"Motion Blur Neighbor Max",
			"byte_engine/rendering/motion_blur/neighbor-max.pipeline",
		);
		let gather = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Motion Blur",
				"byte_engine/rendering/motion_blur/gather.pipeline",
				&[output.format()],
			),
		);
		let debug_views = render_pass_builder.debug_views(&MOTION_BLUR_DEBUG_VIEWS);
		let counters = ["motion-blur-tile-max", "motion-blur-neighbor-max", "motion-blur-gather"]
			.map(|name| render_pass_builder.create_gpu_counter(name));
		let context = render_pass_builder.context();
		let parameters = context.build_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Motion Blur Parameters")
				.host_access(ghi::HostAccess::Write)
				.copies(ghi::Copies::PerFrame),
		);
		let tile_buffer = |context: &mut ghi::implementation::Context, name| {
			context.build_buffer::<[[f32; 2]]>(
				ghi::buffer::Builder::new(ghi::Uses::Storage)
					.name(name)
					.length(MAX_TILES as usize),
			)
		};
		let tile_max_buffer = tile_buffer(context, "Motion Blur Tile Max");
		let neighbor_max_buffer = tile_buffer(context, "Motion Blur Neighbor Max");
		// Every stage fetches exact texels.
		let sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest),
		);
		let read = ghi::Layouts::Read;

		let tile_max_pass = tile_max.bind(
			"Motion Blur Tile Max Descriptor Set",
			&[
				simple_compute::Resource::combined_image_sampler("velocity_texture", velocity, sampler, read),
				simple_compute::Resource::buffer("parameters", parameters),
				simple_compute::Resource::buffer("tile_max", tile_max_buffer),
			],
		);
		let neighbor_max_pass = neighbor_max.bind(
			"Motion Blur Neighbor Max Descriptor Set",
			&[
				simple_compute::Resource::buffer("parameters", parameters),
				simple_compute::Resource::buffer("tile_max", tile_max_buffer),
				simple_compute::Resource::buffer("neighbor_max", neighbor_max_buffer),
			],
		);
		let gather_pass = gather.bind(
			"Motion Blur Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("scene_texture", source, sampler, read),
				simple_tile::Resource::combined_image_sampler("velocity_texture", velocity, sampler, read),
				simple_tile::Resource::buffer("parameters", parameters),
				simple_tile::Resource::buffer("neighbor_max", neighbor_max_buffer),
				simple_tile::Resource::image_array(DEBUG_VIEW_TARGETS_BINDING, debug_views.targets()),
			],
			&[simple_tile::Output::new("result", output)],
		);

		Self {
			controls,
			parameters,
			tile_max_pass,
			neighbor_max_pass,
			gather_pass,
			counters,
			debug_views,
			main_copy,
			frame: 0,
		}
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
		self.frame = self.frame.wrapping_add(1);
		let fraction = shutter_fraction(settings.shutter, sink.frame_interval());
		let shown = self.debug_views.shown(0..MOTION_BLUR_DEBUG_VIEWS.len());
		// A closed shutter blurs nothing, so `main` passes through unless a debug view needs the stages.
		if fraction <= 0.0 && shown.views == 0 {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		}
		let extent = sink.extent();
		let tiles = [extent.width().div_ceil(TILE_WIDTH), extent.height().div_ceil(TILE_WIDTH)];
		if tiles[0] * tiles[1] > MAX_TILES {
			log::error!(
				"Motion blur was skipped. The most likely cause is a window of {} by {} pixels, more than the {MAX_TILES} tiles of {TILE_WIDTH} pixels the pass holds. Make the window smaller or raise `MAX_TILES`.",
				extent.width(),
				extent.height()
			);
			return self.main_copy.prepare(frame, sink, frame_allocator);
		}
		let (Some(tile_max), Some(neighbor_max), Some(gather)) = (
			self.tile_max_pass.ready(frame),
			self.neighbor_max_pass.ready(frame),
			self.gather_pass.ready(frame),
		) else {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		*frame.map(self.parameters) = motion_blur_shader_data(&settings, fraction, tiles, self.frame, shown);
		let [tile_max_counter, neighbor_max_counter, gather_counter] = self.counters;
		// One 16x16 workgroup per tile, and one lane per tile for the neighbor max.
		let tile_max_extent = Extent::rectangle(tiles[0] * TILE_MAX_WORKGROUP_WIDTH, tiles[1] * TILE_MAX_WORKGROUP_WIDTH);
		let tile_extent = Extent::rectangle(tiles[0], tiles[1]);
		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("Motion Blur"),
					|command_buffer| {
						command_buffer.counter(tile_max_counter, |command_buffer| {
							tile_max.record(command_buffer, tile_max_extent)
						});
						command_buffer.counter(neighbor_max_counter, |command_buffer| {
							neighbor_max.record(command_buffer, tile_extent)
						});
						command_buffer.counter(gather_counter, |command_buffer| gather.record(command_buffer, extent));
					},
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
		// A bypassed frame still applies the queued parameters, so they report their result. This pass took the
		// builder's forwarding copy, so it forwards `main` itself.
		self.controls.settings();
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

/// Builds one sink's blur constants for a frame whose shutter covers `fraction` of the motion over `tiles` tiles,
/// writing the `shown` debug views.
fn motion_blur_shader_data(
	settings: &MotionBlurPassSettings,
	fraction: f32,
	tiles: [u32; 2],
	frame: u32,
	shown: ShownDebugViews,
) -> MotionBlurShaderData {
	MotionBlurShaderData {
		shutter_fraction: fraction,
		max_radius: settings.max_blur_radius.clamp(0.0, TILE_WIDTH as f32),
		tiles,
		// The noise shifts by this much per frame and repeats after 64 frames.
		noise_offset: 5.588_238 * (frame % 64) as f32,
		taps: settings.taps.clamp(1, MAX_TAPS),
		debug_view_first_slot: shown.first_slot,
		debug_views: shown.views,
		_padding: [0; 3],
	}
}

#[cfg(test)]
mod tests {
	use besl::vm::{DescriptorBindings, ExecutableProgram, Value, WorkgroupState, output_slot};

	use super::*;
	use crate::configuration::{ConfigurationValue, apply_setting};
	use crate::rendering::shader_vm_test::{
		PIXEL_TILE, assert_rgba_close, asset_source, binding_slot, buffer, compile, empty_image, lane_config, rgba, run_at,
		run_pixel, slot, texture_2d,
	};

	/// The shader declares one `bool` member per debug view, consecutive and in the order the Rust list declares them.
	#[test]
	fn debug_views_are_declared_in_order() {
		let members = MOTION_BLUR_DEBUG_VIEWS
			.iter()
			.map(|view| format!("\tdebug_view_{}: bool,\n", view.replace('-', "_")))
			.collect::<String>();

		assert!(
			asset_source("motion_blur/layouts/parameters.besl").contains(&members),
			"{members}"
		);
	}

	#[test]
	fn taps_parameter_takes_whole_numbers_within_the_shader_limit() {
		let settings = MotionBlurPassSettings::default();
		let text = |value: &str| ConfigurationValue::Text(value.to_string());

		let (updated, effective) = apply_setting(
			settings,
			MotionBlurPassSettings::SETTINGS,
			"",
			"taps",
			&ConfigurationValue::Integer(16),
		)
		.unwrap();
		assert_eq!((updated.taps, effective), (16, ConfigurationValue::Integer(16)));
		let (updated, effective) = apply_setting(settings, MotionBlurPassSettings::SETTINGS, "", "taps", &text("4")).unwrap();
		assert_eq!((updated.taps, effective), (4, ConfigurationValue::Integer(4)));
		for value in [
			ConfigurationValue::Integer(0),
			ConfigurationValue::Integer(33),
			ConfigurationValue::Float(2.5),
			text("many"),
		] {
			assert!(
				apply_setting(settings, MotionBlurPassSettings::SETTINGS, "", "taps", &value).is_err(),
				"{value:?}"
			);
		}
		assert!(
			apply_setting(
				settings,
				MotionBlurPassSettings::SETTINGS,
				"",
				"shutter",
				&ConfigurationValue::Float(0.01)
			)
			.is_err()
		);
	}

	#[test]
	fn controls_apply_queued_taps_and_report_them() {
		use crate::configuration::{Configuration, ConfigurationUpdateState};

		let configuration = Configuration::new();
		let port = configuration.register("render.motion-blur.");
		let controls = MotionBlurControls::new(MotionBlurPassSettings::default(), port, Rc::default());
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
	fn reduced_motion_closes_the_shutter_and_full_motion_restores_it() {
		use crate::configuration::Configuration;

		let configuration = Configuration::new();
		let motion = Rc::new(Cell::new(MotionPreference::Reduced));
		let settings = MotionBlurPassSettings::default();
		let controls = MotionBlurControls::new(settings, configuration.register("render.motion-blur."), motion.clone());

		assert_eq!(controls.settings().shutter, MediaTime::ZERO);
		motion.set(MotionPreference::Full);
		assert_eq!(controls.settings(), settings);
	}

	#[test]
	fn shutter_fraction_follows_the_frame_interval() {
		let frame = MediaTime::from_micros(16_667);
		assert_eq!(shutter_fraction(MediaTime::from_micros(8_333), MediaTime::ZERO), 0.0);
		assert_eq!(shutter_fraction(MediaTime::ZERO, frame), 0.0);
		assert_eq!(shutter_fraction(MediaTime::from_micros(100_000), frame), 1.0);
		assert!((shutter_fraction(MediaTime::from_micros(8_333), frame) - 0.5).abs() < 1e-3);
	}

	/// The fixture image: two tiles wide and one tall.
	const WIDTH: u32 = 64;
	const HEIGHT: u32 = 8;
	const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
	const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

	/// One column of the fixture: its color, and its `Velocity` texel of motion in pixels and linear depth.
	#[derive(Clone, Copy)]
	struct Column {
		color: [f32; 4],
		motion: [f32; 2],
		depth: f32,
	}

	/// Writes the shared constants of a frame whose shutter covers `fraction` of the motion into `parameters`.
	fn write_parameters(parameters: &mut besl::vm::Buffer, fraction: f32) {
		let tiles = [WIDTH.div_ceil(TILE_WIDTH), HEIGHT.div_ceil(TILE_WIDTH)];
		let data = motion_blur_shader_data(
			&MotionBlurPassSettings::default(),
			fraction,
			tiles,
			1,
			ShownDebugViews::default(),
		);
		for (name, value) in [
			("shutter_fraction", Value::F32(data.shutter_fraction)),
			("max_radius", Value::F32(data.max_radius)),
			("tiles", Value::Vec2U(data.tiles)),
			("noise_offset", Value::F32(data.noise_offset)),
			("taps", Value::U32(data.taps)),
		] {
			parameters.write(name, value).expect(
				"Failed to initialize motion blur parameters. The most likely cause is a changed production buffer layout.",
			);
		}
	}

	/// Links a checked-in motion blur shader.
	fn program(asset: &str) -> (besl::NodeReference, ExecutableProgram) {
		let linked = simple_compute::compile_test_program(asset_source(asset));
		let program = compile(linked.clone());
		(linked, program)
	}

	/// Runs the tile max over a `Velocity` image whose every row is `columns`, and returns each tile's longest blur.
	fn tile_max(velocity: &mut besl::vm::Texture, fraction: f32) -> Vec<[f32; 2]> {
		let (linked, program) = program("motion_blur/tile-max.besl");
		let slot = |name| binding_slot(&linked, name);
		let mut parameters = buffer(&program, slot("parameters"));
		write_parameters(&mut parameters, fraction);
		let mut tile_max = buffer(&program, slot("tile_max"));
		let tiles = [WIDTH.div_ceil(TILE_WIDTH), HEIGHT.div_ceil(TILE_WIDTH)];
		for tile_y in 0..tiles[1] {
			for tile_x in 0..tiles[0] {
				let width = TILE_MAX_WORKGROUP_WIDTH;
				let configs: Vec<_> = (0..width * width)
					.map(|lane| {
						lane_config(lane).with_thread_id([tile_x * width + lane % width, tile_y * width + lane / width])
					})
					.collect();
				let mut workgroup = WorkgroupState::new();
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(slot("velocity_texture"), velocity);
				descriptors.bind_buffer(slot("parameters"), &mut parameters);
				descriptors.bind_buffer(slot("tile_max"), &mut tile_max);
				descriptors.bind_workgroup_state(&mut workgroup);
				program
					.run_workgroup(&mut descriptors, &configs)
					.expect("The tile max workgroup runs in the VM.");
			}
		}
		(0..(tiles[0] * tiles[1]) as usize)
			.map(|index| match tile_max.read_array_element(index).expect("A tile max value") {
				Value::Vec2F(value) => value,
				value => panic!("Unexpected tile max value: {value:?}."),
			})
			.collect()
	}

	/// Blurs every pixel of an image whose every row is `columns`, through the three production stages, for a frame
	/// whose shutter covers `fraction` of the motion.
	fn blur(columns: &[Column], fraction: f32) -> Vec<[f32; 4]> {
		assert_eq!(columns.len(), WIDTH as usize);
		let row = |texel: &dyn Fn(&Column) -> [f32; 4]| -> Vec<[f32; 4]> {
			(0..HEIGHT).flat_map(|_| columns.iter().map(texel)).collect()
		};
		let mut scene = texture_2d(WIDTH, HEIGHT, &row(&|column| column.color));
		let mut velocity = texture_2d(
			WIDTH,
			HEIGHT,
			&row(&|column| [column.motion[0], column.motion[1], column.depth, column.depth]),
		);
		let tiles = tile_max(&mut velocity, fraction);

		let (linked, neighbor_program) = program("motion_blur/neighbor-max.besl");
		let neighbor_slot = |name| binding_slot(&linked, name);
		let mut parameters = buffer(&neighbor_program, neighbor_slot("parameters"));
		write_parameters(&mut parameters, fraction);
		let mut tile_max = buffer(&neighbor_program, neighbor_slot("tile_max"));
		for (index, value) in tiles.iter().enumerate() {
			tile_max
				.write_array_element(index, Value::Vec2F(*value))
				.expect("A tile max value");
		}
		let mut neighbor_max = buffer(&neighbor_program, neighbor_slot("neighbor_max"));
		for tile in 0..tiles.len() as u32 {
			let mut descriptors = DescriptorBindings::new();
			descriptors.bind_buffer(neighbor_slot("parameters"), &mut parameters);
			descriptors.bind_buffer(neighbor_slot("tile_max"), &mut tile_max);
			descriptors.bind_buffer(neighbor_slot("neighbor_max"), &mut neighbor_max);
			run_at(
				&neighbor_program,
				&mut descriptors,
				[tile % WIDTH.div_ceil(TILE_WIDTH), tile / WIDTH.div_ceil(TILE_WIDTH)],
			);
		}

		let (_, gather) = program("motion_blur/gather.besl");
		let mut gather_parameters = buffer(&gather, slot(&gather, "parameters"));
		write_parameters(&mut gather_parameters, fraction);
		let mut gather_neighbors = buffer(&gather, slot(&gather, "neighbor_max"));
		for index in 0..tiles.len() {
			let value = neighbor_max.read_array_element(index).expect("A neighbor max value");
			gather_neighbors
				.write_array_element(index, value)
				.expect("A neighbor max value");
		}
		let mut result = empty_image(WIDTH, HEIGHT);
		let mut debug_targets = empty_image(WIDTH, HEIGHT);
		for y in 0..HEIGHT {
			for x in 0..WIDTH {
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(slot(&gather, "scene_texture"), &mut scene);
				descriptors.bind_texture(slot(&gather, "velocity_texture"), &mut velocity);
				descriptors.bind_buffer(slot(&gather, "parameters"), &mut gather_parameters);
				descriptors.bind_buffer(slot(&gather, "neighbor_max"), &mut gather_neighbors);
				descriptors.bind_image(slot(&gather, "debug_view_targets"), &mut debug_targets);
				descriptors.bind_image(output_slot(0), &mut result);
				run_pixel(&gather, &mut descriptors, [x, y], PIXEL_TILE, [WIDTH, HEIGHT]);
			}
		}
		(0..WIDTH).map(|x| rgba(&result, [x, HEIGHT / 2])).collect()
	}

	/// The left half red and the right half blue, every column `depth` meters away and moving by `motion`.
	fn halves(motion: [f32; 2]) -> Vec<Column> {
		(0..WIDTH)
			.map(|x| Column {
				color: if x < WIDTH / 2 { RED } else { BLUE },
				motion,
				depth: 5.0,
			})
			.collect()
	}

	#[test]
	fn a_still_image_passes_through() {
		let blurred = blur(&halves([0.0, 0.0]), 1.0);
		for (x, color) in blurred.iter().enumerate() {
			assert_rgba_close(*color, halves([0.0, 0.0])[x].color, 1e-6);
		}
	}

	/// A camera pan moves every pixel alike, which the old matrix-only blur left sharp.
	#[test]
	fn a_pan_mixes_the_halves_across_their_edge() {
		let blurred = blur(&halves([16.0, 0.0]), 1.0);
		for x in [30, 31, 32, 33] {
			let [red, _, blue, alpha] = blurred[x];
			assert!(
				red > 0.05 && blue > 0.05 && (red + blue - 1.0).abs() < 1e-4 && alpha == 1.0,
				"{x}: {red} {blue}"
			);
		}
		// Farther from the edge than the blur reaches, only one color mixes.
		assert_rgba_close(blurred[8], RED, 1e-4);
		assert_rgba_close(blurred[56], BLUE, 1e-4);
	}

	#[test]
	fn a_closed_shutter_keeps_the_image_sharp() {
		let blurred = blur(&halves([16.0, 0.0]), 0.0);
		for (x, color) in blurred.iter().enumerate() {
			assert_rgba_close(*color, halves([0.0, 0.0])[x].color, 1e-6);
		}
	}

	/// A red object in front, moving right, over a still blue background, and the reverse.
	fn object_over_background(object_motion: [f32; 2], background_motion: [f32; 2]) -> Vec<Column> {
		(0..WIDTH)
			.map(|x| match x < WIDTH / 2 {
				true => Column {
					color: RED,
					motion: object_motion,
					depth: 2.0,
				},
				false => Column {
					color: BLUE,
					motion: background_motion,
					depth: 10.0,
				},
			})
			.collect()
	}

	/// A moving object blurs past its own silhouette over the still background behind it.
	#[test]
	fn a_moving_object_blurs_over_the_still_background() {
		let blurred = blur(&object_over_background([16.0, 0.0], [0.0, 0.0]), 1.0);
		let [red, _, blue, _] = blurred[33];
		assert!(red > 0.05 && blue > 0.05, "{red} {blue}");
		// Past the object's reach the background stays sharp.
		assert_rgba_close(blurred[50], BLUE, 1e-4);
	}

	/// A still object stays sharp in front of a background that moves behind it.
	#[test]
	fn a_still_object_stays_sharp_over_a_moving_background() {
		let blurred = blur(&object_over_background([0.0, 0.0], [16.0, 0.0]), 1.0);
		for x in [28, 30, 31] {
			assert_rgba_close(blurred[x], RED, 1e-3);
		}
	}

	/// A tile's blur is half its fastest pixel's motion over the shutter, held to the longest blur.
	#[test]
	fn tile_max_keeps_the_longest_blur_held_to_the_radius() {
		let mut columns = halves([0.0, 0.0]);
		columns[5].motion = [6.0, -2.0];
		columns[40].motion = [500.0, 0.0];
		let row: Vec<_> = (0..HEIGHT)
			.flat_map(|_| columns.iter().map(|column| [column.motion[0], column.motion[1], 1.0, 1.0]))
			.collect();
		let mut velocity = texture_2d(WIDTH, HEIGHT, &row);
		let tiles = tile_max(&mut velocity, 0.5);
		let close = |actual: [f32; 2], expected: [f32; 2]| {
			(actual[0] - expected[0]).abs() < 1e-4 && (actual[1] - expected[1]).abs() < 1e-4
		};
		assert!(close(tiles[0], [1.5, -0.5]), "{:?}", tiles[0]);
		assert!(close(tiles[1], [32.0, 0.0]), "{:?}", tiles[1]);
	}

	/// Lowers each motion blur program through the platform shader compiler. The VM tests prove they link and behave
	/// as BESL; defects the backend mishandles only surface when the real platform compiler runs.
	#[cfg(target_os = "macos")]
	#[compio::test]
	async fn motion_blur_besl_programs_lower_to_the_platform_shader_language() {
		use resource_management::shader::{ShaderGenerationSettings, besl::backends::platform::PlatformShaderCompiler};

		for (asset, settings) in [
			(
				"motion_blur/tile-max.besl",
				ShaderGenerationSettings::compute(Extent::new(TILE_MAX_WORKGROUP_WIDTH, TILE_MAX_WORKGROUP_WIDTH, 1)),
			),
			(
				"motion_blur/neighbor-max.besl",
				ShaderGenerationSettings::compute(Extent::new(8, 8, 1)),
			),
			(
				"motion_blur/gather.besl",
				ShaderGenerationSettings::tile(PIXEL_TILE, Extent::square(PIXEL_TILE)),
			),
		] {
			let mut root = crate::rendering::shader_vm_test::with_engine_modules(asset_source(asset));
			root.add(crate::rendering::common_shader_generator::common_shader_module().to_vec());
			let root = besl::lex(root).unwrap_or_else(|error| panic!("{asset} should link: {error:?}"));
			PlatformShaderCompiler::new()
				.generate(&settings.name("motion_blur".to_string()), &root)
				.await
				.unwrap_or_else(|error| panic!("{asset} should compile for the platform shader language: {error}"));
		}
	}
}
