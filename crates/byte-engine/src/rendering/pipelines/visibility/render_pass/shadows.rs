//! Depth-only rendering of the directional cascades, cone layers, and point cube faces selected this frame, and the
//! passes that fit the cascades to the surfaces the camera sees.
//!
//! [`ShadowMaps`] renders the maps that update this frame once for every sink, sizing each image to the maps the shared
//! shadow-map budget gives it. [`CascadeFitPass`] runs per sink, because it reads that sink's depth.

use std::num::NonZeroU32;

use ghi::context::{Context as _, ContextCreate as _};
use smallvec::SmallVec;
use utils::Extent;

use super::super::layout::{
	CONE_SHADOW_MAP_FORMAT, DIRECTIONAL_SHADOW_MAP_FORMAT, LOCAL_SHADOW_SIZE_CLASS_COUNT, MAX_DIRECTIONAL_SHADOW_COUNT,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_FORMAT, SHADOW_CASCADE_COUNT, local_shadow_map_resolution,
};
use super::super::mesh_dispatch::PhaseDispatches;
use super::super::shader_data::ShaderViewData;
use super::super::shadow_selection::{ShadowLayout, SunShadow};
use super::super::shadow_updates::{ShadowDraw, ShadowDraws, ShadowImage};
use super::depth_pyramid::{ScreenViewData, screen_view_data};
use super::{ComputeStage, OcclusionPhase, Pipelines, StageCounters, record_compute_stages, record_meshlet_dispatches};
use crate::rendering::csm::{CASTER_REACH, CascadeFrame, EDGE_TEXELS, SIZE_STEPS_PER_OCTAVE};
use crate::rendering::pipelines::visibility::set_layouts;
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// Mip count of the packed cascade depth pyramid; one retained level of max-depth cells.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT: u32 = 1;
/// Shadow-map texels on each side of one max-depth cell in the cascade depth pyramid. The directional shadow helpers
/// and `directional-shadow-depth-pyramid.besl` assume this size.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE: u32 = 8;
/// Encoded bounds per cascade: the lower then the upper corner of the box its receivers fill.
const RECEIVER_BOUNDS_PER_CASCADE: usize = 6;
/// Every sun's cascades, one fitted box each.
const RECEIVER_FIT_CASCADES: usize = SHADOW_CASCADE_COUNT * MAX_DIRECTIONAL_SHADOW_COUNT;

/// Each shadowed sun's cascades, fitted to the camera frustum, by sun slot.
pub(crate) type SunCascades = SmallVec<[[CascadeFrame; SHADOW_CASCADE_COUNT]; MAX_DIRECTIONAL_SHADOW_COUNT]>;

/// The `ShadowWork` struct says which shadow views received lights this frame and how many maps each image holds.
#[derive(Clone, Debug, Default)]
pub(crate) struct ShadowWork {
	/// The shadowed suns by sun slot. Each one runs the whole sun path: cascades, receiver fit, depth pyramids, and sun
	/// visibility.
	pub(crate) suns: SmallVec<[SunShadow; MAX_DIRECTIONAL_SHADOW_COUNT]>,
	/// Texels per side of each directional cascade.
	pub(crate) cascade_resolution: u32,
	/// Each sun's cascades as the CPU fitted them to the camera frustum, when this sink shrinks them to the surfaces its
	/// camera sees. Empty draws them as fitted.
	pub(crate) receiver_fit: SunCascades,
	/// One bit per cascade, bit `4s + c` for cascade `c` of sun slot `s`, set for the cascades that hold a drawn map
	/// after this frame's draws.
	pub(crate) drawn_cascades: u32,
	/// The maps each shadow image keeps this frame, which may exceed the ones drawn. See
	/// [`super::super::shadow_selection::retain_layout`].
	pub(crate) layout: ShadowLayout,
	/// The maps that draw this frame, ordered by image and then layer. See
	/// [`super::super::shadow_updates::ShadowMapStates::update`].
	pub(crate) draws: ShadowDraws,
}

