pub mod container;
pub mod handle;

pub use container::Container as EntityContainer;
pub use handle::Handle as EntityHandle;

use utils::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// The Entity trait is the base trait for all entities in the engine.
///
/// An entity is a type that can be spawned and managed by the engine.
/// The trait provides some convenience methods to interact with the entity.
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
