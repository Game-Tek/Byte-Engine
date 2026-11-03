//! Depth of field: blurs what lies away from the camera's focus distance, as a thin lens with an open aperture
//! would.
//!
//! The pass follows the lens each [`Sink`] carries from its camera. Every pixel's circle of confusion, the signed
//! diameter of the disc the lens spreads the pixel's surface over, decides how far its color spreads, and one
//! continuous gather adds up the discs that cover each pixel. See [`DepthOfFieldPass`].
//!
//! [`DepthOfFieldReferencePass`] renders the same lens exactly but slowly, by averaging frames seen from points
//! spread over the aperture, to compare the blur against.

use ghi::{command_buffer::CommonCommandBufferMode as _, context::ContextCreate as _, frame::Frame as _};
use utils::Extent;

use crate::{
	core::Entity,
	rendering::{
		DebugViews, ShownDebugViews, Sink, View,
		debug_views::DEBUG_VIEW_TARGETS_BINDING,
		render_pass::{RenderPass, RenderPassBuilder, RenderPassReturn, simple_compute, simple_tile},
		render_passes::blit::ImageBypassPass,
	},
};

/// How many frames the reference averages evenly before each new frame takes a fixed `1 / REFERENCE_WINDOW` share.
const REFERENCE_WINDOW: u32 = 1024;

/// The debug views the pass declares, in the order `prepare.besl` numbers them from `0`.
const DEPTH_OF_FIELD_DEBUG_VIEWS: [&str; 1] = ["circle-of-confusion"];

/// The side of the square tiles the prepare and gather passes work in, which is also their workgroups' side.
const TILE_WIDTH: u32 = 16;
/// The most tiles a sink holds, which sizes the tile reach buffer: about 16.7 million pixels.
const MAX_TILES: u32 = 65_536;
/// The widest blur diameter, in pixels. The gather's last ring ends past half of it, so a wider blur would outreach the
/// taps.
const MAX_BLUR_DIAMETER: f32 = 64.0;
/// The averaged levels of the source image after the full-resolution one, each half as wide as the one before.
const LEVEL_COUNT: usize = 3;

/// The `DepthOfFieldShaderData` struct mirrors the `DepthOfFieldParameters` layout of `layouts/parameters.besl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DepthOfFieldShaderData {
	/// The circle of confusion at reverse-Z depth 0 and its change per unit of depth, in pixels; see
	/// [`circle_of_confusion`].
	circle_of_confusion: [f32; 2],
	/// The smallest and largest blur diameters, in pixels; see [`blur_limits`].
	blur_limits: [f32; 2],
	/// How many tiles make one row of the tile reach buffer.
	tiles_per_row: u32,
	/// The pool slot the first shown debug view writes; see [`ShownDebugViews`].
	debug_view_first_slot: u32,
	/// The shader's `bool` members, one bit per view of [`DEPTH_OF_FIELD_DEBUG_VIEWS`].
	debug_views: u8,
	_padding: [u8; 3],
	_padding_1: u32,
}

/// The `DepthOfFieldPassSettings` struct bounds how much the depth-of-field pass blurs, whatever the lens asks for.
///
/// Pass it to [`crate::application::graphics::setup_depth_of_field_render_pass`]. Both bounds are blur diameters in
/// pixels, and both are unset by default.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DepthOfFieldPassSettings {
	/// The smallest blur, from `0` to the largest. Every pixel blurs at least this much, even in focus: `2.0` softens
	/// the whole picture by about one pixel around each pixel. Unset, in-focus pixels stay sharp.
	pub min_blur_diameter: Option<f32>,
	/// The largest blur, from `1` to `64`. The gather's cost grows with the widest blur on screen, so set this to keep
	/// a wide aperture or a close focus from costing more than your frame allows. Unset, blurs reach up to 64 pixels.
	pub max_blur_diameter: Option<f32>,
}

/// The `DepthOfFieldPass` struct blurs every sink by its camera's lens.
///
/// Install it with [`crate::application::graphics::setup_depth_of_field_render_pass`] after the passes that write
/// scene light and the fog, and before motion blur, lens flare, bloom, and tone mapping. Set the lens on the camera
/// with [`crate::rendering::Camera::set_f_number`] and [`crate::rendering::Camera::set_focus_distance`]; a pinhole, the
/// default, leaves the scene unchanged. The pass reads the opaque scene depth, so transparent surfaces and particles
/// take the focus of the surface behind them. Show the blur diameters with the
/// `render.debug.view.depth-of-field.circle-of-confusion` parameter.
pub struct DepthOfFieldPass {
	parameters: ghi::DynamicBufferHandle<DepthOfFieldShaderData>,
	/// Writes the source and its averaged levels.
	prepare_pass: simple_compute::Pass,
	gather_pass: simple_compute::Pass,
	/// Removes the grain the gather's noise-placed rings leave and writes the pass's output.
	filter_pass: simple_compute::Pass,
	/// Forwards `main` unchanged for a pinhole lens or while a shader is still compiling.
	main_copy: ImageBypassPass,
	/// Which debug views to write this frame.
	debug_views: DebugViews,
	/// The smallest and largest blur diameters, in pixels; see [`blur_limits`].
	blur_limits: [f32; 2],
	/// The GPU counters of the prepare, gather, and filter stages, reported as `stage.depth-of-field-<stage>`.
	counters: [ghi::CounterHandle; 3],
}

impl Entity for DepthOfFieldPass {}

