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
}

pub trait Spawn {
	type Domain: Domain + ?Sized;
	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E>;
}

impl Spawn for EntityHandle<Space> {
	type Domain = Space;
	
	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E> {
		spawn_as_child(self.clone(), spawner)
	}
}

impl Spawn for EntityHandle<dyn Domain> {
	type Domain = dyn Domain;
	
	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E> {
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