impl ShadowWork {
	/// Returns one bit per cascade, bit `4s + c` for cascade `c` of sun slot `s`, set for the cascades that draw this
	/// frame. A cascade's directional layer is its bit.
	pub(crate) fn updated_cascades(&self) -> u32 {
		self.draws
			.iter()
			.filter(|draw| draw.image == ShadowImage::Directional)
			.fold(0, |mask, draw| mask | 1 << draw.layer)
	}
}

/// The `ShadowRun` struct is a run of maps of one image that draw in one layered render pass: consecutive layers drawn
/// from consecutive views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ShadowRun {
	image: ShadowImage,
	first_layer: u32,
	first_view: u32,
	count: u32,
}

/// Groups `draws`, ordered by image and then layer, into runs of consecutive layers drawn from consecutive views.
fn shadow_runs(draws: &[ShadowDraw]) -> SmallVec<[ShadowRun; 16]> {
	let mut runs = SmallVec::<[ShadowRun; 16]>::new();
	for draw in draws {
		match runs.last_mut() {
			Some(run)
				if run.image == draw.image
					&& run.first_layer + run.count == draw.layer
					&& run.first_view + run.count == draw.view =>
			{
				run.count += 1;
			}
			_ => runs.push(ShadowRun {
				image: draw.image,
				first_layer: draw.layer,
				first_view: draw.view,
				count: 1,
			}),
		}
	}
	runs
}

/// The `ReceiverFitShaderData` struct carries what the receiver-bounds and cascade-fit passes need from the CPU: how to
/// rebuild each pixel's view-space position and project it into each sun's cascade, and where the cascades lie.
///
/// Every member is a four-float row, so the CPU and every shader backend agree on its layout. Cascade `c` of sun slot
/// `s` is fitted box `4s + c`.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ReceiverFitShaderData {
	/// Three rows per fitted box that map a camera view-space position to the frustum-fitted cascade's normalized
	/// device x, y, and z.
	pub(crate) view_to_cascade_rows: [[f32; 4]; 3 * RECEIVER_FIT_CASCADES],
	/// Per fitted box, the frustum-fitted view's light-space center x and y, half extent, and depth range, in meters.
	pub(crate) cascade_frames: [[f32; 4]; RECEIVER_FIT_CASCADES],
	/// The camera-space distance at which each cascade ends. Splits depend only on the camera, so every sun shares them.
	pub(crate) split_far: [f32; 4],
	/// The pixel-to-ray scale in x and y and offset in z and w. See [`ScreenViewData`].
	pub(crate) pixel_to_ray: [f32; 4],
	/// The depth unprojection numerator in x and denominator offset in y. See [`ScreenViewData`].
	pub(crate) depth_unproject: [f32; 4],
	/// The shadow-map resolution, edge margin in texels, caster reach in meters, and size steps per octave.
	pub(crate) fit_constants: [f32; 4],
	/// One bit per fitted box in x for the cascades that draw this frame, and in y for the ones that hold a drawn map.
	/// A cascade that doesn't draw gets the view it was drawn with, or a view no receiver falls in when it holds none.
	pub(crate) cascade_masks: [u32; 4],
}

impl Default for ReceiverFitShaderData {
	fn default() -> Self {
		bytemuck::Zeroable::zeroed()
	}
}

