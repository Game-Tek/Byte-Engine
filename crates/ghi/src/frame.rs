use utils::Extent;

use crate::{
	command_buffer::CommandBufferRecording, graphics_hardware_interface, BufferHandle, CommandBufferHandle,
	DynamicBufferHandle, ImageHandle, PresentKey, SwapchainHandle,
};

/// The `Frame` trait contains methods for performing per frame operations.
/// This trait is used to safely access and manage resources within a frame. This is achieved with Rust's lifetime system by mutably borrowing the `Device` while performing per frame operations.
pub trait Frame<'a>
where
	Self: Sized,
{
	type CBR<'f>: CommandBufferRecording
	where
		Self: 'f;

	// Return a mutable slice to the buffer data.
	fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: BufferHandle<T>) -> &mut T;

	/// Returns a mutable reference to the dynamic buffer's contents.
	fn get_mut_dynamic_buffer_slice<T: Copy>(&mut self, buffer_handle: DynamicBufferHandle<T>) -> &mut T;

	/// Resizes an image to the specified extent.
	/// Does nothing if the image is already the specified extent.
	/// May not reallocate if a smaller size is requested.
	fn resize_image(&mut self, image_handle: ImageHandle, extent: Extent);

	/// Creates a new command buffer recording.
	fn create_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) -> Self::CBR<'_>;

	/// Acquires an image from the swapchain as to have it ready for presentation.
	///
	/// # Arguments
	///
	/// * `frame_handle` - The frame to acquire the image for. If `None` is passed, the image will be acquired for the next frame.
	///
	/// # Returns
	/// A present key for future presentation and, if defined, the extent of the image.
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
