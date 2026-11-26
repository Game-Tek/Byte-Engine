//! Shared entity ownership used across subsystem boundaries.
//!
//! Convert concrete values into [`EntityHandle`] when several systems need
//! stable access to the same object. Trait-object handles are used by
//! headed applications for physics bodies and renderable meshes.

pub mod handle;

pub use handle::Handle as EntityHandle;
