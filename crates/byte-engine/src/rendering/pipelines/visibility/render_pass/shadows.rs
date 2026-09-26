//! Depth-only rendering of the directional cascades, cone layers, and point cube faces selected this frame, and the
//! passes that fit the cascades to the surfaces the camera sees.

use ghi::context::{Context as _, ContextCreate as _};
use utils::Extent;

use super::super::layout::{
	CONE_SHADOW_MAP_RESOLUTION, CONE_SHADOW_VIEW_OFFSET, MAX_CONE_SHADOW_POOL_CAPACITY, MAX_POINT_SHADOW_POOL_CAPACITY,
	POINT_SHADOW_FACE_COUNT, POINT_SHADOW_MAP_RESOLUTION, POINT_SHADOW_VIEW_OFFSET, SHADOW_CASCADE_COUNT,
	SHADOW_MAP_RESOLUTION,
};
use super::super::mesh_dispatch::{MeshDispatch, PhaseDispatches};
use super::depth_pyramid::{ScreenViewData, screen_view_data};
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

impl ShadowWork {
	pub(crate) fn any(self) -> bool {
		self.directional.is_some() || self.cone_count > 0 || self.point_count > 0
	}
}

/// Returns the cascade view indices that receive one batched shadow dispatch.
pub(super) fn directional_shadow_view_indices(mesh_dispatch: MeshDispatch) -> impl Iterator<Item = u32> {
	let has_work = !mesh_dispatch.is_empty();
	(1..=SHADOW_CASCADE_COUNT as u32).filter(move |_| has_work)
}

/// Returns the packed cone view and target-layer indices that receive shadow dispatches.
pub(super) fn cone_shadow_view_indices(mesh_dispatch: MeshDispatch, cone_count: usize) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		cone_count.min(MAX_CONE_SHADOW_POOL_CAPACITY)
	};
	(0..count).map(|layer| ((CONE_SHADOW_VIEW_OFFSET + layer) as u32, layer as u32))
}

