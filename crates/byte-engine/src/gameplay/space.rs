use crate::core::{spawn_as_child, SpawnHandler};

use crate::core::{domain::Domain, Entity, EntityHandle};

pub struct Space {
}

impl Space {
	pub fn new() -> Self {
		Space {
		}
	}
}

impl Domain for Space {
	
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
		todo!();
	}
}

impl Entity for Space {
}