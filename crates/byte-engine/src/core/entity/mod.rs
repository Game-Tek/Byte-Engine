//! Shared entity ownership used across subsystem boundaries.
//!
//! Convert concrete values into [`EntityHandle`] when several systems need
//! stable access to the same object. Trait-object handles are used by
//! [`crate::gameplay::world::DefaultWorld`] for physics bodies and renderable
//! meshes.

pub mod container;
pub mod handle;

pub use container::Container as EntityContainer;
pub use handle::Handle as EntityHandle;
use utils::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The [`Entity`] trait marks values that participate in engine-managed shared
/// ownership.
pub trait Entity {}

use std::ops::CoerceUnsized;
use std::{marker::Unsize, ops::Deref};

use super::listener::Listener;
use super::Task;

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
}