/// Returns the packed point-cube view and target-face indices that receive shadow dispatches.
pub(super) fn point_shadow_view_indices(mesh_dispatch: MeshDispatch, point_count: usize) -> impl Iterator<Item = (u32, u32)> {
	let count = if mesh_dispatch.is_empty() {
		0
	} else {
		point_count.min(MAX_POINT_SHADOW_POOL_CAPACITY)
	};
	(0..count * POINT_SHADOW_FACE_COUNT).map(|face| ((POINT_SHADOW_VIEW_OFFSET + face) as u32, face as u32))
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

/// The `ShadowPass` struct owns the pipelines and depth targets used by directional, cone, and point shadow rendering.
pub(super) struct ShadowPass {
	descriptor_set: ghi::DescriptorSetHandle,
	depth_pyramid_descriptor_set: ghi::DescriptorSetHandle,
	receiver_fit_descriptor_set: ghi::DescriptorSetHandle,
	receiver_fit_parameters: ghi::DynamicBufferHandle<ReceiverFitShaderData>,
	/// The box each cascade's receivers fill, rebuilt every frame the cascades fit receivers.
	receiver_bounds: ghi::BufferHandle<[u32; RECEIVER_BOUNDS_PER_CASCADE * SHADOW_CASCADE_COUNT]>,
	directional_pipeline: crate::rendering::PipelineRef,
	depth_pyramid_pipeline: crate::rendering::PipelineRef,
	receiver_bounds_pipeline: crate::rendering::PipelineRef,
	cascade_fit_pipeline: crate::rendering::PipelineRef,
	local_pipeline: crate::rendering::PipelineRef,
	masked_directional_pipeline: crate::rendering::PipelineRef,
	masked_local_pipeline: crate::rendering::PipelineRef,
	pub(super) directional_shadow_map: ghi::BaseImageHandle,
	pub(super) depth_pyramid: ghi::BaseImageHandle,
	pub(super) cone_shadow_map: ghi::BaseImageHandle,
	pub(super) point_shadow_map: ghi::BaseImageHandle,
}

#[derive(Clone, Copy)]
struct ShadowPipelines {
	directional: ghi::PipelineHandle,
	masked_directional: ghi::PipelineHandle,
	depth_pyramid: ghi::PipelineHandle,
	receiver_bounds: ghi::PipelineHandle,
	cascade_fit: ghi::PipelineHandle,
	/// Cone and point maps share one perspective depth pipeline.
	local: ghi::PipelineHandle,
	masked_local: ghi::PipelineHandle,
}

impl ShadowPass {
	/// Creates shadow targets and requests the depth pipelines matching their formats. `depth` is the sink's opaque
	/// depth, which the cascade fit reads.
	pub(super) fn new(
		context: &mut ghi::implementation::Context,
		pipeline_manager: &PipelineManagerClient,
		descriptor_set: ghi::DescriptorSetHandle,
		depth: ghi::BaseImageHandle,
		directional_shadow_map: ghi::BaseImageHandle,
		depth_pyramid: ghi::BaseImageHandle,
		cone_shadow_map: ghi::BaseImageHandle,
		point_shadow_map: ghi::BaseImageHandle,
	) -> Self {
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
				directional_shadow_map,
				max_sampler,
				ghi::Layouts::Read,
			),
			ghi::DescriptorWrite::image_mip(
				depth_pyramid_descriptor_set,
				DEPTH_PYRAMID_OUTPUT_BINDING.slot(),
				depth_pyramid,
				ghi::Layouts::General,
				0,
			),
		]);
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
			depth_pyramid_descriptor_set,
			receiver_fit_descriptor_set,
			receiver_fit_parameters,
			receiver_bounds,
			directional_pipeline: request("byte-engine/rendering/visibility/directional-shadow.pipeline"),
			depth_pyramid_pipeline: request("byte-engine/rendering/visibility/directional-shadow-depth-pyramid.pipeline"),
			receiver_bounds_pipeline: request("byte-engine/rendering/visibility/directional-shadow-receiver-bounds.pipeline"),
			cascade_fit_pipeline: request("byte-engine/rendering/visibility/directional-shadow-cascade-fit.pipeline"),
			local_pipeline: request("byte-engine/rendering/visibility/cone-shadow.pipeline"),
			masked_directional_pipeline: request("byte-engine/rendering/visibility/masked-directional-shadow.pipeline"),
			masked_local_pipeline: request("byte-engine/rendering/visibility/masked-cone-shadow.pipeline"),
			directional_shadow_map,
			depth_pyramid,
			cone_shadow_map,
			point_shadow_map,
		}
	}

	fn pipelines(&self, pipeline_manager: &PipelineManagerClient) -> Option<ShadowPipelines> {
		Some(ShadowPipelines {
			directional: pipeline_manager.pipeline(self.directional_pipeline)?,
			masked_directional: pipeline_manager.pipeline(self.masked_directional_pipeline)?,
			depth_pyramid: pipeline_manager.pipeline(self.depth_pyramid_pipeline)?,
			receiver_bounds: pipeline_manager.pipeline(self.receiver_bounds_pipeline)?,
			cascade_fit: pipeline_manager.pipeline(self.cascade_fit_pipeline)?,
			local: pipeline_manager.pipeline(self.local_pipeline)?,
			masked_local: pipeline_manager.pipeline(self.masked_local_pipeline)?,
		})
	}

	/// Prepares this frame's cascade fit and shadow maps, or `None` while a pipeline is still compiling.
	///
	/// Returns two recordings. The first fits the cascades to `sink`'s opaque surfaces when `work` asks for it, and
	/// records nothing otherwise; record it after the opaque visibility layer. The second draws the maps; record it
	/// after the first. Blend materials have no alpha-aware shadow shader, so only opaque and masked geometry casts shadows.
	pub(super) fn prepare(
		&self,
		frame: &mut ghi::implementation::Frame,
		pipeline_manager: &PipelineManagerClient,
		dispatches: PhaseDispatches,
		work: ShadowWork,
		sink: &Sink,
	) -> Option<(impl RenderPassFunction + use<>, impl RenderPassFunction + use<>)> {
		use ghi::frame::Frame as _;

		let pipelines = self.pipelines(pipeline_manager)?;
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
		let depth_pyramid_descriptor_set = self.depth_pyramid_descriptor_set;
		let directional_shadow_map = self.directional_shadow_map;
		let cone_shadow_map = self.cone_shadow_map;
		let point_shadow_map = self.point_shadow_map;
		let directional_extent = Extent::square(SHADOW_MAP_RESOLUTION);
		let depth_pyramid_extent = Extent::rectangle(
			SHADOW_MAP_RESOLUTION / 2,
			SHADOW_MAP_RESOLUTION / 2 * SHADOW_CASCADE_COUNT as u32,
		);
		let cone_extent = Extent::square(CONE_SHADOW_MAP_RESOLUTION);
		let point_extent = Extent::square(POINT_SHADOW_MAP_RESOLUTION);

		if work.directional.is_some() {
			frame.resize_image(directional_shadow_map, directional_extent);
		}
		if work.cone_count > 0 {
			frame.resize_image(cone_shadow_map, cone_extent);
		}
		if work.point_count > 0 {
			frame.resize_image(point_shadow_map, point_extent);
		}

		let fit = move |c: &mut ghi::implementation::CommandBufferRecording, _: &[ghi::AttachmentInformation]| {
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
			let bounds = c.bind_compute_pipeline(pipelines.receiver_bounds);
			bounds.bind_descriptor_sets(&[receiver_fit_descriptor_set]);
			bounds.dispatch(ghi::DispatchExtent::new(threads, Extent::square(8)));
			// One thread per cascade rewrites its view in the base set's views buffer.
			let fit = c.bind_compute_pipeline(pipelines.cascade_fit);
			fit.bind_descriptor_sets(&[descriptor_set, receiver_fit_descriptor_set]);
			fit.dispatch(ghi::DispatchExtent::new(
				Extent::line(SHADOW_CASCADE_COUNT as u32),
				Extent::line(SHADOW_CASCADE_COUNT as u32),
			));
			c.end_region();
		};

		Some((
			fit,
			move |c: &mut ghi::implementation::CommandBufferRecording, _: &[ghi::AttachmentInformation]| {
				use ghi::command_buffer::{
					BoundComputePipelineMode as _, BoundPipelineLayoutMode as _, BoundRasterizationPipelineMode as _,
					CommandBufferRecording as _, CommonCommandBufferMode as _, RasterizationRenderPassMode as _,
				};

				// Draws every solid and masked work range into the layers named by `views`.
				let record_maps = |c: &mut ghi::implementation::CommandBufferRecording,
				                   name: &str,
				                   target: ghi::BaseImageHandle,
				                   extent: Extent,
				                   layers: usize,
				                   solid_pipeline: ghi::PipelineHandle,
				                   masked_pipeline: ghi::PipelineHandle,
				                   views: &dyn Fn(MeshDispatch) -> Vec<(u32, u32)>| {
					c.start_region(|label| label.write_str(name));
					let attachments = [ghi::AttachmentInformation::new(
						target,
						ghi::Layouts::RenderTarget,
						ghi::LoadOp::Clear(ghi::ClearValue::Depth(0.0)),
						ghi::StoreOp::Store,
					)
					.layers(layers as u32)];
					let c = c.start_render_pass(extent, &attachments);
					for (dispatch, pipeline) in [(dispatches.opaque, solid_pipeline), (dispatches.masked, masked_pipeline)] {
						if dispatch.is_empty() {
							continue;
						}
						let c = c.bind_raster_pipeline(pipeline);
						c.bind_descriptor_sets(&[descriptor_set]);
						for (view_index, layer) in views(dispatch) {
							c.write_push_constant(0, dispatch.work_item_base());
							c.write_push_constant(4, view_index);
							c.write_push_constant(8, layer);
							c.dispatch_meshes(dispatch.workgroup_count(), 1, 1);
						}
					}
					c.end_render_pass();
					c.end_region();
				};

				if work.directional.is_some() {
					record_maps(
						c,
						"Directional Shadow Map",
						directional_shadow_map,
						directional_extent,
						SHADOW_CASCADE_COUNT,
						pipelines.directional,
						pipelines.masked_directional,
						&|dispatch| {
							directional_shadow_view_indices(dispatch)
								.map(|view| (view, view - 1))
								.collect()
						},
					);
					// Each SIMD-width workgroup reduces two adjacent 8x8 source tiles into one cell each.
					c.start_region(|label| label.write_str("Directional Shadow Depth Pyramid"));
					let c = c.bind_compute_pipeline(pipelines.depth_pyramid);
					c.bind_descriptor_sets(&[depth_pyramid_descriptor_set]);
					c.dispatch(ghi::DispatchExtent::new(depth_pyramid_extent, Extent::new(8, 4, 1)));
					c.end_region();
				}
				if work.cone_count > 0 {
					record_maps(
						c,
						"Cone Shadow Map",
						cone_shadow_map,
						cone_extent,
						work.cone_count,
						pipelines.local,
						pipelines.masked_local,
						&|dispatch| cone_shadow_view_indices(dispatch, work.cone_count).collect(),
					);
				}
				if work.point_count > 0 {
					record_maps(
						c,
						"Point Shadow Map",
						point_shadow_map,
						point_extent,
						work.point_count * POINT_SHADOW_FACE_COUNT,
						pipelines.local,
						pipelines.masked_local,
						&|dispatch| point_shadow_view_indices(dispatch, work.point_count).collect(),
					);
				}
			},
		))
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn shadow_dispatches_preserve_directional_cascades_cone_layers_and_point_cube_faces() {
		let dispatch = MeshDispatch::with_workgroup_count(19);

		assert_eq!(directional_shadow_view_indices(dispatch).collect::<Vec<_>>(), [1, 2, 3, 4]);
		assert_eq!(
			cone_shadow_view_indices(dispatch, 4).collect::<Vec<_>>(),
			[(5, 0), (6, 1), (7, 2), (8, 3)]
		);
		assert_eq!(
			cone_shadow_view_indices(dispatch, MAX_CONE_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(CONE_SHADOW_VIEW_OFFSET + MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32,
				(MAX_CONE_SHADOW_POOL_CAPACITY - 1) as u32
			))
		);
		assert_eq!(directional_shadow_view_indices(MeshDispatch::default()).count(), 0);
		assert_eq!(cone_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
		assert_eq!(
			point_shadow_view_indices(dispatch, 2).collect::<Vec<_>>(),
			(0..12u32)
				.map(|face| (POINT_SHADOW_VIEW_OFFSET as u32 + face, face))
				.collect::<Vec<_>>()
		);
		assert_eq!(
			point_shadow_view_indices(dispatch, MAX_POINT_SHADOW_POOL_CAPACITY + 1).last(),
			Some((
				(POINT_SHADOW_VIEW_OFFSET + MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
				(MAX_POINT_SHADOW_POOL_CAPACITY * POINT_SHADOW_FACE_COUNT - 1) as u32,
			))
		);
		assert_eq!(point_shadow_view_indices(MeshDispatch::default(), 4).count(), 0);
	}
}
