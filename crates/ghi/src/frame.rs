use utils::Extent;

use crate::{
	command_buffer::CommandBufferRecording, descriptors, BaseBufferHandle, BaseImageHandle, BufferHandle, CommandBufferHandle,
	DynamicBufferHandle, PresentKey, SwapchainHandle,
};

/// The `Frame` trait scopes frame-local GPU work so per-frame resources stay tied to an active frame.
/// The frame lifetime keeps operations borrowed from the [`crate::Device`] only
/// while the frame is active.
pub trait Frame<'a>
where
	Self: Sized + crate::context::ContextCreate,
{
	/// The command-buffer recording type used while the frame is mutably borrowed for recording.
	type CBR<'record>: CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode
	where
		Self: 'record;

	/// Returns the frame key for the active frame.
	fn key(&self) -> crate::FrameKey;

	/// Returns a mutable view into CPU-visible buffer contents for the active frame.
	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T;

	/// Flushes or uploads pending writes for the provided buffer.
	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>);

	/// Returns mutable CPU access to an image's backing bytes for the active frame.
	fn get_texture_slice_mut(&self, texture_handle: BaseImageHandle) -> &'static mut [u8];

	/// Flushes or uploads pending writes for the provided image.
	fn sync_texture(&mut self, image_handle: BaseImageHandle);

	/// Writes descriptor set updates during the active frame.
	fn write(&mut self, descriptor_set_writes: &[descriptors::DescriptorWrite]);

	/// Returns a mutable reference to the dynamic buffer's contents.
	fn get_mut_dynamic_buffer_slice<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>) -> &mut T;

	/// Returns a mutable reference to the dynamic image's contents for the current frame.
	fn get_mut_dynamic_texture_slice(&mut self, image_handle: BaseImageHandle) -> &'static mut [u8] {
		self.get_texture_slice_mut(image_handle)
	}

	/// Resizes an image to the specified extent.
	/// This method has no effect when the image already has the requested extent.
	/// A smaller extent does not always require reallocation.
	fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent);

	/// Creates a new command buffer recording.
	fn create_command_buffer_recording<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record>;

	/// Creates a command-buffer recording without pending frame synchronization work.
	///
	/// Use this method for explicit transfer or maintenance work. The regular frame
	/// path can prepend dynamic-resource uploads that helper queues must not replay.
	fn create_command_buffer_recording_without_implicit_sync<'record>(
		&'record mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> Self::CBR<'record> {
		self.create_command_buffer_recording(command_buffer_handle)
	}

	/// Acquires a swapchain image for presentation.
	///
	/// Returns a presentation key and the image extent.
	/// # Errors
	fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> (PresentKey, Extent);
}
