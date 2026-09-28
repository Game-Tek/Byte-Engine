//! Resource Manager to GHI utilities for loader lanes.
//!
//! [`crate::rendering::loading`] owns request coalescing, lanes, the loader timeline, and completed residency
//! events. This module owns the reusable byte-preparation pieces lanes build on: upload staging and ordinary
//! sampled-texture loading.
//!
//! Most pipelines request an image through the Resource Manager and call [`load_texture`]. Keep request identity,
//! bindless slots, samplers, and resident publication in the renderer-specific
//! [`crate::rendering::loading::LoadPipeline`] implementation.

pub(crate) mod texture;
mod upload_staging;

pub use texture::{TextureTransferError, load_texture};
pub(crate) use upload_staging::UploadStagingWorker;
pub use upload_staging::{StagingLease, UploadStagingArena};