impl DepthOfFieldPass {
	/// Creates a depth of field pass that reads the sink's scene and depth, blurs it within `settings`' bounds, and
	/// remaps `main` for downstream passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder, settings: DepthOfFieldPassSettings) -> Self {
		let blur_limits = blur_limits(settings);
		if Some(blur_limits[0]) != settings.min_blur_diameter.or(Some(0.0))
			|| Some(blur_limits[1]) != settings.max_blur_diameter.or(Some(MAX_BLUR_DIAMETER))
		{
			log::warn!(
				"Depth of field blur bounds were changed to {} and {} pixels. The most likely cause is a maximum blur diameter outside 1 to 64 pixels, or a minimum outside 0 to the maximum. Set `DepthOfFieldPassSettings` within those ranges.",
				blur_limits[0],
				blur_limits[1]
			);
		}
		let scene = render_pass_builder.read_from("main");
		let depth = render_pass_builder.read_from("depth");
		let main_format = render_pass_builder.format_of("main");
		// Each pixel's color and blur diameter, then the same averaged over 2, 4, and 8 pixels.
		let levels: [ghi::BaseImageHandle; LEVEL_COUNT + 1] = std::array::from_fn(|level| {
			let builder = ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage | ghi::Uses::Image).name(
				[
					"Depth of Field Source",
					"Depth of Field Level 1",
					"Depth of Field Level 2",
					"Depth of Field Level 3",
				][level],
			);
			render_pass_builder.create_scaled_render_target(builder, 1 << level).into()
		});
		// The gather's blur, with the share of its weight the rings carried in alpha, for the filter to finish.
		let gathered: ghi::BaseImageHandle = render_pass_builder
			.create_scaled_render_target(
				ghi::image::Builder::new(ghi::Formats::RGBA16F, ghi::Uses::Storage | ghi::Uses::Image)
					.name("Depth of Field Gathered"),
				1,
			)
			.into();
		let output = render_pass_builder
			.create_main_render_target(ghi::image::Builder::new(main_format, ghi::Uses::Storage).name("Depth of Field"));
		// The builder's copy of the incoming `main` into `output` doubles as this pass's forwarding copy.
		let main_copy = render_pass_builder.take_main_copy().expect(
			"Main copy is missing. The most likely cause is that the pass did not call `RenderPassBuilder::create_main_render_target` first.",
		);
		let pipeline = |render_pass_builder: &mut RenderPassBuilder, label, id| {
			simple_compute::Pipeline::compile(render_pass_builder, simple_compute::Descriptor::new(label, id))
		};
		let prepare = pipeline(
			render_pass_builder,
			"Depth of Field Prepare",
			"byte_engine/rendering/depth_of_field/prepare.pipeline",
		);
		let gather = pipeline(
			render_pass_builder,
			"Depth of Field Gather",
			"byte_engine/rendering/depth_of_field/gather.pipeline",
		);
		let filter = pipeline(
			render_pass_builder,
			"Depth of Field Filter",
			"byte_engine/rendering/depth_of_field/filter.pipeline",
		);
		let debug_views = render_pass_builder.debug_views(&DEPTH_OF_FIELD_DEBUG_VIEWS);
		let counters = ["depth-of-field-prepare", "depth-of-field-gather", "depth-of-field-filter"]
			.map(|name| render_pass_builder.create_gpu_counter(name));

		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Depth of Field Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let tile_reaches = context.build_buffer::<[f32]>(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Depth of Field Tile Reaches")
				.length(MAX_TILES as usize)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly),
		);
		// The prepare pass fetches texels by coordinate; the gather's far taps blend between texels, and
		// the clamp keeps taps past the image's edge on its edge texels.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let blue_noise = crate::rendering::lookup_textures::create_blue_noise_texture(context);
		let read = |name, image| simple_compute::Resource::combined_image_sampler(name, image, sampler, ghi::Layouts::Read);

		let prepare_pass = prepare.bind(
			"Depth of Field Prepare Descriptor Set",
			&[
				read("scene_texture", scene.into()),
				read("depth_texture", depth.into()),
				simple_compute::Resource::buffer("parameters", parameters),
				simple_compute::Resource::image("source", levels[0]),
				simple_compute::Resource::buffer("tile_reaches", tile_reaches),
				simple_compute::Resource::image_array(DEBUG_VIEW_TARGETS_BINDING, debug_views.targets()),
				simple_compute::Resource::image("level_1", levels[1]),
				simple_compute::Resource::image("level_2", levels[2]),
				simple_compute::Resource::image("level_3", levels[3]),
			],
		);
		let gather_pass = gather.bind(
			"Depth of Field Gather Descriptor Set",
			&[
				read("source_level_0", levels[0]),
				read("source_level_1", levels[1]),
				read("source_level_2", levels[2]),
				read("source_level_3", levels[3]),
				simple_compute::Resource::buffer("parameters", parameters),
				simple_compute::Resource::buffer("tile_reaches", tile_reaches),
				simple_compute::Resource::image("result", gathered),
				read("blue_noise", blue_noise.into()),
			],
		);
		let filter_pass = filter.bind(
			"Depth of Field Filter Descriptor Set",
			&[read("gathered", gathered), simple_compute::Resource::image("result", output)],
		);

		Self {
			parameters,
			prepare_pass,
			gather_pass,
			filter_pass,
			main_copy,
			debug_views,
			blur_limits,
			counters,
		}
	}
}

