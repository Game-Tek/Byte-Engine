use crate::core::domain::DomainEvents;
use crate::core::listener::DeleteEvent;
use crate::core::{spawn_as_child, SpawnHandler};

use crate::core::{domain::Domain, Entity, EntityHandle};

pub struct Space {
	events: Vec<DomainEvents>,
}

impl Space {
	pub fn new() -> Self {
		Space {
			events: Vec::with_capacity(16384),
		}
	}
}

impl Domain for Space {
	fn get_events(&mut self) -> Vec<DomainEvents> {
		self.events.drain(..).collect()
	}

	fn events_mut(&mut self) -> &mut Vec<DomainEvents> {
		&mut self.events
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
		self.write().events_mut().push(DomainEvents::EntityRemoved{ f: Box::new(move |executor| {
			executor.broadcast_event(DeleteEvent::new(handle));
		})});
	}
}

impl Entity for Space {
}