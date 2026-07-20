use objc2_foundation::NSAutoreleasePool;
use objc2_foundation::NSString;
use objc2_metal::MTLBlitCommandEncoder;
use objc2_metal::MTLCommandBuffer;
use objc2_metal::MTLCommandEncoder;
use smallvec::SmallVec;

use super::*;
use crate::image::ImageHandle;
use crate::SwapchainHandle;

/// The `Frame` struct scopes Metal rendering state to one frame.
///
/// Its `NSAutoreleasePool` releases temporary Metal objects at the end of the frame.
/// Without this pool, objects accumulate on threads that do not have a run-loop pool.
///
/// Field order matters: Rust drops fields in declaration order. The drawables must be
/// released before the autorelease pool drains, so `_autorelease_pool` is declared last.
pub struct Frame<'a> {
	frame_key: graphics_hardware_interface::FrameKey,
	drawables: SmallVec<[(SwapchainHandle, Retained<ProtocolObject<dyn CAMetalDrawable>>); 4]>,
	device: &'a mut context::Context,
	_autorelease_pool: Retained<NSAutoreleasePool>,
}

impl<'a> Frame<'a> {
	pub fn new(device: &'a mut context::Context, frame_key: graphics_hardware_interface::FrameKey) -> Self {
		let pool = unsafe { NSAutoreleasePool::new() };
		Self {
			frame_key,
			drawables: SmallVec::new(),
			device,
			_autorelease_pool: pool,
		}
	}

	fn get_current_image_handle(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> ImageHandle {
		self.device
			.images
			.nth_handle(image_handle, self.frame_key.sequence_index as _)
			.unwrap()
	}

	fn get_current_buffer_handle(
		&self,
		buffer_handle: graphics_hardware_interface::BaseBufferHandle,
	) -> crate::buffer::BufferHandle {
		self.device
			.buffers
			.nth_handle(buffer_handle, self.frame_key.sequence_index as _)
			.expect(
				"Missing Metal frame-local buffer. The most likely cause is that the dynamic buffer chain was not created for this frame.",
			)
	}

	fn frame_buffer_pointer(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> *mut u8 {
		let buffer = self.device.buffers.resource(self.get_current_buffer_handle(buffer_handle));
		let buffer = buffer
			.staging
			.map(|staging_handle| self.device.buffers.resource(staging_handle))
			.unwrap_or(buffer);

		buffer.pointer
	}

	fn frame_texture_staging_parts(&self, image_handle: graphics_hardware_interface::BaseImageHandle) -> (*mut u8, usize) {
		let image = self.device.images.resource(self.get_current_image_handle(image_handle));
		let staging = image.staging.as_ref().expect(
			"Missing Metal texture staging data. The most likely cause is that CPU texture access was requested for a device-only image.",
		);

		(staging.as_ptr() as *mut u8, staging.len())
	}
}

impl Frame<'_> {
	pub fn intern_raster_pipeline(
		&mut self,
		pipeline: crate::metal::device::Pipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_raster_pipeline(pipeline)
	}

	pub fn intern_compute_pipeline(
		&mut self,
		pipeline: crate::metal::device::ComputePipeline,
	) -> graphics_hardware_interface::PipelineHandle {
		self.device.intern_compute_pipeline(pipeline)
	}

	/// Interns a factory-built image through this frame's device.
	pub fn intern_image(&mut self, image: crate::metal::device::Image) -> graphics_hardware_interface::ImageHandle {
		self.device.intern_image(image)
	}

	/// Interns a factory-built sampler through this frame's device.
	pub fn intern_sampler(&mut self, sampler: crate::metal::device::Sampler) -> graphics_hardware_interface::SamplerHandle {
		self.device.intern_sampler(sampler)
	}

	pub fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &'static mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	pub fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	pub fn get_mut_dynamic_buffer_slice<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		unsafe { &mut *(self.frame_buffer_pointer(buffer_handle.into()) as *mut T) }
	}

