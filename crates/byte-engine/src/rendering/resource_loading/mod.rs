//! Shared asynchronous render-resource loading and GPU upload lifecycle.
//!
//! This module solves the parts every renderer needs: stable request identity,
//! duplicate coalescing, bounded client/server queues, retry-safe completion
//! routing, exclusive staging memory, and GPU-frame lifetime tracking. It does
//! not define a universal GPU resource or storage layout. The renderer keeps
//! that policy by defining its [`RenderResource::Prepared`] value and
//! implementing [`ResourceUploadStore`].
//!
//! # Build a renderer integration
//!
//! Work from the renderer toward the shared lifecycle:
//!
//! 1. Implement [`RenderResource`] as the protocol for one renderer. Keep
//!    logical identity in [`RenderResource::Key`], owned worker input in
//!    [`RenderResource::Request`], and storage-independent results in
//!    [`RenderResource::Prepared`].
//! 2. Implement [`ResourcePreparer`] for resource I/O and CPU conversion. A
//!    preparer may create detached factory resources, but it must not assign
//!    renderer buffer offsets, bindless slots, or other resident identities.
//!    For a baked 2D texture, use [`PreparedTextureTransfer`] to share mip,
//!    staging, and native-I/O mechanics without sharing image or sampler policy.
//! 3. Create a [`ResourceLoader`] on the render thread. Convert its
//!    [`ResourceLoadingEndpoint`] into one or more [`ResourceLoadingServer`]
//!    values, and run each server on an application-owned async task.
//! 4. At [`crate::rendering::PipelineManager::begin_frame`], submit queued
//!    requests and drain [`ResourceCompletion`] values. Publish results that
//!    need no GPU work with [`ResourceLoader::mark_ready`]. Enqueue transfer
//!    work in a [`FrameUploadQueue`].
//! 5. At [`crate::rendering::PipelineManager::record_frame_uploads`], pass the
//!    queue to the renderer's [`ResourceUploadStore`]. The store chooses the
//!    destination objects, memory layout, offsets, table slots, and resident
//!    handle.
//! 6. At the next matching frame completion, call
//!    [`FrameUploadQueue::retire_frame`]. Only then publish its
//!    `(token, resident)` values to scene rendering.
//!
//! # Follow one request back to the renderer
//!
//! The reverse direction explains who calls renderer code and why:
//!
//! - [`ResourceLoader::submit_requests`] transfers owned requests to a server
//!   without waiting on the render thread.
//! - [`ResourceLoadingServer::run`] calls [`ResourcePreparer::prepare`] on its
//!   own sequential lane. Clone the endpoint when independent lanes should
//!   compete for work; give every lane its own preparer state.
//! - [`ResourceLoader::take_completion`] returns only the current request
//!   revision. Cancelled and superseded work is discarded before it can reach
//!   renderer storage.
//! - [`FrameUploadQueue::record_frame`] calls [`ResourceUploadStore::record`]
//!   while recording transfer commands. This is the deliberate policy seam:
//!   two renderers can load the same logical mesh and still choose unrelated
//!   GPU layouts.
//! - [`FrameUploadQueue::retire_frame`] returns the store's resident value only
//!   after the exact frame that used its upload data has completed.
//!
//! # Ownership and thread placement
//!
//! Keep [`ResourceLoader`], [`FrameUploadQueue`], and the renderer store on the
//! render thread. Move each [`ResourceLoadingServer`] and its
//! [`ResourcePreparer`] to an async task. Share [`UploadStagingArena`] with
//! preparers, but run its single [`UploadStagingWorker`] on an async task. This
//! arrangement keeps synchronization and task ownership above GHI while the
//! queue retains [`StagingLease`] values for the complete GPU-use interval.
//!
//! The application must stop and join loading tasks before dropping the
//! renderer, its GHI context, or the mapped upload buffer. Dropping the loader
//! closes the request side; servers then finish or stop, dropping every arena
//! client closes the staging worker, and completed upload values return their
//! leases automatically.
//!
//! # Choose a GPU creation path
//!
//! Use [`FrameUploadQueue`] when a render-thread command recording writes the
//! resident resource. A preparer can also create detached GHI factory objects;
//! the render thread interns them before queueing their transfer. Native GPU
//! I/O may bypass the queue, but it must use the same lifecycle contract: call
//! [`ResourceLoader::mark_uploading`] before submission, then
//! [`ResourceLoader::mark_ready`] only after the native completion makes the
//! resident usable. Call [`ResourceLoader::mark_failed`] when adoption or
//! native I/O cannot finish.
//!
//! # Failure, cancellation, and retry
//!
//! A [`ResourceRef`] identifies the logical resource for the lifetime of one
//! loader. A [`ResourceToken`] adds a revision so late work cannot publish over
//! a retry. Cancel only [`ResourceState::Queued`] or
//! [`ResourceState::Loading`] work. Once storage or GPU I/O claims a resource
//! as [`ResourceState::Uploading`], cleanup belongs to the renderer because the
//! shared lifecycle cannot undo implementation-specific allocation. Retry
//! failed or cancelled references with [`ResourceLoader::retry`]; dependent
//! resources remain a renderer concern because only that renderer knows its
//! material, texture, mesh, or environment graph.
//!
//! # Minimal protocol
//!
//! ```no_run
//! use std::future::Future;
//! use byte_engine::rendering::resource_loading::{
//!     RenderResource, ResourceLoader, ResourcePreparer, ResourceUploadStore,
//! };
//!
//! enum MeshResource {}
//!
//! impl RenderResource for MeshResource {
//!     type Key = &'static str;
//!     type Request = &'static str;
//!     type Prepared = Vec<u8>;
//!     type Error = String;
//! }
//!
//! struct MeshPreparer;
//!
//! impl ResourcePreparer<MeshResource> for MeshPreparer {
//!     fn prepare(
//!         &mut self,
//!         request: &'static str,
//!     ) -> impl Future<Output = Result<Vec<u8>, String>> + '_ {
//!         async move { Ok(request.as_bytes().to_vec()) }
//!     }
//! }
//!
//! struct MeshStore;
//!
//! impl ResourceUploadStore for MeshStore {
//!     type Upload = Vec<u8>;
//!     type Resident = usize;
//!     type Error = String;
//!
//!     fn record(
//!         &mut self,
//!         _recording: &mut byte_engine::ghi::implementation::CommandBufferRecording<'_>,
//!         upload: &Self::Upload,
//!     ) -> Result<Self::Resident, Self::Error> {
//!         // This renderer chooses the destination buffers, offsets, and resident ID.
//!         Ok(upload.len())
//!     }
//! }
//!
//! let (mut loader, endpoint) = ResourceLoader::<MeshResource>::new(4_096, 64);
//! let server = endpoint.server(MeshPreparer);
//! let mesh = loader.request("scene.mesh", "scene.mesh").expect("resource capacity");
//! loader.submit_requests(64);
//!
//! // Run `server.run()` on an application-owned async task. At frame
//! // boundaries, call `loader.take_completion()` and adopt its result.
//! let _ = (mesh, server);
//! ```

mod loader;
pub(crate) mod texture;
mod upload_queue;
mod upload_staging;

pub use loader::{
	RenderResource, ResourceCompletion, ResourceLoader, ResourceLoadingEndpoint, ResourceLoadingServer, ResourcePreparer,
	ResourceRef, ResourceState, ResourceToken,
};
pub use texture::{
	NativeTextureUpload, PreparedTextureSource, PreparedTextureTransfer, StagedTextureUpload, TextureMetadata,
	TexturePreparationError,
};
pub use upload_queue::{FrameUploadQueue, ResourceUploadStore};
pub use upload_staging::{StagingLease, UploadStagingArena, UploadStagingWorker};