impl RenderPass for DepthOfFieldPass {
	fn name(&self) -> &'static str {
		"depth-of-field"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let circle_of_confusion = circle_of_confusion(sink);
		let shown: ShownDebugViews = self.debug_views.shown(0..DEPTH_OF_FIELD_DEBUG_VIEWS.len());
		let extent = sink.extent();
		let tiles_per_row = extent.width().div_ceil(TILE_WIDTH);
		let tiles = tiles_per_row * extent.height().div_ceil(TILE_WIDTH);
		if tiles > MAX_TILES {
			log::error!(
				"Depth of field was skipped. The most likely cause is a window of {} by {} pixels, more than the {MAX_TILES} tiles of {TILE_WIDTH} pixels the pass holds. Make the window smaller or raise `MAX_TILES`.",
				extent.width(),
				extent.height()
			);
			return self.main_copy.prepare(frame, sink, frame_allocator);
		}
		// A pinhole blurs nothing without a smallest blur, so `main` passes through unless the blur view needs the
		// prepare pass.
		if circle_of_confusion == [0.0, 0.0] && self.blur_limits[0] == 0.0 && shown.views == 0 {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		}
		let (Some(prepare), Some(gather), Some(filter)) = (
			self.prepare_pass.ready(frame),
			self.gather_pass.ready(frame),
			self.filter_pass.ready(frame),
		) else {
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = DepthOfFieldShaderData {
			circle_of_confusion,
			blur_limits: self.blur_limits,
			tiles_per_row,
			debug_view_first_slot: shown.first_slot,
			debug_views: shown.views,
			_padding: [0; 3],
			_padding_1: 0,
		};
		let [prepare_counter, gather_counter, filter_counter] = self.counters;
		Some(crate::rendering::render_pass::allocate_render_command(
			frame_allocator,
			move |command_buffer| {
				command_buffer.region(
					|label| label.write_str("Depth of Field"),
					|command_buffer| {
						command_buffer.counter(prepare_counter, |command_buffer| prepare.record(command_buffer, extent));
						command_buffer.counter(gather_counter, |command_buffer| gather.record(command_buffer, extent));
						command_buffer.counter(filter_counter, |command_buffer| filter.record(command_buffer, extent));
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
		// This pass took the builder's forwarding copy, so it forwards `main` itself.
		self.main_copy.prepare(frame, sink, frame_allocator)
	}
}

/// Returns the smallest and largest blur diameters `settings` ask for, in pixels: the largest held to 1 to 64 pixels,
/// 64 when unset, and the smallest held to 0 to the largest, 0 when unset. A NaN bound counts as unset.
fn blur_limits(settings: DepthOfFieldPassSettings) -> [f32; 2] {
	let largest = settings
		.max_blur_diameter
		.filter(|largest| !largest.is_nan())
		.map_or(MAX_BLUR_DIAMETER, |largest| largest.clamp(1.0, MAX_BLUR_DIAMETER));
	let smallest = settings
		.min_blur_diameter
		.filter(|smallest| !smallest.is_nan())
		.map_or(0.0, |smallest| smallest.clamp(0.0, largest));
	[smallest, largest]
}

/// Returns the sink's circle of confusion, in pixels, as an affine function of reverse-Z depth `d`: the value at
/// `d = 0` and its change per unit of `d`.
///
/// A thin lens with focal length `F`, aperture `A`, and focus distance `S` spreads a point at depth `z` over a disc of
/// diameter `A · F / (S − F) · (1 − S / z)` on the sensor. Reverse-Z depth is `d = α + β / z`, so `1 / z` and with it
/// the circle of confusion are affine in `d`. The focal length follows from the sensor height and the vertical field
/// of view, and the sensor-to-pixel scale from the sink's height.
fn circle_of_confusion(sink: &Sink) -> [f32; 2] {
	let [focal_length, aperture, focus_distance] = sink.thin_lens();
	let [_, _, sensor_height] = sink.lens();
	// The circle of confusion of a point at infinity, in pixels; a pinhole's infinite f-number makes it 0.
	let at_infinity = aperture * focal_length / (focus_distance - focal_length) * sink.extent().height() as f32 / sensor_height;
	let view = sink.view();
	let (near, far) = (view.near(), view.far());
	[
		at_infinity * (1.0 - focus_distance / far),
		-at_infinity * focus_distance * (far - near) / (near * far),
	]
}

/// The `DepthOfFieldReferenceShaderData` struct mirrors the `DepthOfFieldReferenceParameters` layout of
/// `reference.besl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DepthOfFieldReferenceShaderData {
	/// This frame's share of the mean. `1` starts the mean over.
	weight: f32,
}

/// The `DepthOfFieldReferencePass` struct shows the exact blur of each sink's camera lens, to judge
/// [`DepthOfFieldPass`] against.
///
/// While it is enabled, the renderer moves the sink's view to a new point of the aperture every frame, and the pass
/// replaces `main` with the mean of the frames since the camera, its lens, or the sink's extent last changed. Past
/// 1,024 frames the mean becomes an exponential average over about as many, so changes to the scene, such as meshes
/// streaming in, fade in. The mean is only right for a still scene: moving objects smear. Install it with
/// [`crate::application::graphics::setup_depth_of_field_reference_render_pass`], which leaves it bypassed, and enable
/// it with `render.pass.depth-of-field-reference=enabled`. Bypass motion blur while it runs, since it blurs the moves.
pub struct DepthOfFieldReferencePass {
	parameters: ghi::DynamicBufferHandle<DepthOfFieldReferenceShaderData>,
	accumulate_pass: simple_tile::Pass,
	/// Forwards `main` unchanged while the pass is bypassed or its shader is still compiling.
	main_copy: ImageBypassPass,
	/// The camera view, lens, and extent the mean belongs to, and how many frames it holds. `None` while the pass
	/// draws nothing, which also tells on-demand rendering it can stop.
	mean: Option<((View, [f32; 3], Extent), u32)>,
}

impl Entity for DepthOfFieldReferencePass {}

impl DepthOfFieldReferencePass {
	/// Creates a reference pass that averages `main` into persistent targets and remaps `main` for downstream
	/// passes.
	pub fn new(render_pass_builder: &mut RenderPassBuilder) -> Self {
		let source = render_pass_builder.read_from("main");
		let main_format = render_pass_builder.format_of("main");
		// One 32-bit image per channel, since a half-float mean stops moving once a frame's share falls below its
		// precision, and the image formats have no 32-bit RGBA.
		let [red, green, blue] = [
			"Depth of Field Reference Red",
			"Depth of Field Reference Green",
			"Depth of Field Reference Blue",
		]
		.map(|name| {
			render_pass_builder
				.create_persistent_render_target(ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage).name(name))
		});
		let output = render_pass_builder.create_main_render_target(
			ghi::image::Builder::new(main_format, ghi::Uses::Image).name("Depth of Field Reference"),
		);
		// The builder's copy of the incoming `main` into `output` doubles as this pass's forwarding copy.
		let main_copy = render_pass_builder.take_main_copy().expect(
			"Main copy is missing. The most likely cause is that the pass did not call `RenderPassBuilder::create_main_render_target` first.",
		);
		let pipeline = simple_tile::Pipeline::compile(
			render_pass_builder,
			simple_tile::Descriptor::new(
				"Depth of Field Reference",
				"byte_engine/rendering/depth_of_field/reference.pipeline",
				&[output.format()],
			),
		);
		let context = render_pass_builder.context();
		let parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Depth of Field Reference Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		// The shader fetches texels by coordinate, so the sampler only completes the texture binding.
		let sampler = context.build_sampler(ghi::sampler::Builder::new());
		let accumulate_pass = pipeline.bind(
			"Depth of Field Reference Descriptor Set",
			&[
				simple_tile::Resource::combined_image_sampler("scene_texture", source, sampler, ghi::Layouts::Read),
				simple_tile::Resource::image("mean_red", red),
				simple_tile::Resource::image("mean_green", green),
				simple_tile::Resource::image("mean_blue", blue),
				simple_tile::Resource::buffer("parameters", parameters),
			],
			&[simple_tile::Output::new("result", output)],
		);

		Self {
			parameters,
			accumulate_pass,
			main_copy,
			mean: None,
		}
	}
}

impl RenderPass for DepthOfFieldReferencePass {
	fn name(&self) -> &'static str {
		"depth-of-field-reference"
	}

	fn prepare<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		let Some(command) = self.accumulate_pass.prepare(frame, sink, frame_allocator) else {
			self.mean = None;
			return self.main_copy.prepare(frame, sink, frame_allocator);
		};
		// The mean starts over whenever what it averages changes. The n-th frame since then takes 1 / n of it, which
		// averages the frames evenly, until the window, from which on each takes a fixed share.
		let key = (sink.camera_view(), sink.lens(), sink.extent());
		let count = self
			.mean
			.filter(|(mean_key, _)| *mean_key == key)
			.map_or(0, |(_, count)| count);
		let frames = (count + 1).min(REFERENCE_WINDOW);
		*frame.get_mut_dynamic_buffer_slice(self.parameters) = DepthOfFieldReferenceShaderData {
			weight: 1.0 / frames as f32,
		};
		self.mean = Some((key, frames));
		Some(command)
	}

	fn bypass<'a>(
		&mut self,
		frame: &mut ghi::implementation::Frame,
		sink: &Sink,
		frame_allocator: &'a bumpalo::Bump,
	) -> Option<RenderPassReturn<'a>> {
		// Re-enabling starts a new mean. This pass took the builder's forwarding copy, so it forwards `main` itself.
		self.mean = None;
		self.main_copy.prepare(frame, sink, frame_allocator)
	}

	fn needs_frame(&mut self) -> bool {
		self.mean.is_some()
	}

	fn samples_lens(&self) -> bool {
		true
	}
}