	pub fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &'static mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	pub fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back(handle);
	}

	pub fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	pub fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		let handle = self.get_current_image_handle(image_handle);
		let image = self.device.images.resource(handle);

		if image.extent == extent {
			return;
		}

		let replacement = self.device.create_image_resource(
			image.name.as_deref(),
			extent,
			image.format,
			image.uses,
			image.access,
			image.array_layers,
		);
		*self.device.images.resource_mut(handle) = replacement;
		self.device.rewrite_descriptors_for_handle(PrivateHandles::Image(handle));
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBufferRecording<'a> {
		self.device
			.create_command_buffer_recording_with_frame_key(command_buffer_handle, Some(self.frame_key))
	}

	pub fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		let sequence_index = self.frame_key.sequence_index;

		// Update layer extent before acquiring the drawable so that if a resize occurred,
		// the drawable is allocated at the correct size. update_layer_extent only calls
		// setDrawableSize when the size actually changed, avoiding unnecessary drawable
		// pool invalidation.
		let extent = {
			let swapchain = &self.device.swapchains[swapchain_handle.0 as usize];
			update_layer_extent(&swapchain.layer, &swapchain.view)
		};

		// Resize proxy images to match the new drawable size so the blit has matching dimensions.
		self.device.resize_swapchain_images(swapchain_handle, extent);

		let drawable = self.device.swapchains[swapchain_handle.0 as usize]
			.layer
			.nextDrawable()
			.expect("Failed to acquire Metal drawable. The most likely cause is that the layer has no available drawables.");

		let present_key = graphics_hardware_interface::PresentKey {
			image_index: 0,
			sequence_index,
			swapchain: swapchain_handle,
		};

		self.drawables.push((swapchain_handle, drawable));

		(present_key, extent)
	}

	pub fn device(&mut self) -> &mut context::Context {
		self.device
	}

	pub fn execute_finished(
		&mut self,
		cbr: super::FinishedCommandBuffer<'_>,
		present_keys: &[graphics_hardware_interface::PresentKey],
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) {
		let super::FinishedCommandBuffer {
			command_buffer_handle: _command_buffer_handle,
			command_buffer,
			state_updates,
			_marker,
		} = cbr;
		let mut present_drawables = SmallVec::<
			[(
				graphics_hardware_interface::PresentKey,
				Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
			); 4],
		>::new();

		for &present_key in present_keys {
			let drawable = self
				.drawables
				.iter()
				.position(|(swapchain, _)| *swapchain == present_key.swapchain)
				.map(|index| self.drawables.swap_remove(index).1);
			present_drawables.push((present_key, drawable));
		}

		if !present_keys.is_empty() {
			let blit_encoder = command_buffer.blitCommandEncoder().expect(
				"Metal blit command encoder creation failed. The most likely cause is that the command buffer could not start the swapchain resolve pass.",
			);
			#[cfg(debug_assertions)]
			if self.device.settings.debug_labels {
				blit_encoder.setLabel(Some(&NSString::from_str("Present Resolve")));
			}

			for (present_key, drawable) in &present_drawables {
				let Some(drawable) = drawable else {
					continue;
				};
				let swapchain = &self.device.swapchains[present_key.swapchain.0 as usize];
				let Some(proxy_image) = swapchain.images[present_key.sequence_index as usize] else {
					continue;
				};
				let source_texture = &self.device.images.resource(proxy_image).texture;
				let destination_texture = drawable.texture();

				unsafe {
					blit_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
				}
			}

			blit_encoder.endEncoding();
		}

		for (_, drawable) in &present_drawables {
			let Some(drawable) = drawable else {
				continue;
			};
			let drawable_ref: &ProtocolObject<dyn mtl::MTLDrawable> = drawable.as_ref();
			command_buffer.presentDrawable(drawable_ref);
		}

		self.device
			.submit_metal_command_buffer_for_synchronizer(command_buffer, synchronizer, self.frame_key.sequence_index);

		self.device.states.extend(state_updates);
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= super::CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key
	}

	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: crate::BufferHandle<T>) -> &'static mut T {
		self.device.get_mut_buffer_slice(buffer_handle)
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<crate::BaseBufferHandle>) {
		self.device.sync_buffer(buffer_handle);
	}

	fn get_texture_slice_mut(&self, texture_handle: graphics_hardware_interface::BaseImageHandle) -> &'static mut [u8] {
		let (pointer, length) = self.frame_texture_staging_parts(texture_handle);

		unsafe { std::slice::from_raw_parts_mut(pointer, length) }
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle) {
		let handle = self.get_current_image_handle(image_handle);
		self.device.pending_image_syncs.push_back(handle);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn get_mut_dynamic_buffer_slice<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
	) -> &mut T {
		Frame::get_mut_dynamic_buffer_slice(self, buffer_handle)
	}

	fn resize_image(&mut self, image_handle: graphics_hardware_interface::BaseImageHandle, extent: Extent) {
		Frame::resize_image(self, image_handle, extent);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.device
			.create_command_buffer_recording_with_frame_key(command_buffer_handle, Some(self.frame_key))
	}

	fn acquire_swapchain_image(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> (graphics_hardware_interface::PresentKey, Extent) {
		Frame::acquire_swapchain_image(self, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	fn create_allocation(
		&mut self,
		size: usize,
		resource_uses: crate::Uses,
		resource_device_accesses: crate::DeviceAccesses,
	) -> crate::AllocationHandle {
		self.device.create_allocation(size, resource_uses, resource_device_accesses)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> crate::MeshHandle {
		self.device
			.add_mesh_from_vertices_and_indices(vertex_count, index_count, vertices, indices, vertex_layout)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<crate::ShaderHandle, ()> {
		self.device
			.create_shader(name, shader_source_type, stage, shader_resource_descriptors)
	}

	fn create_descriptor_set(&mut self, name: Option<&str>) -> crate::DescriptorSetHandle {
		self.device.create_descriptor_set(name)
	}

	fn create_raster_pipeline(&mut self, builder: crate::pipelines::raster::Builder) -> crate::PipelineHandle {
		self.device.create_raster_pipeline(builder)
	}

	fn create_compute_pipeline(&mut self, builder: crate::pipelines::compute::Builder) -> crate::PipelineHandle {
		self.device.create_compute_pipeline(builder)
	}

	fn create_ray_tracing_pipeline(&mut self, builder: crate::pipelines::ray_tracing::Builder) -> crate::PipelineHandle {
		self.device.create_ray_tracing_pipeline(builder)
	}

	fn build_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::BufferHandle<T> {
		self.device.build_buffer(builder)
	}

	fn build_dynamic_buffer<T: Copy>(&mut self, builder: crate::buffer::Builder) -> crate::DynamicBufferHandle<T> {
		self.device.build_dynamic_buffer(builder)
	}

	fn build_dynamic_image(&mut self, builder: crate::image::Builder) -> crate::DynamicImageHandle {
		self.device.build_dynamic_image(builder)
	}

	fn build_image(&mut self, builder: crate::image::Builder) -> crate::ImageHandle {
		self.device.build_image(builder)
	}

	fn build_sampler(&mut self, builder: crate::sampler::Builder) -> crate::SamplerHandle {
		self.device.build_sampler(builder)
	}

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::BaseBufferHandle {
		self.device
			.create_acceleration_structure_instance_buffer(name, max_instance_count)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.device.create_top_level_acceleration_structure(name, max_instance_count)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &crate::BottomLevelAccelerationStructure,
	) -> crate::BottomLevelAccelerationStructureHandle {
		self.device.create_bottom_level_acceleration_structure(description)
	}

	fn create_synchronizer(&mut self, name: Option<&str>, signaled: bool) -> crate::SynchronizerHandle {
		self.device.create_synchronizer(name, signaled)
	}
}