/// Builds the receiver-fit constants for a camera and each sun's cascades, by sun slot, as the CPU fitted them to its
/// frustum, for cascades of `shadow_map_resolution` texels per side. `updated_cascades` and `drawn_cascades` mark the
/// cascades that draw this frame and the ones that hold a drawn map, one bit per fitted box.
pub(crate) fn receiver_fit_shader_data(
	screen: ScreenViewData,
	camera_view: View,
	suns: &[[CascadeFrame; SHADOW_CASCADE_COUNT]],
	shadow_map_resolution: u32,
	updated_cascades: u32,
	drawn_cascades: u32,
) -> ReceiverFitShaderData {
	let camera_to_world = math::inverse(camera_view.view());
	let mut data = ReceiverFitShaderData {
		pixel_to_ray: [
			screen.pixel_to_ray_mul[0],
			screen.pixel_to_ray_mul[1],
			screen.pixel_to_ray_add[0],
			screen.pixel_to_ray_add[1],
		],
		depth_unproject: [
			screen.depth_unproject_numerator,
			screen.depth_unproject_denominator_offset,
			0.0,
			0.0,
		],
		fit_constants: [shadow_map_resolution as f32, EDGE_TEXELS, CASTER_REACH, SIZE_STEPS_PER_OCTAVE],
		cascade_masks: [updated_cascades, drawn_cascades, 0, 0],
		..Default::default()
	};
	for (slot, cascades) in suns.iter().enumerate() {
		for (cascade, frame) in cascades.iter().enumerate() {
			let fitted_box = slot * SHADOW_CASCADE_COUNT + cascade;
			// Orthographic views are affine, so three rows carry the whole map.
			let view_to_cascade = frame.view.view_projection() * camera_to_world;
			for row in 0..3 {
				data.view_to_cascade_rows[3 * fitted_box + row] =
					std::array::from_fn(|column| view_to_cascade[4 * row + column]);
			}
			data.cascade_frames[fitted_box] = [frame.center[0], frame.center[1], frame.half_extent, frame.depth];
			data.split_far[cascade] = frame.slice_far;
		}
	}
	data
}

/// The `ShadowMaps` struct holds the frame's shadow maps for every sink at once.
///
/// The shadow views come from the first sink, so each sink would render the same maps. The visibility pipeline manager
/// owns one `ShadowMaps` and records it with the first sink only; every sink's material evaluation samples its
/// [`Self::directional`], [`Self::directional_depth_pyramid`], [`Self::cone`], and [`Self::point`] images. Each frame
/// renders its maps before it reads them, and the graphics queue orders one frame's reads before the next frame's
/// writes, so frames in flight share the images too.
///
/// Each image holds as many maps as [`ShadowWork::layout`] gives its kind, so their memory together stays within the
/// shadow-map budget. Sun slot `s` draws into directional layers `4s` through `4s + 3`. Cone and point lights keep one
/// image per size class, so each light's map has the size its class gives it.
pub(crate) struct ShadowMaps {
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	directional_pipelines: Pipelines<4>,
	/// Cone and point maps share one perspective depth format, so they share these pipelines.
	local_pipelines: Pipelines<4>,
	depth_pyramid_pipeline: Pipelines<1>,
	pub(crate) directional: ghi::BaseImageHandle,
	/// One max-depth cell per [`DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE`] texels of each cascade, one layer's cells below
	/// the previous layer's.
	pub(crate) directional_depth_pyramid: ghi::BaseImageHandle,
	/// The same cells' min depth, which proves receivers behind every texel of an area fully shadowed.
	pub(crate) directional_depth_minimum_pyramid: ghi::BaseImageHandle,
	/// One image per size class, the smallest first.
	pub(crate) cone: [ghi::BaseImageHandle; LOCAL_SHADOW_SIZE_CLASS_COUNT],
	/// One image per size class, the smallest first.
	pub(crate) point: [ghi::BaseImageHandle; LOCAL_SHADOW_SIZE_CLASS_COUNT],
}

/// The fewest layers the cone image keeps: Metal requires two to create the array texture that material evaluation
/// always binds.
const MIN_CONE_SHADOW_LAYERS: usize = 2;

/// Returns the max-depth pyramid's extent for `suns` suns' cascades of `resolution` texels per side. It keeps one sun's
/// cells without a sun, so its storage descriptors always bind an image.
fn depth_pyramid_extent(resolution: u32, suns: usize) -> Extent {
	let cells = resolution / DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE;
	Extent::rectangle(cells, cells * (SHADOW_CASCADE_COUNT * suns.max(1)) as u32)
}