#[cfg(test)]
mod tests {
	use math::{Degrees, Point, UnitVector};
	use maths_rs::Vec4f;
	use utils::Extent;

	use super::*;
	use crate::rendering::View;
	use crate::rendering::shader_vm_test::asset_source;
	const REFERENCE_BESL: &str = include_str!("../../../assets/rendering/depth_of_field/reference.besl");

	/// A 1080-pixel-tall sink whose camera has a 50 mm lens on a full frame sensor.
	fn sink(f_number: f32, focus_distance: f32) -> Sink {
		// A 24 mm sensor behind a 50 mm lens sees 2 · atan(12 / 50) vertically.
		let fov = Degrees::new(2.0 * (12.0f32 / 50.0).atan().to_degrees());
		let view = View::new_perspective(fov, 16.0 / 9.0, 0.1, 100.0, Point::origin(), UnitVector::z_axis());
		Sink::new(view, Extent::rectangle(1920, 1080), 0).with_lens(f_number, focus_distance, 0.024)
	}

	/// The circle of confusion the pass computes for a point `z` meters in front of `sink`'s camera.
	fn pass_circle_of_confusion(sink: &Sink, z: f32) -> f32 {
		let clip = sink.view().projection() * Vec4f::new(0.0, 0.0, z, 1.0);
		let [bias, scale] = circle_of_confusion(sink);
		bias + scale * clip.z / clip.w
	}

	/// The thin-lens circle of confusion of the Reference page, in pixels of a 1080-pixel-tall image.
	fn thin_lens(focal_length: f32, f_number: f32, focus_distance: f32, z: f32) -> f32 {
		focal_length / f_number * focal_length / (focus_distance - focal_length) * (1.0 - focus_distance / z) * 1080.0 / 0.024
	}

	#[test]
	fn circle_of_confusion_follows_the_thin_lens() {
		let sink = sink(1.8, 2.0);
		for z in [0.1, 0.5, 1.0, 2.0, 4.0, 30.0, 100.0] {
			let expected = thin_lens(0.05, 1.8, 2.0, z);
			let actual = pass_circle_of_confusion(&sink, z);
			assert!(
				(actual - expected).abs() <= 1e-3 * expected.abs().max(1.0),
				"z {z}: {actual}, expected {expected}"
			);
		}
	}

	#[test]
	fn circle_of_confusion_is_signed_by_side_of_the_focus_distance() {
		let sink = sink(1.8, 2.0);
		assert!(pass_circle_of_confusion(&sink, 1.0) < 0.0);
		assert!(pass_circle_of_confusion(&sink, 2.0).abs() < 1e-3);
		assert!(pass_circle_of_confusion(&sink, 4.0) > 0.0);
	}

	#[test]
	fn a_pinhole_keeps_every_depth_in_focus() {
		assert_eq!(circle_of_confusion(&sink(f32::INFINITY, 2.0)), [0.0, 0.0]);
	}

	#[test]
	fn focus_nearer_than_the_focal_length_is_held_beyond_it() {
		// The 50 mm lens focuses no nearer than 51 mm, so a point there is in focus and the blur stays finite.
		let sink = sink(1.8, 0.01);
		let [bias, scale] = circle_of_confusion(&sink);
		assert!(bias.is_finite() && scale.is_finite());
		assert!(pass_circle_of_confusion(&sink, 0.051).abs() < 1e-2);
	}

