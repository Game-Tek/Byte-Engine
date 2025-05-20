use crate::core::{Entity, EntityHandle, SpawnHandler};

use super::{event::EventRegistry, task::TaskExecutor};

pub trait Domain {
	fn get_events(&mut self) -> Vec<DomainEvents>;
	fn events_mut(&mut self) -> &mut Vec<DomainEvents>;
}

pub enum DomainEvents {
	EntityCreated { f: Box<dyn FnOnce(&mut TaskExecutor)> },
	EntityRemoved { f: Box<dyn FnOnce(&mut TaskExecutor)> },
	StartListen { f: Box<dyn FnOnce(&mut TaskExecutor)> },
}