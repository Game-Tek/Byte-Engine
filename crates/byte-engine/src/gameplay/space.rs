use crate::core::{entity::EntityBuilder, spawn_as_child, SpawnHandler};

use utils::BoxedFuture;

use crate::core::{domain::Domain, entity::EntityTrait, listener::{BasicListener, EntitySubscriber, Listener}, Entity, EntityHandle};

pub struct Space {
	listener: BasicListener,
}

impl Space {
	pub fn new() -> Self {
		Space {
			listener: BasicListener::new(),
		}
	}

	// fn spawn<E: Entity>(&mut self, spawner: EntityBuilder<'static, E>) -> EntityHandle<E> {
	// 	let internal_id = 0;
		
	// 	let entity = (spawner.create)(domain.clone()).await;
		
	// 	let obj = std::sync::Arc::new(RwLock::new(entity));
		
	// 	let mut handle = EntityHandle::<R>::new(obj, internal_id,);
		
	// 	for f in self.post_creation_functions {
	// 		f(&mut handle,);
	// 	}
		
	// 	if let Some(domain) = domain.clone() {
	// 		for f in self.listens_to {
	// 			f(domain.clone(), handle.clone())
	// 		}
	// 	}
		
	// 	if let Some(domain) = domain {
	// 		if let Some(listener) = domain.write_sync().deref().get_listener() {
	// 			handle.read_sync().deref().call_listeners(listener, handle.clone()).await;
	// 		}
	// 	}
		
	// 	Some(handle)
	// }
}

pub trait Spawn {
	fn spawn<E>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E>;
}

impl Spawn for EntityHandle<Space> {
	fn spawn<E>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E> {
		spawn_as_child(self.clone(), spawner)
	}
}

impl Domain for Space {
}

impl Listener for Space {
	fn invoke_for<'a, T: ?Sized + 'static>(&'a self, handle: EntityHandle<T>, reference: &'a T) -> () {
		self.listener.invoke_for(handle, reference)
	}

	fn add_listener<T: Entity + ?Sized + 'static>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>) {
		self.listener.add_listener::<T>(listener);
	}
}

impl Entity for Space {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(&self.listener)
	}
}