	/// Reads the constant array `name` of `gather.besl` as numbers.
	fn gather_table(name: &str) -> Vec<f32> {
		let source = asset_source("depth_of_field/gather.besl");
		let start = source
			.find(&format!("{name}: const"))
			.expect("The gather declares the table.");
		let body = &source[start..];
		let body = &body[body.find('(').expect("The table has values.") + 1..body.find(");").expect("The table ends.")];
		body.split(',')
			.map(|value| value.trim().parse().expect("Every entry is a number."))
			.collect()
	}

	/// The inner taps are every pixel within 3.5 pixels, at their place in the 22-texel-wide shared tile, nearest first,
	/// and each group of them is skipped only when its nearest tap lies beyond the search reach.
	#[test]
	fn inner_taps_are_the_pixels_within_their_radius_nearest_first() {
		let mut expected = (-3i32..=3)
			.flat_map(|y| (-3i32..=3).map(move |x| (((x * x + y * y) as f32).sqrt(), (y + 3) * 22 + x + 3)))
			.filter(|(distance, _)| *distance <= 3.5)
			.collect::<Vec<_>>();
		expected.sort_by(|a, b| a.0.total_cmp(&b.0));
		let gather = asset_source("depth_of_field/gather.besl");
		let main = &gather[gather.find("let center: f32").expect("The gather reads the pixel's blur.")..];
		let main = &main[..main.find("// The rings beyond").expect("The rings follow the inner taps.")];
		// Each group, after the first, opens with the distance past which the search reach must extend.
		let mut taps = Vec::new();
		for (group, body) in main.split("if (search > ").enumerate() {
			let threshold: f32 = if group == 0 {
				0.0
			} else {
				body[..body.find(')').expect("The group's test closes.")]
					.parse()
					.expect("The group tests a distance.")
			};
			// Each tap reads its texel as `texel_<n>`, then weighs it at its distance with `inner_tap(texel_<n>, d, center)`.
			let mut texels = std::collections::HashMap::new();
			let mut group_taps = Vec::new();
			for line in body.lines().map(str::trim) {
				if let Some(rest) = line.strip_prefix("let texel_") {
					let name = &rest[..rest.find(':').expect("The read declares its type.")];
					let start = rest.find("window + ").expect("An inner tap reads the shared tile.") + "window + ".len();
					let texel: i32 = rest[start..rest.find(']').expect("The read closes.")]
						.parse()
						.expect("The texel is a number.");
					texels.insert(name.to_string(), texel);
				}
				if let Some(rest) = line.strip_prefix("sum = sum + inner_tap(texel_") {
					let mut arguments = rest.split(", ");
					let name = arguments.next().expect("The tap names its read.");
					let distance: f32 = arguments
						.next()
						.expect("The tap gives its distance.")
						.parse()
						.expect("The distance is a number.");
					group_taps.push((distance, texels[name]));
				}
			}
			assert!(!group_taps.is_empty(), "group {group} has taps");
			let nearest = group_taps.iter().map(|tap| tap.0).fold(f32::INFINITY, f32::min);
			assert!(
				(nearest - threshold).abs() < 1e-5,
				"group {group} is skipped by its nearest tap, {nearest}"
			);
			taps.extend(group_taps);
		}
		assert_eq!(taps.len(), expected.len());
		let mut previous = 0.0;
		for (distance, texel) in &taps {
			let (expected_distance, _) = expected
				.iter()
				.find(|(_, expected_texel)| expected_texel == texel)
				.unwrap_or_else(|| panic!("texel {texel} lies within 3.5 pixels"));
			assert!((distance - expected_distance).abs() < 1e-5, "texel {texel}: {distance}");
			assert!(*distance >= previous, "texel {texel} comes after a farther tap");
			previous = *distance;
		}
		let mut texels: Vec<_> = taps.iter().map(|tap| tap.1).collect();
		texels.sort();
		texels.dedup();
		assert_eq!(texels.len(), expected.len(), "every tap reads its own texel");
	}

	/// The rings grow by 1.39 times from 3.5 pixels past the widest reach, each tap stands for a sixteenth of its ring
	/// at the radius that halves the ring's area, and each ring reads the widest level no wider than its taps' spacing.
	#[test]
	fn outer_rings_cover_every_reach_with_even_areas() {
		let (inner, radii, areas) = (
			gather_table("RING_INNER_RADIUS"),
			gather_table("RING_RADIUS"),
			gather_table("RING_AREA"),
		);
		let (xs, ys) = (gather_table("OUTER_TAP_X"), gather_table("OUTER_TAP_Y"));
		let gather = asset_source("depth_of_field/gather.besl");
		for (ring, ((&low, &radius), &area)) in inner.iter().zip(&radii).zip(&areas).enumerate() {
			let high = low * 1.39;
			assert!((low - 3.5 * 1.39f32.powi(ring as i32)).abs() < 1e-3, "ring {ring}");
			assert!(
				(radius - ((low * low + high * high) * 0.5).sqrt()).abs() < 1e-3,
				"ring {ring}"
			);
			assert!(
				(area * 16.0 - std::f32::consts::PI * (high * high - low * low)).abs() < 1e-2,
				"ring {ring}"
			);
			for tap in ring * 16..ring * 16 + 16 {
				assert!((xs[tap].hypot(ys[tap]) - radius).abs() < 1e-3, "tap {tap}");
			}
			let level = (std::f32::consts::TAU * radius / 16.0).log2().floor().clamp(0.0, 3.0) as u32;
			assert!(
				gather.contains(&format!("gather_ring(source_level_{level}, {ring},")),
				"ring {ring} reads level {level}"
			);
		}
		// The widest blur is under 64 pixels across, so no reach passes 32.5 pixels.
		assert!(inner[inner.len() - 1] * 1.39 > 32.5);
	}

