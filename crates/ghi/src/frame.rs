use utils::Extent;

use crate::{CommandBufferHandle, CommandBufferRecording, Device, DynamicBufferHandle, ImageHandle, PresentKey, SwapchainHandle};

/// The `Frame` trait contains methods for performing per frame operations.
/// This trait is used to safely access and manage resources within a frame. This is achieved with Rust's lifetime system by mutably borrowing the `Device` while performing per frame operations.
pub trait Frame {
	/// Returns a mutable reference to the dynamic buffer's contents.
	fn get_mut_dynamic_buffer_slice<'a, T: Copy>(&'a self, buffer_handle: DynamicBufferHandle<T>) -> &'a mut T;

	/// Resizes an image to the specified extent.
	/// Does nothing if the image is already the specified extent.
	/// May not reallocate if a smaller size is requested.
	fn resize_image(&mut self, image_handle: ImageHandle, extent: Extent);

	/// Creates a new command buffer recording.
	fn create_command_buffer_recording(&mut self, command_buffer_handle: CommandBufferHandle) -> CommandBufferRecording<'_>;

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

	fn device(&mut self) -> &mut Device;
}
