pub mod builder;
pub mod handle;

pub use builder::Builder as EntityBuilder;
pub use handle::Handle as EntityHandle;

use utils::{sync::{RwLock, Arc, RwLockReadGuard, RwLockWriteGuard}};

use crate::core::spawn_as_child;
use crate::gameplay::space::{Space, Spawner};

/// The Entity trait is the base trait for all entities in the engine.
///
/// An entity is a type that can be spawned and managed by the engine.
/// The trait provides some convenience methods to interact with the entity.
pub trait Entity: downcast_rs::Downcast + std::any::Any + 'static {
	/// Create an entity builder for this entity.
	/// This is a convenience method to create an entity builder with the current "unwrapped" entity as the base.
	///
	/// Implementations of this trait should override this method to provide a custom entity builder, such as one that listens to events, or produces creation events.
	///
	/// The default implementation will create a new `EntityBuilder` with the current entity as the base and will produce a creation event as the type of the entity itself.
	///
	/// # Examples
	///
	/// ```rust
	/// use byte_engine::core::{Entity, EntityBuilder};
	///
	/// struct MyEntity {}
	///
	/// impl MyEntity {
	/// 	fn new() -> Self {
	/// 		MyEntity {}
	/// 	}
	/// }
	///
	/// impl Entity for MyEntity {
	/// 	fn builder(self) -> EntityBuilder<'static, Self> {
	/// 		EntityBuilder::new(self).r#as::<MyEntity>(|e| e) // Produces a creation event as `MyEntity`
	/// 	}
	/// }
	///
	/// fn main() {
	/// 	let entity = MyEntity::new(); // Choose how to create the entity
	/// 	let builder = entity.builder(); // Make a builder for extra functionality when spawning
	/// 	// Do something with the builder...
	/// }
	/// ```
	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self).r#as(|h| h)
	}
}

pub unsafe fn get_entity_trait_for_type<T: ?Sized + 'static>() -> EntityTrait {
	EntityTrait {
		trait_id: std::any::TypeId::of::<T>(),
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityTrait {
	trait_id: std::any::TypeId,
}

pub trait SelfDestroyingEntity: Entity {
	fn destroy(&self);
}

downcast_rs::impl_downcast!(Entity);

use std::{marker::Unsize, ops::Deref};
use std::ops::CoerceUnsized;

use super::domain::{Domain, DomainEvents};
use super::event::{Event, EventRegistry};
use super::listener::CreateEvent;
use super::{spawn, Task};
use super::{listener::Listener, SpawnHandler};

pub type DomainType = EntityHandle<dyn Domain>;

pub trait PostCreationFunction<T> = FnOnce(DomainType, EntityHandle<T>);

pub(crate) enum EntityEvents<T> {
	As { f: Box<dyn Fn(EntityHandle<T>, &mut Vec<DomainEvents>)> },
	Listen { f: Box<dyn Fn(EntityHandle<T>, &mut Vec<DomainEvents>)> },
}

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
	use crate::core::{listener::CreateEvent, spawn};

	#[test]
	fn spawn_entities() {
		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> System {
				System {}
			}
		}

		impl Listener<CreateEvent<Component>> for System {
			fn handle(&mut self, event: &CreateEvent<Component>) {
				println!("Component created");
			}
		}

		let _: EntityHandle<System> = spawn(System::new());

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });
	}
}