	/// Runs the layer's `blur_diameter` on each circle of confusion with `limits` and returns the blur diameters.
	fn shader_blur_diameters(circles: &[f32], limits: [f32; 2]) -> Vec<f32> {
		use besl::vm::{DescriptorBindings, ResourceSlot, Value};

		use crate::rendering::shader_vm_test::{buffer, compile, run_at, with_engine_modules};

		let fields: String = (0..circles.len()).map(|index| format!("\tb{index}: f32,\n")).collect();
		let calls: String = circles
			.iter()
			.enumerate()
			.map(|(index, circle)| {
				format!(
					"\tresults.b{index} = blur_diameter({circle:?}, vec2f({:?}, {:?}));\n",
					limits[0], limits[1]
				)
			})
			.collect();
		let stand_in = format!(
			"use ::byte_engine::rendering::depth_of_field::layouts::parameters::*;\nBlurResults: struct {{\n{fields}}}\nresults: descriptor<{{ type: BlurResults, binding: 900, access: read_write }}>;\nmain: fn () -> void {{\n{calls}}}\n"
		);
		let program = compile(besl::lex(with_engine_modules(stand_in.leak())).expect("The blur diameter stand-in links."));
		let slot = ResourceSlot::new(900);
		let mut results = buffer(&program, slot);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_buffer(slot, &mut results);
		run_at(&program, &mut descriptors, [0, 0]);
		drop(descriptors);
		(0..circles.len())
			.map(
				|index| match results.read(&format!("b{index}")).expect("Every result is written.") {
					Value::F32(value) => value,
					value => panic!("b{index} is {value:?}, not an f32"),
				},
			)
			.collect()
	}

	/// The blur diameter the reference defines for circle of confusion `c` between `limits`.
	fn reference_blur_diameter(c: f32, [smallest, largest]: [f32; 2]) -> f32 {
		let knee = largest / 2.0;
		let held = if c.abs() <= knee {
			c.abs()
		} else {
			knee + knee * ((c.abs() - knee) / knee).tanh()
		};
		held.max(smallest).copysign(if c == 0.0 { 1.0 } else { c })
	}

	/// The shader holds blur diameters between the smallest and largest as the reference defines them: unchanged up to
	/// half the largest, rolling off below the largest, and raised to the smallest, in front of the focus and behind it.
	#[test]
	fn blur_diameters_stay_within_their_bounds() {
		let circles = [0.0, -0.5, 1.5, -6.0, 12.0, 31.0, -40.0, 200.0, -1.0e9];
		for settings in [
			DepthOfFieldPassSettings::default(),
			DepthOfFieldPassSettings {
				min_blur_diameter: Some(2.0),
				max_blur_diameter: None,
			},
			DepthOfFieldPassSettings {
				min_blur_diameter: Some(2.0),
				max_blur_diameter: Some(16.0),
			},
		] {
			let limits = blur_limits(settings);
			for (circle, blur) in circles.iter().zip(shader_blur_diameters(&circles, limits)) {
				let expected = reference_blur_diameter(*circle, limits);
				assert!(
					(blur - expected).abs() <= 1e-4 * expected.abs().max(1.0),
					"{settings:?}, c {circle}: {blur}, expected {expected}"
				);
				assert!(
					blur.abs() >= limits[0] && blur.abs() <= limits[1],
					"{settings:?}, c {circle}: {blur}"
				);
			}
		}
	}

	/// Unset bounds blur from 0 to 64 pixels, and bounds outside their ranges are held to them.
	#[test]
	fn blur_bounds_are_held_to_their_ranges() {
		let limits = |min_blur_diameter, max_blur_diameter| {
			blur_limits(DepthOfFieldPassSettings {
				min_blur_diameter,
				max_blur_diameter,
			})
		};
		assert_eq!(limits(None, None), [0.0, 64.0]);
		assert_eq!(limits(Some(2.0), Some(16.0)), [2.0, 16.0]);
		assert_eq!(limits(None, Some(500.0)), [0.0, 64.0]);
		assert_eq!(limits(None, Some(0.25)), [0.0, 1.0]);
		assert_eq!(limits(Some(-1.0), None), [0.0, 64.0]);
		assert_eq!(limits(Some(20.0), Some(8.0)), [8.0, 8.0]);
		assert_eq!(limits(Some(f32::NAN), Some(f32::NAN)), [0.0, 64.0]);
	}

	/// The parameters declare one `bool` member per debug view, consecutive and in the order the Rust list declares
	/// them.
	#[test]
	fn debug_views_are_declared_in_order() {
		let members = DEPTH_OF_FIELD_DEBUG_VIEWS
			.iter()
			.map(|view| format!("\tdebug_view_{}: bool,\n", view.replace('-', "_")))
			.collect::<String>();

		let parameters = include_str!("../../../assets/rendering/depth_of_field/layouts/parameters.besl");
		assert!(parameters.contains(&members), "{members}");
	}

	#[test]
	fn depth_of_field_shaders_link() {
		for shader in ["prepare", "gather", "filter"] {
			simple_compute::compile_test_program(asset_source(&format!("depth_of_field/{shader}.besl")));
		}
	}

	/// Averages 2 × 2 texels as each level's texel width `width` asks: each by min(|b| / w, 1), evenly where all are 0.
	fn reference_average(texels: [[f32; 4]; 4], width: f32) -> [f32; 4] {
		let weights = texels.map(|texel| (texel[3].abs() / width).min(1.0));
		let total: f32 = weights.iter().sum();
		std::array::from_fn(|channel| {
			if total <= 0.0 {
				texels.iter().map(|texel| texel[channel]).sum::<f32>() * 0.25
			} else {
				texels
					.iter()
					.zip(weights)
					.map(|(texel, weight)| texel[channel] * weight)
					.sum::<f32>() / total
			}
		})
	}

