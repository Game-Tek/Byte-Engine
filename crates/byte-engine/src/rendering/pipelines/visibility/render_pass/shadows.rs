//! Depth-only rendering of the directional cascades, cone layers, and point cube faces selected this frame, and the
//! passes that fit the cascades to the surfaces the camera sees.
//!
//! [`ShadowMaps`] renders the maps once per frame for every sink. [`CascadeFitPass`] runs per sink, because it reads
//! that sink's depth.

use std::num::NonZeroU32;

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::super::layout::{
	CONE_SHADOW_MAP_FORMAT, CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET, DIRECTIONAL_SHADOW_MAP_FORMAT,
	MAX_CONE_SHADOW_POOL_CAPACITY, MAX_POINT_SHADOW_POOL_CAPACITY, POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_FORMAT,
	POINT_SHADOW_MAP_RESOLUTION, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT, SHADOW_MAP_RESOLUTION,
};
use super::super::mesh_dispatch::PhaseDispatches;
use super::depth_pyramid::{ScreenViewData, screen_view_data};
use super::{OcclusionPhase, PhasePipelines, record_meshlet_dispatches};
use crate::rendering::csm::{CASTER_REACH, CascadeFrame, EDGE_TEXELS, SIZE_STEPS_PER_OCTAVE};
use crate::rendering::render_pass::RenderPassFunction;
use crate::rendering::{PipelineManagerClient, Sink, View};

/// Mip count of the packed cascade depth pyramid; one retained level of max-depth cells.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT: u32 = 1;
/// Shadow-map texels on each side of one max-depth cell in the cascade depth pyramid. The directional shadow helpers
/// and `directional-shadow-depth-pyramid.besl` assume this size.
pub(crate) const DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE: u32 = 8;
const DEPTH_PYRAMID_SOURCE_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
)
.texture_view_type(ghi::TextureViewTypes::Texture2DArray);
const DEPTH_PYRAMID_OUTPUT_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageImage,
	ghi::AccessPolicies::WRITE,
);
const RECEIVER_DEPTH_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1033),
	ghi::ResourceKind::CombinedImageSampler,
	ghi::AccessPolicies::READ,
);
const RECEIVER_BOUNDS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1034),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ_WRITE,
);
const RECEIVER_FIT_PARAMETERS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1035),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ,
);
const CASCADE_SIZE_STEPS_BINDING: ghi::ShaderResourceDescriptor = ghi::ShaderResourceDescriptor::single(
	ghi::ResourceSlot::new(1036),
	ghi::ResourceKind::StorageBuffer,
	ghi::AccessPolicies::READ_WRITE,
);
/// Screen pixels on each side of the square one receiver-bounds thread reads. The bounds shader assumes this size.
const RECEIVER_BOUNDS_PIXELS_PER_THREAD: u32 = 4;
/// Encoded bounds per cascade: the lower then the upper corner of the box its receivers fill.
const RECEIVER_BOUNDS_PER_CASCADE: usize = 6;

/// The `ShadowWork` struct says which shadow views received lights this frame.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ShadowWork {
	/// The world-space direction the shadow-casting sun's light travels, or `None` without a sun.
	pub(crate) directional: Option<math::UnitVector>,
	/// The sun's cascades as the CPU fitted them to the camera frustum, when this sink shrinks them to the surfaces its
	/// camera sees. `None` draws them as fitted.
	pub(crate) receiver_fit: Option<[CascadeFrame; SHADOW_CASCADE_COUNT]>,
	pub(crate) cone_count: usize,
	pub(crate) point_count: usize,
}

/// The `ReceiverFitShaderData` struct carries what the receiver-bounds and cascade-fit passes need from the CPU: how to
/// rebuild each pixel's view-space position and project it into its cascade, and where the cascades lie.
///
/// Every member is a four-float row, so the CPU and every shader backend agree on its layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ReceiverFitShaderData {
	/// Three rows per cascade that map a camera view-space position to the frustum-fitted cascade's normalized device
	/// x, y, and z.
	pub(crate) view_to_cascade_rows: [[f32; 4]; 3 * SHADOW_CASCADE_COUNT],
	/// Per cascade, the frustum-fitted view's light-space center x and y, half extent, and depth range, in meters.
	pub(crate) cascade_frames: [[f32; 4]; SHADOW_CASCADE_COUNT],
	/// The camera-space distance at which each cascade ends.
	pub(crate) split_far: [f32; 4],
	/// The pixel-to-ray scale in x and y and offset in z and w. See [`ScreenViewData`].
	pub(crate) pixel_to_ray: [f32; 4],
	/// The depth unprojection numerator in x and denominator offset in y. See [`ScreenViewData`].
	pub(crate) depth_unproject: [f32; 4],
	/// The shadow-map resolution, edge margin in texels, caster reach in meters, and size steps per octave.
	pub(crate) fit_constants: [f32; 4],
}

