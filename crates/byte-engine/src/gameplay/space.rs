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

impl Domain for Space {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(&self.listener)
	}

	fn get_listener_mut(&mut self) -> Option<&mut BasicListener> {
		Some(&mut self.listener)
	}
}

/// This trait allows implementers to spawn entities.
pub trait Spawner {
	type Domain: Domain + ?Sized;

	/// Spawns an entity in the domain.
	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E>;
}

/// This trait allows implementers to destroy entities.
pub trait Destroyer {
	type Domain: Domain + ?Sized;

	/// Destroys an entity in the domain.
	fn destroy<E: Entity>(&self, handle: EntityHandle<E>);
}

impl Spawner for EntityHandle<dyn Domain> {
	type Domain = dyn Domain;
	
	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E> {
		spawn_as_child(self.clone(), spawner)
	}
}

impl Destroyer for EntityHandle<dyn Domain> {
	type Domain = dyn Domain;

	fn destroy<E: Entity>(&self, handle: EntityHandle<E>) {
		if let Some(listener) = self.read().get_listener() {
			handle.read().call_listeners(listener, handle.clone());
		}
	}
}

impl Listener for Space {
	fn add_listener<T: Entity + ?Sized + 'static>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>) {
		self.listener.add_listener::<T>(listener);
	}

	fn broadcast_creation<'a, T: ?Sized + 'static>(&'a self, handle: EntityHandle<T>, reference: &'a T) -> () {
		self.listener.broadcast_creation(handle, reference)
	}

	fn broadcast_deletion<'a, T: ?Sized + 'static>(&'a self, handle: EntityHandle<T>) -> () {
		self.listener.broadcast_deletion(handle)
	}
}

impl Entity for Space {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(&self.listener)
	}
}