	/// The prepare pass's averaged levels follow the averaging rule over the half-float source, level by level, also
	/// over an image whose tiles and levels end partway.
	#[test]
	fn prepare_averages_each_level_from_the_one_before() {
		use besl::vm::{DescriptorBindings, Value, WorkgroupState};

		use crate::rendering::shader_vm_test::{binding_slot, buffer, compile, empty_image, lane_config, rgba, texture_2d};

		let [width, height] = [40u32, 24u32];
		let linked = simple_compute::compile_test_program(asset_source("depth_of_field/prepare.besl"));
		let slot = |name| binding_slot(&linked, name);
		let program = compile(linked.clone());
		// Colors and depths that vary per pixel, so blur diameters cover sharp, mid, and wide, on both sides.
		let texel = |x: u32, y: u32| {
			[
				(x * 7 % 11) as f32 * 0.37,
				(y * 5 % 13) as f32 * 0.21,
				((x + y) % 7) as f32 * 1.3,
				1.0,
			]
		};
		let depth = |x: u32, y: u32| [((x * 13 + y * 29) % 31) as f32 / 30.0, 0.0, 0.0, 1.0];
		let colors: Vec<_> = (0..height).flat_map(|y| (0..width).map(move |x| texel(x, y))).collect();
		let depths: Vec<_> = (0..height).flat_map(|y| (0..width).map(move |x| depth(x, y))).collect();
		let mut scene = texture_2d(width, height, &colors);
		let mut depth_texture = texture_2d(width, height, &depths);
		let mut parameters = buffer(&program, slot("parameters"));
		for (member, value) in [
			("circle_of_confusion", Value::Vec2F([-24.0, 48.0])),
			("blur_limits", Value::Vec2F([0.0, 64.0])),
			("tiles_per_row", Value::U32(width.div_ceil(TILE_WIDTH))),
		] {
			parameters
				.write(member, value)
				.expect("The prepare parameters hold the member.");
		}
		let mut reaches = buffer(&program, slot("tile_reaches"));
		let mut source = empty_image(width, height);
		let mut levels = [1u32, 2, 3].map(|level| empty_image(width >> level, height >> level));
		let mut debug_target = empty_image(width, height);
		for tile_y in 0..height.div_ceil(TILE_WIDTH) {
			for tile_x in 0..width.div_ceil(TILE_WIDTH) {
				let configs: Vec<_> = (0..TILE_WIDTH * TILE_WIDTH)
					.map(|lane| {
						lane_config(lane).with_thread_id([
							tile_x * TILE_WIDTH + lane % TILE_WIDTH,
							tile_y * TILE_WIDTH + lane / TILE_WIDTH,
						])
					})
					.collect();
				let mut workgroup = WorkgroupState::new();
				let mut descriptors = DescriptorBindings::new();
				descriptors.bind_texture(slot("scene_texture"), &mut scene);
				descriptors.bind_texture(slot("depth_texture"), &mut depth_texture);
				descriptors.bind_buffer(slot("parameters"), &mut parameters);
				descriptors.bind_image(slot("source"), &mut source);
				descriptors.bind_buffer(slot("tile_reaches"), &mut reaches);
				descriptors.bind_image(slot("debug_view_targets"), &mut debug_target);
				let [level_1, level_2, level_3] = &mut levels;
				descriptors.bind_image(slot("level_1"), level_1);
				descriptors.bind_image(slot("level_2"), level_2);
				descriptors.bind_image(slot("level_3"), level_3);
				descriptors.bind_workgroup_state(&mut workgroup);
				program
					.run_workgroup(&mut descriptors, &configs)
					.expect("The prepare workgroup runs in the VM.");
			}
		}

		// Each level averages the one before as the GPU stores it, in half floats.
		let half = |texel: [f32; 4]| texel.map(|value| half::f16::from_f32(value).to_f32());
		let mut previous: Vec<[f32; 4]> = (0..height)
			.flat_map(|y| (0..width).map(move |x| [x, y]))
			.map(|coordinate| half(rgba(&source, coordinate)))
			.collect();
		for (level, image) in levels.iter().enumerate() {
			let [child_width, level_width, level_height] = [width >> level, width >> (level + 1), height >> (level + 1)];
			let mut current = Vec::new();
			for y in 0..level_height {
				for x in 0..level_width {
					let child = |dx: u32, dy: u32| previous[((y * 2 + dy) * child_width + x * 2 + dx) as usize];
					let expected = reference_average([child(0, 0), child(1, 0), child(0, 1), child(1, 1)], (2 << level) as f32);
					let actual = rgba(image, [x, y]);
					for (actual, expected) in actual.iter().zip(expected) {
						assert!(
							(actual - expected).abs() <= 1e-6 * expected.abs().max(1.0),
							"level {} texel {x}, {y}: {actual} expected {expected}",
							level + 1
						);
					}
					current.push(half(actual));
				}
			}
			previous = current;
		}
		// The blurs span sharp to wide on both sides of the focus, so every weighting case ran.
		let blurs: Vec<f32> = (0..height)
			.flat_map(|y| (0..width).map(move |x| [x, y]))
			.map(|c| rgba(&source, c)[3])
			.collect();
		assert!(
			blurs.iter().any(|blur| blur.abs() < 1.0)
				&& blurs.iter().any(|blur| *blur < -8.0)
				&& blurs.iter().any(|blur| *blur > 16.0)
		);
	}

