pub mod handle;
pub mod container;

pub use handle::Handle as EntityHandle;
pub use container::Container as EntityContainer;

use utils::{sync::{RwLock, Arc, RwLockReadGuard, RwLockWriteGuard}};

/// The Entity trait is the base trait for all entities in the engine.
///
/// An entity is a type that can be spawned and managed by the engine.
/// The trait provides some convenience methods to interact with the entity.
pub trait Entity {
}

use std::{marker::Unsize, ops::Deref};
use std::ops::CoerceUnsized;

use super::{Task};
use super::{listener::Listener};

pub trait MapAndCollectAsAvailable<T: ?Sized, U> {
	/// Maps the entities in the vector and collects them into a new vector but skips taken locks until they are available.
	/// This avoids stalling the thread if a lock is taken.
	/// Order of the elements is **not** preserved.
	fn map_and_collect_as_available(&self, function: impl FnMut(&T) -> U) -> Vec<U>;
}

impl <T: ?Sized, U> MapAndCollectAsAvailable<T, U> for Vec<EntityHandle<T>> {
	fn map_and_collect_as_available(&self, mut function: impl FnMut(&T) -> U) -> Vec<U> {
		let mut source = (0..self.len()).collect::<Vec<_>>();
		let mut res = Vec::with_capacity(self.len());

		while !source.is_empty() {
			source.retain(|i| {
				let e = &self[*i];

				if let Some(b) = e.try_read() {
					res.push(function(&b));
					false
				} else {
					true
				}
			});
		}

		res
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
}
