//! Shared entity ownership used across subsystem boundaries.
//!
//! Convert concrete values into [`EntityHandle`] when several systems need
//! stable access to the same object. Trait-object handles are used by
//! the headed default world for physics bodies and renderable meshes.

pub mod handle;

pub use handle::Handle as EntityHandle;

/// The [`Entity`] trait marks values that participate in engine-managed shared
/// ownership.
pub trait Entity {}