	/// Runs the filter shader on a 3 × 3 gathered image, each texel's color and ring share, and returns the center
	/// pixel it writes.
	fn filter_center(gathered: [[f32; 4]; 9]) -> [f32; 4] {
		use besl::vm::{DescriptorBindings, ResourceSlot};

		use crate::rendering::shader_vm_test::{compile, empty_image, rgba, run_at, texture_2d};

		let program = compile(simple_compute::compile_test_program(asset_source(
			"depth_of_field/filter.besl",
		)));
		let mut gathered = texture_2d(3, 3, &gathered);
		let mut result = empty_image(3, 3);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut gathered);
		descriptors.bind_image(ResourceSlot::new(1), &mut result);
		run_at(&program, &mut descriptors, [1, 1]);
		drop(descriptors);
		rgba(&result, [1, 1])
	}

	/// The filter moves a pixel toward its neighbors by its own ring share, and each neighbor counts by its share, so
	/// pixels the rings didn't reach neither blur nor tint the pixels around them, and a flat blur stays flat.
	#[test]
	fn filter_smooths_only_what_the_rings_carried() {
		let flat = [[0.5, 0.25, 1.0, 1.0]; 9];
		assert_eq!(filter_center(flat), [0.5, 0.25, 1.0, 1.0]);

		// A sharp pixel keeps its color among blurred neighbors.
		let mut sharp = [[1.0, 1.0, 1.0, 1.0]; 9];
		sharp[4] = [0.0, 0.0, 0.0, 0.0];
		assert_eq!(filter_center(sharp), [0.0, 0.0, 0.0, 1.0]);

		// Sharp neighbors don't tint a blurred pixel.
		let mut blurred = [[1.0, 1.0, 1.0, 0.0]; 9];
		blurred[4] = [0.0, 0.0, 0.0, 1.0];
		assert_eq!(filter_center(blurred), [0.0, 0.0, 0.0, 1.0]);

		// A grain on a flat blur fades by the tent: the center weighs 4 of 16, so a pixel of share 1 keeps a quarter of
		// its difference, and one of share 0.5 moves half as far.
		let mut grain = [[0.0, 0.0, 0.0, 1.0]; 9];
		grain[4] = [1.0, 1.0, 1.0, 1.0];
		let [red, ..] = filter_center(grain);
		assert!((red - 0.25).abs() < 1e-6, "{red}");
		for texel in &mut grain {
			texel[3] = 0.5;
		}
		let [red, ..] = filter_center(grain);
		assert!((red - 0.625).abs() < 1e-6, "{red}");
	}

	/// Where `sink`'s view shows a point `z` meters ahead on the view axis, in pixels from the image's left edge.
	fn pixel_x(sink: &Sink, z: f32) -> f32 {
		let clip = sink.view().projection_view() * Vec4f::new(0.0, 0.0, z, 1.0);
		(clip.x / clip.w + 1.0) * 0.5 * sink.extent().width() as f32
	}

	/// Opposite edges of the aperture see a point that many pixels apart: the reference averages each depth into the
	/// circle of confusion the pass computes for it. Behind the focus distance the right edge sees it further right;
	/// in front, further left, as the sign of the circle of confusion says.
	#[test]
	fn lens_samples_spread_each_depth_over_its_circle_of_confusion() {
		let sink = sink(1.8, 2.0);
		for z in [0.5, 1.0, 2.0, 4.0, 30.0] {
			let left = pixel_x(&sink.with_lens_sample([-1.0, 0.0]), z);
			let right = pixel_x(&sink.with_lens_sample([1.0, 0.0]), z);
			let expected = pass_circle_of_confusion(&sink, z);
			assert!(
				((right - left) - expected).abs() <= 1e-3 * expected.abs().max(1.0),
				"z {z}: {}, expected {expected}",
				right - left
			);
		}
		assert_eq!(sink.with_lens_sample([1.0, 0.0]).camera_view(), sink.view());
	}

	/// Runs the reference shader on one pixel whose scene is `scene` and whose mean holds `mean`, giving the frame
	/// `weight` of the mean, and returns the new mean and the pixel it writes to `main`.
	fn accumulate(scene: [f32; 4], mean: [f32; 3], weight: f32) -> ([f32; 3], [f32; 4]) {
		use besl::vm::{DescriptorBindings, ResourceSlot, Value, output_slot};

		use crate::rendering::shader_vm_test::{PIXEL_TILE, buffer, empty_image, rgba, run_pixel, texture_2d};

		let program = crate::rendering::shader_vm_test::compile(simple_compute::compile_test_program(REFERENCE_BESL));
		let parameter_slot = ResourceSlot::new(4);
		let mut parameters = buffer(&program, parameter_slot);
		parameters
			.write("weight", Value::F32(weight))
			.expect("Failed to initialize reference parameters. The most likely cause is a changed production buffer layout.");
		let mut scene_texture = texture_2d(1, 1, &[scene]);
		let mut channels = mean.map(|value| texture_2d(1, 1, &[[value, 0.0, 0.0, 1.0]]));
		let mut result = empty_image(1, 1);
		let mut descriptors = DescriptorBindings::new();
		descriptors.bind_texture(ResourceSlot::new(0), &mut scene_texture);
		for (slot, channel) in (1..).zip(channels.iter_mut()) {
			descriptors.bind_image(ResourceSlot::new(slot), channel);
		}
		descriptors.bind_buffer(parameter_slot, &mut parameters);
		descriptors.bind_image(output_slot(0), &mut result);
		run_pixel(&program, &mut descriptors, [0, 0], PIXEL_TILE, [1, 1]);
		drop(descriptors);
		(channels.map(|channel| rgba(&channel, [0, 0])[0]), rgba(&result, [0, 0]))
	}

	#[test]
	fn reference_besl_vm_moves_the_mean_by_the_frames_share() {
		let scene = [1.0, 0.5, 0.25, 0.75];
		// A first frame starts the mean at the scene, whatever the targets held, even NaN, and keeps the scene's alpha.
		assert_eq!(accumulate(scene, [f32::NAN, 9.0, 9.0], 1.0), ([1.0, 0.5, 0.25], scene));
		// A fourth frame moves a mean of three frames a quarter of the way to the scene.
		let (mean, shown) = accumulate(scene, [0.0, 0.5, 1.0], 0.25);
		assert_eq!(mean, [0.25, 0.5, 0.8125]);
		assert_eq!(shown, [0.25, 0.5, 0.8125, 0.75]);
		// A window's share is small, but 32-bit floats keep it.
		let share = 1.0 / REFERENCE_WINDOW as f32;
		let (mean, _) = accumulate(scene, [0.0, 0.5, 1.0], share);
		assert_eq!(mean, [share, 0.5, 1.0 - 0.75 * share]);
	}
}
