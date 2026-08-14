//! DX12 device operations for transfers.

use super::*;

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
