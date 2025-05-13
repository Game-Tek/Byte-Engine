pub mod orchestrator;

pub mod entity;
pub mod domain;
pub mod property;
pub mod event;
pub mod listener;

pub mod task;

use std::ops::Deref;

use domain::Domain;
pub use orchestrator::Orchestrator;
use listener::Listener;

use entity::DomainType;
use entity::EntityBuilder;

pub use entity::Entity;
pub use entity::EntityHandle;

pub use task::Task;

use utils::sync::{Arc, RwLock};

use crate::gameplay::space::Spawner;

static mut COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// IMPORTANT: `spawn` will not call `Entity::call_listeners` on `entity` as it does not take a domain and does not expect `Entity` derived objects as a paramenter.
/// IMPORTANT: `spawn` will not set entities as listeners of another as it does not take a domain and does not expect `Entity` derived objects as a paramenter.
pub fn spawn<E>(entity: E) -> EntityHandle<E> {
	let internal_id = unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) };

	let obj = Arc::new(RwLock::new(entity));
	let handle = EntityHandle::<E>::new(obj, internal_id);

	handle
}

pub fn spawn_as_child<'a, E: Entity>(parent: EntityHandle<dyn Domain>, entity: impl SpawnHandler<E>) -> EntityHandle<E> {
	let e = entity.call(parent,).unwrap();
	e
}

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait SpawnHandler<E: Entity> {
	fn call<'a>(self, domain: EntityHandle<dyn Domain>,) -> Option<EntityHandle<E>> where Self: Sized;
}

impl <R: Entity + 'static> SpawnHandler<R> for R {
    fn call<'a>(self, domain: EntityHandle<dyn Domain>,) -> Option<EntityHandle<R>> {
		let internal_id = unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) };

		let obj = Arc::new(RwLock::new(self));

		let handle = EntityHandle::<R>::new(obj, internal_id,);

		if let Some(listener) = domain.read().get_listener() {
		}

		Some(handle)
    }
}

impl <R: Entity + 'static> SpawnHandler<R> for EntityBuilder<'_, R> {
    fn call<'a>(self, domain: EntityHandle<dyn Domain>,) -> Option<EntityHandle<R>> {
		let internal_id = unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) };

		let entity = (self.create)(domain.clone());

		let obj = std::sync::Arc::new(RwLock::new(entity));

		let handle = EntityHandle::<R>::new(obj, internal_id,);

		for f in self.post_creation_functions {
			f(domain.clone(), handle.clone(),);
		}

		for f in self.listens_to {
			f(domain.clone(), handle.clone())
		}

		if let Some(listener) = domain.read().get_listener() {
		}

		Some(handle)
    }
}

impl <R: Entity + 'static> SpawnHandler<R> for Vec<EntityBuilder<'_, R>> {
    fn call<'a>(self, domain: EntityHandle<dyn Domain>,) -> Option<EntityHandle<R>> {
		let handles = self.into_iter().map(|builder| {
			let internal_id = unsafe { COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst) };

			let entity = (builder.create)(domain.clone());

			let obj = std::sync::Arc::new(RwLock::new(entity));

			let handle = EntityHandle::<R>::new(obj, internal_id,);

			for f in builder.post_creation_functions {
				f(domain.clone(),  handle.clone(),);
			}

			for f in builder.listens_to {
				f(domain.clone(), handle.clone())
			}

			handle
		}).collect::<Vec<_>>();

		if let Some(listener) = domain.read().get_listener() {
			for handle in handles.iter() {
				
			}
		}

		Some(handles[0].clone())
    }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::core::listener::CreateEvent;
	use crate::gameplay::space::Space;
	use crate::gameplay::space::Spawner;

	#[test]
	fn test_entity_has_listeners_called_with_own_type() {
		struct EntityObject {}

		impl Entity for EntityObject {}

		struct ListenerTest {
			called: bool,
		}

		impl Entity for ListenerTest {}

		impl Listener<CreateEvent<EntityObject>> for ListenerTest {
			fn handle(&mut self, event: &CreateEvent<EntityObject>) {
				self.called = true;
			}
		}

		let space = spawn(Space::new());

		let listener = spawn_as_child(space.clone(), EntityBuilder::new(ListenerTest { called: false }).listen_to::<CreateEvent<EntityObject>>());

		assert_eq!(listener.read().called, false);

		let entity = spawn_as_child(space.clone(), EntityObject {});

		assert_eq!(listener.read().called, true);
	}
}