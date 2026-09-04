//! Loader-thread ownership of the render-resource lifecycle.
//!
//! Each rendering pipeline owns one loader object that hands out requests and receives residency notifications.
//! Model that pipeline's resource families as variants of its key, request, and resident enums instead of
//! creating an independent loader for each resource type. Everything between request and residency
//! points happens on loader threads: file I/O, decoding, staging writes, GPU object creation through a
//! detached [`ghi::implementation::Factory`], and the transfer that makes the resource resident.
//!
//! ```text
//! Pipeline::request(request)  ->  lane: I/O, decode, stage, create objects   (no context lock)
//!                                 lane: commit -> intern and record copies   (short context lock)
//!                                 lane: send Ready { key, resident }
//! Pipeline::poll()            <-  adopt into scene state on the next frame
//! ```
//!
//! # Build an integration
//!
//! 1. Implement [`LoadPipeline`] once for one rendering pipeline. [`LoadPipeline::key`] derives the
//!    pipeline-wide identity used to coalesce each owned [`LoadPipeline::Request`], and
//!    [`LoadPipeline::Resident`] is the finished, already-interned value the render thread adopts.
//! 2. Call [`spawn`] with the renderer's [`SharedContext`](crate::rendering::SharedContext), a staging
//!    arena, and a lane count. Keep the [`LoaderClient`] on the render thread and run every
//!    [`LoaderLane`] on an application-owned async task.
//! 3. Poll [`LoaderClient::poll`] until it returns `None` once per frame and publish each [`Event`] to scene state.
//!
//! # Ownership
//!
//! The client owns request coalescing and nothing else. A lane owns whatever mutable storage the
//! resource is written into, so growing a geometry buffer or assigning a bindless slot is lane work and
//! needs no render-thread round trip. Dependencies a load discovers travel back with its result so the
//! client can coalesce them against everything else in flight.

mod client;
mod lane;

pub use client::{Event, LoaderClient};
pub use lane::{LoadError, LoadPipeline, Loaded, LoaderLane, spawn};
