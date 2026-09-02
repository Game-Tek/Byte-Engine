//! DX12 device operations for transfers.

use super::*;

/// The `NativeTextureCopyFootprint` struct keeps the driver-independent buffer layout for one texture subresource.
struct NativeTextureCopyFootprint {
	placed: D3D12_PLACED_SUBRESOURCE_FOOTPRINT,
	row_count: usize,
	row_size: usize,
	total_size: usize,
}

impl Device {
	/// Queries the exact staging-buffer layout required to copy one native texture subresource.
	fn native_texture_copy_footprint(&self, resource: &ID3D12Resource, subresource: u32) -> Option<NativeTextureCopyFootprint> {
		// SAFETY: `resource` is a live COM interface retained by the caller for this query.
		let descriptor = unsafe { resource.GetDesc() };
		let mut placed = D3D12_PLACED_SUBRESOURCE_FOOTPRINT::default();
		let mut row_count = 0;
		let mut row_size = 0;
		let mut total_size = 0;
		// SAFETY: Every output points to initialized storage for the one requested subresource.
		unsafe {
			self.device.GetCopyableFootprints(
				&descriptor,
				subresource,
				1,
				0,
				Some(&mut placed),
				Some(&mut row_count),
				Some(&mut row_size),
				Some(&mut total_size),
			);
		}
		if total_size == u64::MAX || placed.Offset != 0 || row_count == 0 || row_size == 0 || placed.Footprint.Depth == 0 {
			return None;
		}

		Some(NativeTextureCopyFootprint {
			placed,
			row_count: usize::try_from(row_count).ok()?,
			row_size: usize::try_from(row_size).ok()?,
			total_size: usize::try_from(total_size).ok()?,
		})
	}
}

#[path = "transfers/attachments.rs"]
mod attachments;
#[path = "transfers/buffers.rs"]
mod buffers;
#[path = "transfers/descriptors.rs"]
mod descriptors;
#[path = "transfers/image_data.rs"]
mod image_data;
#[path = "transfers/image_operations.rs"]
mod image_operations;
#[path = "transfers/image_uploads.rs"]
mod image_uploads;
#[path = "transfers/resource_updates.rs"]
mod resource_updates;
#[path = "transfers/resources.rs"]
mod resources;
#[path = "transfers/swapchains.rs"]
mod swapchains;
#[path = "transfers/synchronization.rs"]
mod synchronization;
