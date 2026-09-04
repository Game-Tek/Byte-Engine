//! Resource Manager to GHI utilities for renderer loader lanes.
//!
//! [`crate::rendering::loading`] owns request coalescing, loader tasks, detached
//! factories, context commits, and completed residency events. This module owns
//! the reusable byte-transfer pieces those loaders build on: upload staging and
//! ordinary sampled-texture creation.
//!
//! Most pipelines request an image through the Resource Manager and call
//! [`TextureTransfer::load`]. Keep request identity, bindless slots, and resident
//! publication in the renderer-specific
//! [`crate::rendering::loading::LoadPipeline`] implementation.

pub(crate) mod texture;
mod upload_staging;

pub use texture::{LoadedTexture, TextureAddressMode, TextureDescriptor, TextureTransfer, TextureTransferError};
pub use upload_staging::{StagingLease, UploadStagingArena, UploadStagingWorker};