/// Builds the receiver-fit constants for a camera and the cascades the CPU fitted to its frustum.
pub(crate) fn receiver_fit_shader_data(
	screen: ScreenViewData,
	camera_view: View,
	cascades: &[CascadeFrame; SHADOW_CASCADE_COUNT],
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
		fit_constants: [SHADOW_MAP_RESOLUTION as f32, EDGE_TEXELS, CASTER_REACH, SIZE_STEPS_PER_OCTAVE],
		..Default::default()
	};
	for (cascade, frame) in cascades.iter().enumerate() {
		// Orthographic views are affine, so three rows carry the whole map.
		let view_to_cascade = frame.view.view_projection() * camera_to_world;
		for row in 0..3 {
			data.view_to_cascade_rows[3 * cascade + row] = std::array::from_fn(|column| view_to_cascade[4 * row + column]);
		}
		data.cascade_frames[cascade] = [frame.center[0], frame.center[1], frame.half_extent, frame.depth];
		data.split_far[cascade] = frame.slice_far;
	}
	data
}

/// The images the shadow maps are rendered into. Every sink's material evaluation samples the same images.
#[derive(Clone, Copy)]
pub(crate) struct ShadowMapImages {
	pub(crate) directional: ghi::BaseImageHandle,
	/// One max-depth cell per [`DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE`] texels of each cascade.
	pub(crate) directional_depth_pyramid: ghi::BaseImageHandle,
	pub(crate) cone: ghi::BaseImageHandle,
	pub(crate) point: ghi::BaseImageHandle,
}

/// The `ShadowMaps` struct holds the frame's shadow maps for every sink at once.
///
/// The shadow views come from the first sink, so each sink would render the same maps. The visibility pipeline manager
/// owns one `ShadowMaps` and records it with the first sink only; every sink's material evaluation samples its
/// [`ShadowMaps::images`]. Each frame renders its maps before it reads them, and the graphics queue orders one frame's
/// reads before the next frame's writes, so frames in flight share the images too.
pub(crate) struct ShadowMaps {
	descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	directional_pipelines: PhasePipelines,
	/// Cone and point maps share one perspective depth format, so they share these pipelines.
	local_pipelines: PhasePipelines,
	depth_pyramid_pipeline: crate::rendering::PipelineRef,
	/// The images every sink's material evaluation samples.
	pub(crate) images: ShadowMapImages,
}

impl ShadowMaps {
	/// Creates the shadow maps and requests their depth pipelines. `descriptor_set` is the base visibility set.
	///
	/// Next, write [`Self::images`] into each sink's material-evaluation descriptor set, and record
	/// [`Self::prepare`] with the first sink each frame.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		cone_shadow_pool_capacity: usize,
		point_shadow_pool_capacity: usize,
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
		let directional_depth_pyramid: ghi::BaseImageHandle = context
			.build_image(
				ghi::image::Builder::new(ghi::Formats::R32F, ghi::Uses::Storage | ghi::Uses::Image)
					.name("Directional Shadow Depth Pyramid")
					.extent(Extent::rectangle(
						SHADOW_MAP_RESOLUTION / DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE,
						SHADOW_MAP_RESOLUTION / DIRECTIONAL_SHADOW_DEPTH_CELL_SIZE * SHADOW_CASCADE_COUNT as u32,
					))
					.device_accesses(ghi::DeviceAccesses::DeviceOnly)
					.mip_levels(DIRECTIONAL_SHADOW_DEPTH_PYRAMID_MIP_COUNT),
			)
			.into();
		// Images start at zero extent, so these pools have no backing maps until a visible light uses them.
		// Metal requires two layers to create the array texture that material evaluation always binds.
		let cone: ghi::BaseImageHandle = context
			.build_image(
				depth_map(CONE_SHADOW_MAP_FORMAT, "Cone Shadow Map")
					.array_layers(NonZeroU32::new(cone_shadow_pool_capacity.max(2) as u32)),
			)
			.into();
		let point: ghi::BaseImageHandle = context
			.build_image(
				depth_map(POINT_SHADOW_MAP_FORMAT, "Point Shadow Map").cube_array_compatible(
					NonZeroU32::new(point_shadow_pool_capacity.max(1) as u32)
						.expect("Point shadow map pool has a nonzero fallback cube."),
				),
			)
			.into();

