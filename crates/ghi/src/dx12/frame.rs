use utils::Extent;

use crate::context::Context as _;
use crate::{BaseImageHandle, BufferHandle, CommandBufferHandle, FrameKey, ImageHandle, SwapchainHandle, SynchronizerHandle};

pub struct Frame<'a> {
	frame_key: FrameKey,
	synchronizer: SynchronizerHandle,
	device: &'a mut super::Device,
}

impl<'a> Frame<'a> {
	pub(crate) fn new(device: &'a mut super::Device, frame_key: FrameKey, synchronizer: SynchronizerHandle) -> Self {
		Self {
			frame_key,
			synchronizer,
			device,
		}
	}

	/// Interns a raster pipeline a factory compiled. Interning only adds it to the context, so it doesn't stall the
	/// frame.
	pub fn intern_raster_pipeline(&mut self, pipeline: crate::implementation::RasterPipeline) -> crate::PipelineHandle {
		self.device.intern_pipeline(pipeline.0)
	}

	/// Interns a compute pipeline a factory compiled. See [`Self::intern_raster_pipeline`].
	pub fn intern_compute_pipeline(&mut self, pipeline: crate::implementation::ComputePipeline) -> crate::PipelineHandle {
		self.device.intern_pipeline(pipeline.0)
	}

	/// Interns a ray tracing pipeline a factory linked. See [`Self::intern_raster_pipeline`].
	pub fn intern_ray_tracing_pipeline(&mut self, pipeline: crate::dx12::factory::RayTracingPipeline) -> crate::PipelineHandle {
		self.device.intern_pipeline(pipeline.0)
	}

	/// Interns an image another context exported. See [`super::Device::intern_image`].
	pub fn intern_image(&mut self, image: crate::dx12::factory::DetachedImage) -> crate::ImageHandle {
		self.device.intern_image(image)
	}

	/// Deletes an image once every frame started so far has completed. The handle is invalid afterwards.
	///
	/// # Panics
	///
	/// Panics when the handle names no image of this context, an image with one representation per frame, or an
	/// image-group member.
	pub fn delete_image(&mut self, image: crate::ImageHandle) {
		self.device.delete_image(image, self.frame_key.sequence_index);
	}

	pub fn device(&mut self) -> &mut super::Device {
		self.device
	}
}

impl<'a> crate::frame::Frame<'a> for Frame<'a> {
	type CBR<'record>
		= super::CommandBufferRecording<'record>
	where
		Self: 'record;

	fn key(&self) -> FrameKey {
		self.frame_key
	}

	fn map<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>) -> &mut T {
		self.device.map_for_sequence(buffer_handle, self.frame_key.sequence_index)
	}

	fn read<T: ?Sized + crate::buffer::BufferContents>(&self, buffer_handle: BufferHandle<T>) -> Option<&T> {
		// The frame that completed when this one started is the one before it in this frame's sequence.
		let completed = self.frame_key.frame_index >= u64::from(self.device.frames);
		self.device
			.read_for_sequence(buffer_handle, self.frame_key.sequence_index, completed)
	}

	fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>, length: usize) {
		self.device.resize_buffer(buffer_handle, length);
	}

	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		self.device.write(descriptor_set_writes);
	}

	fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent) {
		self.device.image_groups.assert_resizable(image_handle);
		self.device
			.resize_image_internal(ImageHandle(image_handle), extent, None, self.frame_key.sequence_index);
	}

	fn resize_image_layers(&mut self, image_handle: BaseImageHandle, extent: Extent, array_layers: std::num::NonZeroU32) {
		// DX12 images keep no cube-array flag; their cube views come from the descriptor that binds them.
		self.device.image_groups.assert_resizable(image_handle);
		self.device.resize_image_internal(
			ImageHandle(image_handle),
			extent,
			Some(array_layers.get()),
			self.frame_key.sequence_index,
		);
	}

	fn place_image_group(&mut self, group: crate::ImageGroupHandle, members: &[crate::ImageGroupMember]) {
		self.device.place_image_group(group, members);
	}

	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.device
			.begin_command_buffer(command_buffer_handle, self.frame_key.sequence_index);
		self.device
			.set_command_buffer_frame_synchronizer(command_buffer_handle, self.synchronizer);
		self.device.record_ready_compactions(command_buffer_handle);
		super::CommandBufferRecording::new(self.device, command_buffer_handle, Some(self.frame_key))
	}

	fn create_command_buffer_recording_without_implicit_sync<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.device
			.begin_command_buffer(command_buffer_handle, self.frame_key.sequence_index);
		self.device
			.set_command_buffer_frame_synchronizer(command_buffer_handle, self.synchronizer);
		super::CommandBufferRecording::new(self.device, command_buffer_handle, Some(self.frame_key))
	}

	/// Acquires a backbuffer from inside the started frame. The sequence fences were already waited by `start_frame`.
	fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> Option<crate::frame::SwapchainAcquisition> {
		self.device
			.acquire_swapchain_image_for_sequence(self.frame_key.sequence_index, swapchain_handle)
	}
}

impl<'a> crate::context::ContextCreate for Frame<'a> {
	crate::context::delegate_context_create_to_device!();
}