impl ShadowMaps {
	/// Creates the shadow maps and requests their depth pipelines. `resolution` sizes the cascade depth pyramid for
	/// cascades of that many texels per side, a multiple of 16 so the pyramid reduces them in whole cells; every frame's
	/// [`ShadowWork::cascade_resolution`] draws them at it. Every map image starts empty, and [`Self::prepare`] sizes
	/// it to the maps the budget gives it.
	///
	/// Next, write [`Self::directional`], [`Self::directional_depth_pyramid`], [`Self::cone`], and [`Self::point`] into
	/// each sink's material-evaluation descriptor set, as [`super::VisibilityRenderPass::new`] does, and record
	/// [`Self::prepare`] with the first sink each frame.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		resolution: u32,
	) -> Self {
		fn depth_map(format: ghi::Formats, name: &str) -> ghi::image::Builder<'_> {
			ghi::image::Builder::new(format, ghi::Uses::DepthStencil | ghi::Uses::Image)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.optimized_clear_value(ghi::ClearValue::Depth(0.0))
		}
		let directional: ghi::BaseImageHandle = context
			.build_image(
				depth_map(DIRECTIONAL_SHADOW_MAP_FORMAT, "Directional Shadow Map")
					.array_layers(NonZeroU32::new(SHADOW_CASCADE_COUNT as u32)),
			)
			.into();
		// The cells are the maximum or minimum of 16-bit depths, which is one of those depths, so 16-bit unorm cells
		// hold them exactly and the probes' comparisons stay exact.
		let depth_pyramid = |name| {
			ghi::image::Builder::new(ghi::Formats::R16UNORM, ghi::Uses::Storage | ghi::Uses::Image)
				.name(name)
				.extent(depth_pyramid_extent(resolution, 1))
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
				.mip_levels(DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT)
		};
		let directional_depth_pyramid: ghi::BaseImageHandle =
			context.build_image(depth_pyramid("Directional Shadow Depth Pyramid")).into();
		let directional_depth_minimum_pyramid: ghi::BaseImageHandle = context
			.build_image(depth_pyramid("Directional Shadow Minimum Depth Pyramid"))
			.into();
		// Images start at zero extent, so no map has backing memory until a visible light uses it.
		let cone = std::array::from_fn(|_| {
			context
				.build_image(
					depth_map(CONE_SHADOW_MAP_FORMAT, "Cone Shadow Map")
						.array_layers(NonZeroU32::new(MIN_CONE_SHADOW_LAYERS as u32)),
				)
				.into()
		});
		let point = std::array::from_fn(|_| {
			context
				.build_image(depth_map(POINT_SHADOW_MAP_FORMAT, "Point Shadow Map").cube_array_compatible(NonZeroU32::MIN))
				.into()
		});
		let depth_pyramid_descriptor_set =
			set_layouts::SHADOW_DEPTH_PYRAMID.create_set(context, "Directional Shadow Depth Pyramid Descriptor Set");
		// The pyramid gathers texel quads, which no sampler filter or reduction touches.
		let source_sampler = context.build_sampler(ghi::sampler::Builder::new());
		let pyramid = set_layouts::SHADOW_DEPTH_PYRAMID.binder(depth_pyramid_descriptor_set);
		context.write(&[
			pyramid.sampled("source_depth", directional, source_sampler),
			pyramid.image_mip("reduced_depth_1", directional_depth_pyramid, 0),
			pyramid.image_mip("reduced_minimum_1", directional_depth_minimum_pyramid, 0),
		]);
		Self {
			depth_pyramid_descriptor_set,
			directional_pipelines: Pipelines::phases(pipeline_manager, "directional-shadow"),
			local_pipelines: Pipelines::phases(pipeline_manager, "cone-shadow"),
			depth_pyramid_pipeline: Pipelines::request(pipeline_manager, ["directional-shadow-depth-pyramid"]),
			directional,
			directional_depth_pyramid,
			directional_depth_minimum_pyramid,
			cone,
			point,
		}
	}

	/// Prepares this frame's shadow maps, or `None` while a pipeline is still compiling.
	///
	/// Record the result after the cascade fit of the sink the views were made for, see [`CascadeFitPass::prepare`].
	/// Blend materials have no alpha-aware shadow shader, so only opaque and masked geometry casts shadows.
	///
	/// `descriptor_sets` are the recording sink's meshlet sets: the base set and its occlusion culling set. The shadow
	/// passes share the camera's task shader, which declares the occlusion resources, so they bind them but never cull
	/// by occlusion. `counters` are that sink's stage counters; the maps and the pyramid record theirs only when they
	/// record.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		dispatches: PhaseDispatches,
		work: &ShadowWork,
		descriptor_sets: [ghi::DescriptorSetHandle; 2],
		counters: StageCounters,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let directional_pipelines = self.directional_pipelines.resolve(pipeline_manager)?;
		let local_pipelines = self.local_pipelines.resolve(pipeline_manager)?;
		let [depth_pyramid_pipeline] = self.depth_pyramid_pipeline.resolve(pipeline_manager)?;
		let cascade_count = SHADOW_CASCADE_COUNT * work.suns.len();
		let runs = shadow_runs(&work.draws);
		// One thread per 4x4 texel block: each SIMD-width workgroup reduces four by two cells, four threads per cell.
		let depth_pyramid = ComputeStage {
			label: "Directional Shadow Depth Pyramid",
			pipeline: depth_pyramid_pipeline,
			descriptor_sets: [self.depth_pyramid_descriptor_set],
			extent: Extent::rectangle(
				work.cascade_resolution / 4,
				work.cascade_resolution / 4 * cascade_count as u32,
			),
			workgroup: Extent::new(8, 4, 1),
		};
		let (directional, cone, point) = (self.directional, self.cone, self.point);
		let directional_extent = Extent::square(work.cascade_resolution);
		let class_extent = |size_class| Extent::square(local_shadow_map_resolution(size_class));

		// An image without maps drops its memory; one with maps holds every map the budget kept for its kind. Every image
		// keeps the fewest layers its array view needs.
		let resize = |frame: &mut ghi::implementation::Frame, image, extent, layers: usize, min_layers: usize| {
			let extent = if layers == 0 { Extent::square(0) } else { extent };
			let layers = NonZeroU32::new(layers.max(min_layers) as u32).expect("Shadow map images keep at least one layer.");
			frame.resize_image_layers(image, extent, layers);
		};
		let layout = work.layout;
		resize(
			frame,
			directional,
			directional_extent,
			SHADOW_CASCADE_COUNT * layout.suns,
			SHADOW_CASCADE_COUNT,
		);
		for size_class in 0..LOCAL_SHADOW_SIZE_CLASS_COUNT {
			let extent = class_extent(size_class);
			resize(
				frame,
				cone[size_class],
				extent,
				layout.cones[size_class],
				MIN_CONE_SHADOW_LAYERS,
			);
			resize(
				frame,
				point[size_class],
				extent,
				POINT_SHADOW_FACE_COUNT * layout.points[size_class],
				POINT_SHADOW_FACE_COUNT,
			);
		}
		for pyramid in [self.directional_depth_pyramid, self.directional_depth_minimum_pyramid] {
			frame.resize_image(pyramid, depth_pyramid_extent(work.cascade_resolution, layout.suns));
		}

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
			};

			// Draws every work range into one run of layers: layer `first_layer + n` shows view `first_view + n`. One task
			// workgroup culls its meshlets against a batch of views, so each instance's data is read once per batch instead
			// of once per view. Only the run's layers are cleared, so every other map keeps what it holds.
			let record_run = |c: &mut ghi::implementation::CommandBufferRecording, run: &ShadowRun| {
				let (name, target, extent, pipelines) = match run.image {
					ShadowImage::Directional => (
						"Directional Shadow Map",
						directional,
						directional_extent,
						directional_pipelines,
					),
					ShadowImage::Cone(size_class) => {
						("Cone Shadow Map", cone[size_class], class_extent(size_class), local_pipelines)
					}
					ShadowImage::Point(size_class) => (
						"Point Shadow Map",
						point[size_class],
						class_extent(size_class),
						local_pipelines,
					),
				};
				c.start_region(|label| label.write_str(name));
				let attachments = [ghi::AttachmentInformation::new(
					target,
					ghi::Layouts::RenderTarget,
					ghi::LoadOp::Clear(ghi::ClearValue::Depth(0.0)),
					ghi::StoreOp::Store,
				)
				.layers(run.first_layer, run.count)];
				let c = c.start_render_pass(extent, &attachments);
				let ranges = dispatches.opaque_layer.into_iter().zip(pipelines);
				let (first_view, count) = (run.first_view as usize, run.count as usize);
				record_meshlet_dispatches(c, descriptor_sets, OcclusionPhase::Disabled, ranges, first_view, count);
				c.end_render_pass();
				c.end_region();
			};

			// Every map draws before the pyramid, so the raster passes stay back to back in one stretch of render work.
			if !runs.is_empty() {
				c.counter(counters.shadow_maps, |c| {
					for run in &runs {
						record_run(c, run);
					}
				});
			}
			if cascade_count > 0 {
				c.counter(counters.shadow_depth_pyramid, |c| {
					record_compute_stages(c, None, &[depth_pyramid])
				});
			}
		})
	}
}