		let depth_pyramid_descriptor_set =
			context.create_descriptor_set(Some("Directional Shadow Depth Pyramid Descriptor Set"));
		let max_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Linear)
				.reduction_mode(ghi::SamplingReductionModes::Max)
				.mip_map_mode(ghi::FilteringModes::Linear)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod(0.0),
		);
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				depth_pyramid_descriptor_set,
				DEPTH_PYRAMID_SOURCE_BINDING.slot(),
				directional,
				max_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				DEPTH_PYRAMID_OUTPUT_BINDING.slot(),
				directional_depth_pyramid,
				ghi::Layouts::General,
				0,
			),
		]);
		Self {
			descriptor_set,
			depth_pyramid_descriptor_set,
			directional_pipelines: PhasePipelines::request(
				pipeline_manager,
				[
					"byte-engine/rendering/visibility/directional-shadow.pipeline",
					"byte-engine/rendering/visibility/masked-directional-shadow.pipeline",
					"byte-engine/rendering/visibility/double-sided-directional-shadow.pipeline",
					"byte-engine/rendering/visibility/double-sided-masked-directional-shadow.pipeline",
				],
			),
			local_pipelines: PhasePipelines::request(
				pipeline_manager,
				[
					"byte-engine/rendering/visibility/cone-shadow.pipeline",
					"byte-engine/rendering/visibility/masked-cone-shadow.pipeline",
					"byte-engine/rendering/visibility/double-sided-cone-shadow.pipeline",
					"byte-engine/rendering/visibility/double-sided-masked-cone-shadow.pipeline",
				],
			),
			depth_pyramid_pipeline: pipeline_manager
				.request_pipeline("byte-engine/rendering/visibility/directional-shadow-depth-pyramid.pipeline"),
			images: ShadowMapImages {
				directional,
				directional_depth_pyramid,
				cone,
				point,
			},
		}
	}

	/// Prepares this frame's shadow maps, or `None` while a pipeline is still compiling.
	///
	/// Record the result after the cascade fit of the sink the views were made for, see [`CascadeFitPass::prepare`].
	/// Blend materials have no alpha-aware shadow shader, so only opaque and masked geometry casts shadows.
	///
	/// `occlusion_descriptor_set` is the recording sink's occlusion culling set. The shadow passes share the camera's
	/// task shader, which declares those resources, so they bind it but never cull by occlusion.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		dispatches: PhaseDispatches,
		work: ShadowWork,
		occlusion_descriptor_set: ghi::DescriptorSetHandle,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let directional_pipelines = self.directional_pipelines.resolve(pipeline_manager)?;
		let local_pipelines = self.local_pipelines.resolve(pipeline_manager)?;
		let depth_pyramid_pipeline = pipeline_manager.pipeline(self.depth_pyramid_pipeline)?;
		let descriptor_sets = [self.descriptor_set, occlusion_descriptor_set];
		let depth_pyramid_descriptor_set = self.depth_pyramid_descriptor_set;
		let images = self.images;
		let directional_extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let depth_pyramid_extent = Extent::rectangle(
			SHADOW_MAP_RESOLUTION / 2,
			SHADOW_MAP_RESOLUTION / 2 * SHADOW_CASCADE_COUNT as u32,
		);
		let cone_extent = Extent::square(CONE_SHADOW_MAP_RESOLUTION);
		let point_extent = Extent::square(POINT_SHADOW_MAP_RESOLUTION);

		if work.directional.is_some() {
			frame.resize_image(images.directional, directional_extent);
		}
		if work.cone_count > 0 {
			frame.resize_image(images.cone, cone_extent);
		}
		if work.point_count > 0 {
			frame.resize_image(images.point, point_extent);
		}

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
			};

			// Draws every work range into `view_count` layers: layer `n` shows packed view `view_base + n`. One task
			// workgroup culls its meshlets against a batch of views, so each instance's data is read once per batch
			// instead of once per view.
			let record_maps = |c: &mut ghi::implementation::CommandBufferRecording,
			                   name: &str,
			                   target: ghi::BaseImageHandle,
			                   extent: Extent,
			                   layers: usize,
			                   pipelines: [ghi::PipelineHandle; 4],
			                   view_base: usize,
			                   view_count: usize| {
				c.start_region(|label| label.write_str(name));
				let attachments = [ghi::AttachmentInformation::new(
					target,
					ghi::Layouts::RenderTarget,
					ghi::LoadOp::Clear(ghi::ClearValue::Depth(0.0)),
					ghi::StoreOp::Store,
				)
				.layers(layers as u32)];
				let c = c.start_render_pass(extent, &attachments);
				let ranges = dispatches.opaque_layer().into_iter().zip(pipelines);
				record_meshlet_dispatches(c, descriptor_sets, OcclusionPhase::Disabled, ranges, view_base, view_count);
				c.end_render_pass();
				c.end_region();
			};

			if work.directional.is_some() {
				record_maps(
					c,
					"Directional Shadow Map",
					images.directional,
					directional_extent,
					SHADOW_CASCADE_COUNT,
					directional_pipelines,
					// View zero is the camera, so the cascades follow it.
					1,
					SHADOW_CASCADE_COUNT,
				);
				// Each SIMD-width workgroup reduces two adjacent 8x8 source tiles into one cell each.
				c.start_region(|label| label.write_str("Directional Shadow Depth Pyramid"));
				let c = c.bind_compute_pipeline(depth_pyramid_pipeline);
				c.bind_descriptor_sets(&[depth_pyramid_descriptor_set]);
				c.dispatch(ghi::DispatchExtent::new(depth_pyramid_extent, Extent::new(8, 4, 1)));
				c.end_region();
			}
			if work.cone_count > 0 {
				record_maps(
					c,
					"Cone Shadow Map",
					images.cone,
					cone_extent,
					work.cone_count,
					local_pipelines,
					CONE_SHADOW_VIEW_OFFSET,
					work.cone_count.min(MAX_CONE_SHADOW_POOL_CAPACITY),
				);
			}
			if work.point_count > 0 {
				record_maps(
					c,
					"Point Shadow Map",
					images.point,
					point_extent,
					work.point_count * POINT_SHADOW_FACE_COUNT,
					local_pipelines,
					POINT_SHADOW_VIEW_OFFSET,
					work.point_count.min(MAX_POINT_SHADOW_POOL_CAPACITY) * POINT_SHADOW_FACE_COUNT,
				);
			}
		})
	}
}

