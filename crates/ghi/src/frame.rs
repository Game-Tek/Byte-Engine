use std::time::Instant;

use utils::Extent;

use crate::{
	BaseImageHandle, BufferHandle, CommandBufferHandle, PresentKey, SwapchainHandle, command_buffer::CommandBufferRecording,
	descriptors,
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

	/// Returns this frame's copy of a buffer the CPU writes.
	///
	/// Starting the frame waited for the last frame that used this copy, so no submitted work reads it. Work
	/// submitted after the write reads what the CPU wrote, with no further call. A copy keeps what was last written
	/// to it, so write everything the frame reads.
	///
	/// # Panics
	///
	/// Panics unless the buffer was built with [`crate::HostAccess::Write`] and [`crate::Copies::PerFrame`].
	fn map<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>) -> &mut T;

	/// Returns the copy of a buffer the CPU reads that the frame which completed when this frame started wrote.
	///
	/// Starting a frame waits for the frame of the same sequence before it, so that frame's copy holds what its
	/// commands wrote until this frame's commands run. Returns `None` while the first frames of each sequence start.
	///
	/// # Panics
	///
	/// Panics unless the buffer was built with [`crate::HostAccess::Read`] and [`crate::Copies::PerFrame`].
	fn read<T: ?Sized + crate::buffer::BufferContents>(&self, buffer_handle: BufferHandle<T>) -> Option<&T>;

	/// Writes descriptor set updates during the active frame.
	fn write(&mut self, descriptor_set_writes: &[descriptors::DescriptorWrite]);

	/// Resizes a buffer to `length` elements, replacing every copy with a new one that starts zero-initialized.
	///
	/// Work recorded before the resize keeps the old copies until it completes, so a frame can resize a buffer and
	/// then map its new copy. Next, write it with [`Self::map`].
	fn resize_buffer<T: ?Sized + crate::buffer::BufferContents>(&mut self, buffer_handle: BufferHandle<T>, length: usize);

	/// Resizes an image to the specified extent.
	/// This method has no effect when the image already has the requested extent.
	/// A smaller extent does not always require reallocation.
	fn resize_image(&mut self, image_handle: BaseImageHandle, extent: Extent);

	/// Resizes an array image to `extent` and `array_layers` layers.
	///
	/// Use this for images whose layer count follows a per-frame budget, such as a pool of shadow maps. The image
	/// keeps its handle, so descriptors that reference it see the new storage. The new storage starts without valid
	/// contents. This method has no effect when the image already has the requested extent and layer count. For a
	/// cube-array image, `array_layers` counts faces and must be a multiple of six.
	fn resize_image_layers(&mut self, image_handle: BaseImageHandle, extent: Extent, array_layers: std::num::NonZeroU32);

	/// Sizes every member of `group` and gives each one memory, sharing memory between members whose lifetimes do
	/// not overlap.
	///
	/// List every member exactly once. The call does nothing when no extent or lifetime changed since the last one.
	/// Otherwise every member is recreated and starts without valid contents, so initialize each member before its
	/// first read: clear it, render to it with [`crate::LoadOp::Clear`] or [`crate::LoadOp::Discard`], or call
	/// [`crate::command_buffer::CommandBufferRecording::discard_images`]. Do that every frame, since another member
	/// may reuse the memory in between.
	fn place_image_group(&mut self, group: crate::ImageGroupHandle, members: &[crate::ImageGroupMember]);

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

	/// Acquires a swapchain image for presentation from inside a started frame.
	///
	/// Use [`crate::context::Context::acquire_swapchain_image`] to acquire before the frame is started.
	/// Returns `None` when the presentation engine has no image to give; skip presenting that swapchain this frame.
	fn acquire_swapchain_image(&mut self, swapchain_handle: SwapchainHandle) -> Option<SwapchainAcquisition>;
}

/// The `SwapchainAcquisition` struct provides the image information needed to prepare a frame for presentation.
/// Use [`Self::present_key`] when submitting the frame through [`crate::queue::Queue::execute`].
#[derive(Clone, Copy, Debug)]
pub struct SwapchainAcquisition {
	pub(crate) present_key: PresentKey,
	pub(crate) extent: Extent,
	/// The display time of the most recently presented image of this swapchain, when the backend reports it.
	pub(crate) present_time: Option<Instant>,
	/// The time the display will show the image acquired here, when the backend schedules frames ahead of the display.
	pub(crate) target_present_time: Option<Instant>,
}

impl SwapchainAcquisition {
	pub fn present_key(&self) -> PresentKey {
		self.present_key
	}

	pub fn extent(&self) -> Extent {
		self.extent
	}

	/// Returns the display time of the most recently presented image of this swapchain.
	///
	/// Backends without presentation feedback return `None`.
	pub fn present_time(&self) -> Option<Instant> {
		self.present_time
	}

	/// Returns the time the display will show the image this acquisition gives, if the frame presents in time.
	///
	/// Only backends that schedule frames ahead of the display report it, such as Metal for a display-synced
	/// swapchain; others return `None`. Use it to simulate to the moment the frame is
	/// seen, and fall back to [`Self::present_time`] when it is missing.
	pub fn target_present_time(&self) -> Option<Instant> {
		self.target_present_time
	}
}