/// The `CascadeFitPass` struct shrinks every shadowed sun's cascades to the surfaces one sink's camera sees.
///
/// Each sink owns one, because the fit reads that sink's depth. Only the sink the views were made for fits them; see
/// [`ShadowWork::receiver_fit`].
pub(super) struct CascadeFitPass {
	receiver_fit_descriptor_set: ghi::DescriptorSetHandle,
	receiver_fit_parameters: ghi::DynamicBufferHandle<ReceiverFitShaderData>,
	/// The box each cascade's receivers fill. The linear depth pyramid pass merges every frame's receivers into it,
	/// and the fit resets it after reading, so it is cleared once, before its first frame.
	receiver_bounds: ghi::BufferHandle<[u32; RECEIVER_BOUNDS_PER_CASCADE * RECEIVER_FIT_CASCADES]>,
	bounds_cleared: std::sync::atomic::AtomicBool,
	pipelines: Pipelines<1>,
}

impl CascadeFitPass {
	/// Creates the fit's buffers and requests its pipeline. `views` is the scene's views buffer, whose cascade views
	/// the fit rewrites through its own set, the only one that binds them for writing. Hand
	/// [`Self::receiver_bounds`] and [`Self::receiver_fit_parameters`] to the sink's [`super::DepthPyramidPass`],
	/// which finds the bounds.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		views: ghi::BaseBufferHandle,
	) -> Self {
		let receiver_fit_descriptor_set =
			set_layouts::CASCADE_FIT.create_set(context, "Directional Shadow Receiver Fit Descriptor Set");
		let receiver_fit_parameters = context.build_dynamic_buffer(
			ghi::buffer::Builder::new(ghi::Uses::Storage)
				.name("Directional Shadow Receiver Fit Parameters")
				.device_accesses(ghi::DeviceAccesses::HostToDevice),
		);
		let device_buffer = |name, uses| {
			ghi::buffer::Builder::new(ghi::Uses::Storage | uses)
				.name(name)
				.device_accesses(ghi::DeviceAccesses::DeviceOnly)
		};
		let receiver_bounds = context.build_buffer(device_buffer(
			"Directional Shadow Receiver Bounds",
			ghi::Uses::TransferDestination,
		));
		// The fit clamps whatever size a new buffer holds to a valid one, so it needs no initial contents.
		let cascade_size_steps: ghi::BufferHandle<[u32; RECEIVER_FIT_CASCADES]> =
			context.build_buffer(device_buffer("Directional Shadow Cascade Size Steps", ghi::Uses::empty()));
		// The fit reads a cascade's drawn view only once a frame that drew it has written it.
		let drawn_views: ghi::BufferHandle<[ShaderViewData; RECEIVER_FIT_CASCADES]> =
			context.build_buffer(device_buffer("Directional Shadow Drawn Cascade Views", ghi::Uses::empty()));
		let fit = set_layouts::CASCADE_FIT.binder(receiver_fit_descriptor_set);
		context.write(&[
			fit.buffer("views", views),
			fit.buffer("receiver_bounds", receiver_bounds),
			fit.buffer("receiver_fit", receiver_fit_parameters),
			fit.buffer("cascade_size_steps", cascade_size_steps),
			fit.buffer("drawn_views", drawn_views),
		]);
		Self {
			receiver_fit_descriptor_set,
			receiver_fit_parameters,
			receiver_bounds,
			bounds_cleared: std::sync::atomic::AtomicBool::new(false),
			pipelines: Pipelines::request(pipeline_manager, ["directional-shadow-cascade-fit"]),
		}
	}

	/// The box each cascade's receivers fill, for the pass that finds them.
	pub(super) fn receiver_bounds(&self) -> ghi::BaseBufferHandle {
		self.receiver_bounds.into()
	}

	/// The CPU's cascade fit the bounds are measured against, for the pass that finds them.
	pub(super) fn receiver_fit_parameters(&self) -> ghi::BaseBufferHandle {
		self.receiver_fit_parameters.into()
	}

	/// Prepares this frame's cascade fit, or `None` while a pipeline is still compiling.
	///
	/// The result fits the cascades of every sun in `receiver_fit`, by sun slot as the CPU fitted them to the camera
	/// frustum, to `sink`'s opaque surfaces, and records nothing without them. Only the cascades `updated_cascades`
	/// marks are fitted; every other one gets the view it was drawn with when `drawn_cascades` marks it, or a view no
	/// receiver falls in otherwise. Record it after the depth pyramid pass, which finds the receiver bounds, and the
	/// shadow maps after it.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		receiver_fit: &[[CascadeFrame; SHADOW_CASCADE_COUNT]],
		cascade_resolution: u32,
		[updated_cascades, drawn_cascades]: [u32; 2],
		sink: &Sink,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let [cascade_fit_pipeline] = self.pipelines.resolve(pipeline_manager)?;
		let fitted_cascades = (SHADOW_CASCADE_COUNT * receiver_fit.len()) as u32;
		let fits_receivers = fitted_cascades > 0;
		if fits_receivers {
			*frame.get_mut_dynamic_buffer_slice(self.receiver_fit_parameters) = receiver_fit_shader_data(
				screen_view_data(sink, sink.extent()),
				sink.view(),
				receiver_fit,
				cascade_resolution,
				updated_cascades,
				drawn_cascades,
			);
			frame.sync_buffer(self.receiver_fit_parameters);
		}
		// A new buffer holds anything; the fit zeroes it after every read from then on.
		let clear_bounds = fits_receivers && !self.bounds_cleared.swap(true, std::sync::atomic::Ordering::Relaxed);
		let receiver_fit_descriptor_set = self.receiver_fit_descriptor_set;
		let receiver_bounds = self.receiver_bounds;

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			if !fits_receivers {
				return;
			}
			c.start_region(|label| label.write_str("Directional Shadow Receiver Fit"));
			if clear_bounds {
				c.clear_buffers(&[receiver_bounds.into()]);
			}
			// One thread per cascade of every sun rewrites its view in the scene's views buffer, one workgroup per sun.
			let fit = c.bind_compute_pipeline(cascade_fit_pipeline);
			fit.bind_descriptor_sets(&[receiver_fit_descriptor_set]);
			fit.dispatch(ghi::DispatchExtent::new(
				Extent::line(fitted_cascades),
				Extent::line(SHADOW_CASCADE_COUNT as u32),
			));
			c.end_region();
		})
	}
}
