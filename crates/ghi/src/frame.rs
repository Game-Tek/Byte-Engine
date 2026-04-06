use utils::Extent;

use crate::{
	command_buffer::CommandBufferRecording, descriptors, graphics_hardware_interface, BaseBufferHandle, BaseImageHandle,
	BufferHandle, CommandBufferHandle, DynamicBufferHandle, DynamicImageHandle, PresentKey, SwapchainHandle,
};

/// The `Frame` trait scopes frame-local GPU work so per-frame resources stay tied to an active frame.
/// It exists to use Rust's lifetime system to keep frame operations borrowed from the `Device` for only as long as the frame is active.
pub trait Frame<'a>
where
	Self: Sized + crate::device::DeviceCreate,
{
	/// The command-buffer recording type used while the frame is active.
	type CBR<'f>: CommandBufferRecording
	where
		Self: 'f;

	/// Returns a mutable view into CPU-visible buffer contents for the active frame.
	fn get_mut_buffer_slice<T: Copy>(&self, buffer_handle: BufferHandle<T>) -> &'static mut T;

	/// Flushes or uploads pending writes for the provided buffer.
	fn sync_buffer(&mut self, buffer_handle: impl Into<BaseBufferHandle>);

	/// Returns mutable CPU access to an image's backing bytes for the active frame.
	fn get_texture_slice_mut(&self, texture_handle: BaseImageHandle) -> &'static mut [u8];

	/// Flushes or uploads pending writes for the provided image.
	fn sync_texture(&mut self, image_handle: BaseImageHandle);

	/// Writes descriptor set updates during the active frame.
	fn write(&mut self, descriptor_set_writes: &[descriptors::Write]);

	/// Returns a mutable reference to the dynamic buffer's contents.
	fn get_mut_dynamic_buffer_slice<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>) -> &mut T;

	/// Returns a mutable reference to the dynamic image's contents for the current frame.
	fn get_mut_dynamic_texture_slice(&mut self, image_handle: BaseImageHandle) -> &'static mut [u8] {
		self.get_texture_slice_mut(image_handle)
	}

	/// Resizes an image to the specified extent.
	/// Does nothing if the image is already the specified extent.
	/// May not reallocate if a smaller size is requested.
	fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent);

	/// Creates a new command buffer recording.
	fn create_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) -> Self::CBR<'_>;

	/// Acquires an image from the swapchain as to have it ready for presentation.
	///
	/// # Arguments
	///
	/// * `frame_handle` - The frame to acquire the image for. If `None` is passed, the image will be acquired for the next frame.
	///
	/// # Returns
	/// A present key for future presentation and the extent of the image.
	/// # Errors
	fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> (PresentKey, Extent);

	/// Executes the provided command buffer recording.
	fn execute<'s, 'f>(
		&mut self,
		cbr: <Self::CBR<'f> as CommandBufferRecording>::Result<'s>,
		synchronizer: graphics_hardware_interface::SynchronizerHandle,
	) where
		Self: 'f;
}
