use std::ptr::NonNull;

use ::utils::hash::HashMap;
use objc2_foundation::NSString;
use objc2_metal::{MTLCommandBuffer, MTLCommandEncoder, MTLTexture};

use super::*;
use crate::command_buffer::{
	BoundComputePipelineMode, BoundPipelineLayoutMode, BoundRasterizationPipelineMode, BoundRayTracingPipelineMode,
	CommandBufferRecording as CommandBufferRecordingTrait, CommonCommandBufferMode, RasterizationRenderPassMode,
};

pub struct CommandBufferRecording<'a> {
	device: &'a mut device::Device,
	command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	sequence_index: u8,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	present_drawables: Vec<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
	states: HashMap<Handle, TransitionState>,
	bound_pipeline_layout: Option<graphics_hardware_interface::PipelineLayoutHandle>,
	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	active_render_encoder: Option<Retained<ProtocolObject<dyn mtl::MTLRenderCommandEncoder>>>,
}

pub struct FinishedCommandBuffer<'a> {
	pub(crate) command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	pub(crate) command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
	pub(crate) present_drawables: Vec<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
	pub(crate) states: HashMap<Handle, TransitionState>,
	pub(crate) present_keys: &'a [graphics_hardware_interface::PresentKey],
}

impl<'a> CommandBufferRecording<'a> {
	pub fn new(
		device: &'a mut device::Device,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>,
		frame_key: Option<graphics_hardware_interface::FrameKey>,
	) -> Self {
		let sequence_index = frame_key.map(|key| key.sequence_index).unwrap_or(0);
		let states = device.states.clone();

		Self {
			device,
			command_buffer_handle,
			sequence_index,
			command_buffer,
			present_drawables: Vec::new(),
			states,
			bound_pipeline_layout: None,
			bound_pipeline: None,
			active_render_encoder: None,
		}
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> buffer::BufferHandle {
		let handles = buffer::BufferHandle(handle.0).get_all(&self.device.buffers);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> image::ImageHandle {
		let handles = image::ImageHandle(handle.0).get_all(&self.device.images);
		handles[(self.sequence_index as usize).rem_euclid(handles.len())]
	}

	fn consume_resources(&mut self, consumptions: impl IntoIterator<Item = Consumption>) {
		for consumption in consumptions {
			self.states.insert(
				consumption.handle,
				TransitionState {
					layout: consumption.layout,
				},
			);
		}
	}

	fn finish(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		for drawable in &self.present_drawables {
			let drawable_ref: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			self.command_buffer.presentDrawable(drawable_ref);
		}

		self.command_buffer.commit();
		self.command_buffer.waitUntilCompleted();

		if let Some(synchronizer) = self.device.synchronizers.get_mut(synchronizer.0 as usize) {
			synchronizer.signaled = true;
		}

		self.device.states = self.states;
	}

	fn take_drawable(
		&mut self,
		present_key: graphics_hardware_interface::PresentKey,
	) -> Option<Retained<ProtocolObject<dyn CAMetalDrawable>>> {
		let swapchain = &mut self.device.swapchains[present_key.swapchain.0 as usize];
		swapchain.take_drawable(present_key.image_index)
	}

	fn collect_present_drawables(&mut self, present_keys: &[graphics_hardware_interface::PresentKey]) {
		for &present_key in present_keys {
			if let Some(drawable) = self.take_drawable(present_key) {
				self.present_drawables.push(drawable);
			}
		}
	}
}

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	type Result<'a> = FinishedCommandBuffer<'a>;

	fn build_top_level_acceleration_structure(
		&mut self,
		_acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn start_render_pass(
		&mut self,
		_extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		let consumptions = attachments
			.iter()
			.map(|attachment| Consumption {
				handle: Handle::Image(self.get_internal_image_handle(attachment.image)),
				stages: crate::Stages::FRAGMENT,
				access: if attachment.load {
					crate::AccessPolicies::READ_WRITE
				} else {
					crate::AccessPolicies::WRITE
				},
				layout: attachment.layout,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		let rpd = mtl::MTLRenderPassDescriptor::new();

		for (i, attachment) in attachments
			.iter()
			.filter(|attachment| attachment.format != crate::Formats::Depth32)
			.enumerate()
		{
			let att = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(i) };
			let image = &self.device.images[self.get_internal_image_handle(attachment.image).0 as usize];

			att.setTexture(Some(image.texture.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearColor(utils::clear_color(attachment.clear));
		}

		if let Some(attachment) = attachments
			.iter()
			.find(|attachment| attachment.format == crate::Formats::Depth32)
		{
			let att = unsafe { rpd.depthAttachment() };
			let image = &self.device.images[self.get_internal_image_handle(attachment.image).0 as usize];

			att.setTexture(Some(image.texture.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearDepth(utils::clear_depth(attachment.clear));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).unwrap();

		self.active_render_encoder = Some(rce);

		self
	}

	fn clear_images<I: graphics_hardware_interface::ImageHandleLike>(
		&mut self,
		textures: &[(I, graphics_hardware_interface::ClearValue)],
	) {
		let consumptions = textures
			.iter()
			.map(|(handle, _)| Consumption {
				handle: Handle::Image(self.get_internal_image_handle((*handle).into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		// TODO: Encode blit clears for textures.
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		let consumptions = buffer_handles
			.iter()
			.map(|buffer_handle| Consumption {
				handle: Handle::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		// TODO: Encode fillBuffer on MTLBlitCommandEncoder.
	}

	fn transfer_textures(
		&mut self,
		texture_handles: &[impl graphics_hardware_interface::ImageHandleLike],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		let consumptions = texture_handles
			.iter()
			.map(|handle| Consumption {
				handle: Handle::Image(self.get_internal_image_handle((*handle).into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions);

		texture_handles
			.iter()
			.map(|handle| {
				self.device
					.copy_texture_to_cpu(self.get_internal_image_handle(handle.into_image_handle()))
			})
			.collect()
	}

	fn write_image_data(
		&mut self,
		image_handle: impl graphics_hardware_interface::ImageHandleLike,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle.into_image_handle());
		self.consume_resources([Consumption {
			handle: Handle::Image(image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}]);

		let image = &mut self.device.images[image_handle.0 as usize];
		let Some(staging) = image.staging.as_mut() else {
			return;
		};
		let bytes = unsafe {
			std::slice::from_raw_parts(
				data.as_ptr() as *const u8,
				data.len() * std::mem::size_of::<graphics_hardware_interface::RGBAu8>(),
			)
		};
		let length = staging.len().min(bytes.len());
		staging[..length].copy_from_slice(&bytes[..length]);

		let Some(bytes_per_pixel) = utils::bytes_per_pixel(image.format) else {
			return;
		};
		let width = image.extent.width() as usize;
		let height = image.extent.height() as usize;
		let bytes_per_row = width * bytes_per_pixel;
		let region = mtl::MTLRegion {
			origin: mtl::MTLOrigin { x: 0, y: 0, z: 0 },
			size: mtl::MTLSize {
				width: width as _,
				height: height as _,
				depth: 1,
			},
		};
		let staging_ptr = NonNull::new(staging.as_ptr() as *mut std::ffi::c_void)
			.expect("Texture staging pointer was null. The most likely cause is a zero-sized texture.");

		unsafe {
			image
				.texture
				.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, staging_ptr, bytes_per_row as _);
		}

		self.consume_resources([Consumption {
			handle: Handle::Image(image_handle),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}]);
	}

	fn blit_image(
		&mut self,
		source_image: impl graphics_hardware_interface::ImageHandleLike,
		_source_layout: crate::Layouts,
		destination_image: impl graphics_hardware_interface::ImageHandleLike,
		_destination_layout: crate::Layouts,
	) {
		self.consume_resources([
			Consumption {
				handle: Handle::Image(self.get_internal_image_handle(source_image.into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: crate::Layouts::Transfer,
			},
			Consumption {
				handle: Handle::Image(self.get_internal_image_handle(destination_image.into_image_handle())),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: crate::Layouts::Transfer,
			},
		]);

		// TODO: Encode MTLBlitCommandEncoder copyFromTexture.
	}

	fn copy_to_swapchain(
		&mut self,
		_source_texture_handle: impl graphics_hardware_interface::ImageHandleLike,
		_present_key: graphics_hardware_interface::PresentKey,
		_swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) {
		// TODO: Render/copy source texture into swapchain drawable.
	}

	fn execute(self, _synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.finish(_synchronizer);
	}

	fn end<'a>(mut self, present_keys: &'a [graphics_hardware_interface::PresentKey]) -> Self::Result<'a> {
		self.collect_present_drawables(present_keys);

		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}

		FinishedCommandBuffer {
			command_buffer_handle: self.command_buffer_handle,
			command_buffer: self.command_buffer,
			present_drawables: self.present_drawables,
			states: self.states,
			present_keys,
		}
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.bound_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self
	}

	fn start_region(&self, name: &str) {
		self.command_buffer.pushDebugGroup(&NSString::from_str(name));
	}

	fn end_region(&self) {
		self.command_buffer.popDebugGroup();
	}

	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
		self.start_region(name);
		f(self);
		self.end_region();
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline = &self.device.pipelines[pipeline_handle.0 as usize];

		self.bound_pipeline_layout = Some(pipeline.layout);

		// self.active_render_encoder.unwrap().setRenderPipelineState(pipeline.pipeline);

		self
	}

	fn bind_vertex_buffers(&mut self, _buffer_descriptors: &[crate::BufferDescriptor]) {
		// self.active_render_encoder.unwrap().setVertexBuffer_offset_atIndex();
	}

	fn bind_index_buffer(&mut self, _buffer_descriptor: &crate::BufferDescriptor) {
		//
	}

	fn end_render_pass(&mut self) {
		if let Some(encoder) = self.active_render_encoder.take() {
			encoder.endEncoding();
		}
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, _sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		// TODO: Map descriptor sets to Metal argument buffers and encoder bindings.
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, _offset: u32, _data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		// TODO: Map push constants to MTLBuffer/bytes per stage.
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, _mesh_handle: &graphics_hardware_interface::MeshHandle) {
		// TODO: Issue draw call using mesh buffers.
	}

	fn draw(&mut self, _vertex_count: u32, _instance_count: u32, _first_vertex: u32, _first_instance: u32) {
		// TODO: Issue non-indexed draw call.
	}

	fn draw_indexed(
		&mut self,
		_index_count: u32,
		_instance_count: u32,
		_first_index: u32,
		_vertex_offset: i32,
		_first_instance: u32,
	) {
		// TODO: Issue indexed draw call.
	}

	fn dispatch_meshes(&mut self, _x: u32, _y: u32, _z: u32) {
		// TODO: Map mesh shading to Metal mesh shaders when supported.
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, _dispatch: graphics_hardware_interface::DispatchExtent) {
		// TODO: Encode dispatch on MTLComputeCommandEncoder.
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		_buffer: graphics_hardware_interface::BufferHandle<[(u32, u32, u32); N]>,
		_entry_index: usize,
	) {
		// TODO: Encode indirect dispatch.
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: Encode Metal ray tracing dispatch.
	}
}
