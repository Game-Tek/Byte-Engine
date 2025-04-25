use crate::core::{Entity, EntityHandle, SpawnHandler};

pub trait Domain: Entity {
	// fn spawn<E: Entity>(&mut self, spawner: impl SpawnHandler<E>) -> EntityHandle<E>;
}
