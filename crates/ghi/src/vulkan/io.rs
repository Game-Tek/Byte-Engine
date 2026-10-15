//! Vulkan resource I/O, which the backend does not provide.
//!
//! Vulkan has no native file-to-resource path. Queue creation always fails, and callers fall back to staged
//! uploads through [`crate::command_buffer::CommandBufferRecording`].

use crate::io::{
	ResourceIoCapabilities, ResourceIoContext, ResourceIoError, ResourceIoFileDescriptor, ResourceIoFileHandle,
	ResourceIoImageSourceLayout, ResourceIoQueueDescriptor, ResourceIoRequest, ResourceIoStatus,
};

/// The `ResourceIoQueue` enum names the Vulkan resource-I/O queue type, which has no values.
///
/// [`ResourceIoContext::create_resource_io_queue`] always fails on Vulkan, so load resources with staged uploads
/// instead.
pub enum ResourceIoQueue {}

/// The `ResourceIoTicket` enum names the Vulkan resource-I/O ticket type, which has no values because no
/// [`ResourceIoQueue`] exists to submit a batch.
pub enum ResourceIoTicket {}

impl crate::io::ResourceIoTicket for ResourceIoTicket {
	fn status(&self) -> ResourceIoStatus {
		match *self {}
	}

	fn wait(&self) -> Result<(), ResourceIoError> {
		match *self {}
	}

	fn cancel(&self) -> Result<(), ResourceIoError> {
		match *self {}
	}
}

impl crate::io::ResourceIoQueue for ResourceIoQueue {
	type Ticket = ResourceIoTicket;
	type Context = super::Context;

	fn capabilities(&self) -> ResourceIoCapabilities {
		match *self {}
	}

	fn open_file(&mut self, _descriptor: ResourceIoFileDescriptor<'_>) -> Result<ResourceIoFileHandle, ResourceIoError> {
		match *self {}
	}

	fn submit(
		&mut self,
		_context: &Self::Context,
		_name: Option<&str>,
		_requests: &[ResourceIoRequest],
	) -> Result<Self::Ticket, ResourceIoError> {
		match *self {}
	}
}

impl ResourceIoContext for super::Context {
	type ResourceIoQueue = ResourceIoQueue;

	/// Computes the tightly packed source layout a staged upload of the region reads.
	fn resource_io_image_source_layout(
		&self,
		format: crate::Formats,
		extent: utils::Extent,
	) -> Result<ResourceIoImageSourceLayout, ResourceIoError> {
		let (bytes_per_row, _, bytes_per_image) = format.copy_layout(extent).ok_or(ResourceIoError::InvalidImageLayout)?;
		let total_bytes = bytes_per_image
			.checked_mul(extent.depth().max(1) as usize)
			.filter(|&total_bytes| total_bytes != 0)
			.ok_or(ResourceIoError::InvalidImageLayout)?;
		Ok(ResourceIoImageSourceLayout {
			bytes_per_row,
			bytes_per_image,
			total_bytes,
		})
	}

	fn create_resource_io_queue(
		&mut self,
		_descriptor: ResourceIoQueueDescriptor<'_>,
	) -> Result<Self::ResourceIoQueue, ResourceIoError> {
		Err(ResourceIoError::QueueCreation(
			"the Vulkan backend has no native resource I/O".to_string(),
		))
	}
}