/// The `CascadeFitPass` struct shrinks the sun's cascades to the surfaces one sink's camera sees.
///
/// Each sink owns one, because the fit reads that sink's depth. Only the sink the views were made for fits them; see
/// [`ShadowWork::receiver_fit`].
pub(super) struct CascadeFitPass {
	descriptor_set: ghi::DescriptorSetHandle,
	receiver_fit_descriptor_set: ghi::DescriptorSetHandle,
	receiver_fit_parameters: ghi::DynamicBufferHandle<ReceiverFitShaderData>,
	/// The box each cascade's receivers fill, rebuilt every frame the cascades fit receivers.
	receiver_bounds: ghi::BufferHandle<[u32; RECEIVER_BOUNDS_PER_CASCADE * SHADOW_CASCADE_COUNT]>,
	receiver_bounds_pipeline: crate::rendering::PipelineRef,
	cascade_fit_pipeline: crate::rendering::PipelineRef,
}

impl CascadeFitPass {
	/// Creates the fit's buffers and requests its pipelines. `descriptor_set` is the base visibility set, whose views
	/// the fit rewrites, and `depth` is the sink's opaque depth, which it reads.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
	) -> Self {
		let receiver_fit_descriptor_set = context.create_descriptor_set(Some("Directional Shadow Receiver Fit Descriptor Set"));
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
		let cascade_size_steps: ghi::BufferHandle<[u32; SHADOW_CASCADE_COUNT]> =
			context.build_buffer(device_buffer("Directional Shadow Cascade Size Steps", ghi::Uses::empty()));
		let point_sampler = context.build_sampler(
			ghi::sampler::Builder::new()
				.filtering_mode(ghi::FilteringModes::Closest)
				.mip_map_mode(ghi::FilteringModes::Closest)
				.addressing_mode(ghi::SamplerAddressingModes::Clamp)
				.min_lod(0.0)
				.max_lod(0.0),
		);
		let fit_buffer = |binding: ghi::ShaderResourceDescriptor, buffer: ghi::BaseBufferHandle| {
			ghi::DescriptorWrite::buffer(receiver_fit_descriptor_set, binding.slot(), buffer)
		};
		context.write(&[
			ghi::DescriptorWrite::combined_image_sampler(
				receiver_fit_descriptor_set,
				RECEIVER_DEPTH_BINDING.slot(),
				depth,
				point_sampler,
				ghi::Layouts::Read,
			),
			fit_buffer(RECEIVER_BOUNDS_BINDING, receiver_bounds.into()),
			fit_buffer(RECEIVER_FIT_PARAMETERS_BINDING, receiver_fit_parameters.into()),
			fit_buffer(CASCADE_SIZE_STEPS_BINDING, cascade_size_steps.into()),
		]);
		let request = |name| pipeline_manager.request_pipeline(name);
		Self {
			descriptor_set,
			receiver_fit_descriptor_set,
			receiver_fit_parameters,
			receiver_bounds,
			receiver_bounds_pipeline: request("byte-engine/rendering/visibility/directional-shadow-receiver-bounds.pipeline"),
			cascade_fit_pipeline: request("byte-engine/rendering/visibility/directional-shadow-cascade-fit.pipeline"),
		}
	}

	/// Prepares this frame's cascade fit, or `None` while a pipeline is still compiling.
	///
	/// The result fits the cascades to `sink`'s opaque surfaces when `work` asks for it, and records nothing otherwise.
	/// Record it after the opaque visibility layer, and the shadow maps after it.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		work: ShadowWork,
		sink: &Sink,
	) -> Option<impl RenderPassFunction + use<>> {
		use ghi::frame::Frame as _;

		let receiver_bounds_pipeline = pipeline_manager.pipeline(self.receiver_bounds_pipeline)?;
		let cascade_fit_pipeline = pipeline_manager.pipeline(self.cascade_fit_pipeline)?;
		let receiver_fit_extent = work.receiver_fit.map(|cascades| {
			let extent = sink.extent();
			*frame.get_mut_dynamic_buffer_slice(self.receiver_fit_parameters) =
				receiver_fit_shader_data(screen_view_data(sink, extent), sink.view(), &cascades);
			frame.sync_buffer(self.receiver_fit_parameters);
			extent
		});
		let receiver_fit_descriptor_set = self.receiver_fit_descriptor_set;
		let receiver_bounds = self.receiver_bounds;
		let descriptor_set = self.descriptor_set;

		Some(move |c: &mut ghi::implementation::CommandBufferRecording| {
			use ghi::command_buffer::{
				BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, CommandBufferRecording as _,
				CommonCommandBufferMode as _,
			};

			let Some(extent) = receiver_fit_extent else {
				return;
			};
			c.start_region(|label| label.write_str("Directional Shadow Receiver Fit"));
			// Bounds only grow within a frame, so they start empty.
			c.clear_buffers(&[receiver_bounds.into()]);
			let threads = Extent::rectangle(
				extent.width().div_ceil(RECEIVER_BOUNDS_PIXELS_PER_THREAD),
				extent.height().div_ceil(RECEIVER_BOUNDS_PIXELS_PER_THREAD),
			);
			let bounds = c.bind_compute_pipeline(receiver_bounds_pipeline);
			bounds.bind_descriptor_sets(&[receiver_fit_descriptor_set]);
			bounds.dispatch(ghi::DispatchExtent::new(threads, Extent::square(8)));
			// One thread per cascade rewrites its view in the base set's views buffer.
			let fit = c.bind_compute_pipeline(cascade_fit_pipeline);
			fit.bind_descriptor_sets(&[descriptor_set, receiver_fit_descriptor_set]);
			fit.dispatch(ghi::DispatchExtent::new(
				Extent::line(SHADOW_CASCADE_COUNT as u32),
				Extent::line(SHADOW_CASCADE_COUNT as u32),
			));
			c.end_region();
		})
	}
}
