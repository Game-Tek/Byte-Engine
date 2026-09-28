//! Loading runs on its own timeline, parallel to rendering.
//!
//! Read the [loading timeline design](/docs/develop/rendering/loading) before you change this module. It lists the
//! rules that keep loading from ever blocking rendering, and the tests that guard them.
//!
//! The [`Loader`] owns a context and the copy queue of the [`GraphicsDevice`](crate::rendering::GraphicsDevice).
//! The renderer owns another context and the graphics queue. The two never wait on each other: the render thread
//! learns about loaded resources only through each pipeline's [`LoaderClient`] notifications.
//!
//! Each rendering pipeline owns one loader client that hands out requests and receives residency notifications.
//! Model that pipeline's resource families as variants of its key, request, and resident enums instead of
//! creating an independent client for each resource type. Everything between request and residency happens on the
//! loading thread: file I/O, decoding, staging writes, and the batched copies that make the resource resident.
//!
//! ```text
//! Pipeline::request(request)  ->  lane: read metadata, request dependencies  ->  other lanes
//!                                 lane: I/O, decode, write staging
//!                                 lane: upload (plain data)  ->  loader: batch, submit on copy queue, wait
//!                                 lane: send Ready { key, resident with detached images }
//! Pipeline::poll()            <-  render thread: intern images, adopt into scene state
//! ```
//!
//! # Build an integration
//!
//! 1. Implement [`LoadPipeline`] once for one rendering pipeline. [`LoadPipeline::key`] derives the
//!    pipeline-wide identity used to coalesce each owned [`LoadPipeline::Request`], and
//!    [`LoadPipeline::Resident`] is the finished value the render thread adopts.
//! 2. Share every render buffer the lanes append to with
//!    [`share_buffer`](ghi::implementation::Context::share_buffer), import it with [`Loader::import_buffer`], and
//!    call [`spawn`] with the loader and a lane count. Keep the [`LoaderClient`] on the render thread and run every
//!    [`LoaderLane`] on the loading thread.
//! 3. Poll [`LoaderClient::poll`] until it returns `None` once per frame, intern the detached images each
//!    [`Event`] carries, and publish it to scene state.
//!
//! # Ownership
//!
//! The client sends requests and publishes results. A lane owns whatever CPU bookkeeping the resource is written
//! into, so assigning a bindless slot or an append offset is lane work and needs no render-thread round trip. Lanes
//! request the dependencies a load discovers through [`LoaderLane::request`] as soon as its metadata names them, and
//! coalesce them with the client's requests where they pick requests up. The loader owns all GPU work of the
//! loading timeline.

mod client;
mod lane;
mod loader;

pub use client::{Event, LoaderClient};
pub use lane::{LoadError, LoadPipeline, LoaderLane, spawn};
pub use loader::{BufferRegion, ImageDescription, ImageRegion, ImageUpload, Loader, NativeImageRegion, NativeImageUpload};
