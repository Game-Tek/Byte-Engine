use utils::Extent;

use crate::{CommandBufferHandle, Device, DynamicBufferHandle, ImageHandle, PresentKey, SwapchainHandle, command_buffer::CommandBufferRecording, graphics_hardware_interface};

/// The `Frame` trait contains methods for performing per frame operations.
/// This trait is used to safely access and manage resources within a frame. This is achieved with Rust's lifetime system by mutably borrowing the `Device` while performing per frame operations.
pub trait Frame where Self: Sized {
	type CBR<'f>: CommandBufferRecording
	where
		Self: 'f;

	/// Returns a mutable reference to the dynamic buffer's contents.
	fn get_mut_dynamic_buffer_slice<T: Copy>(&self, buffer_handle: DynamicBufferHandle<T>) -> &mut T;

	/// Resizes an image to the specified extent.
	/// Does nothing if the image is already the specified extent.
	/// May not reallocate if a smaller size is requested.
	fn resize_image(&mut self, image_handle: ImageHandle, extent: Extent);

	/// Creates a new command buffer recording.
	fn create_command_buffer_recording<'f>(&'f mut self, command_buffer_handle: CommandBufferHandle) -> Self::CBR<'f>;

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

	fn device(&mut self) -> &mut Device; // TODO: can lead to nasty stuff, analyze how to remove

	/// Executes the provided command buffer recording.
	fn execute<'f>(&'f mut self, command_buffer_recording: Self::CBR<'f>, present_keys: &[graphics_hardware_interface::PresentKey], synchronizer: graphics_hardware_interface::SynchronizerHandle);
}
