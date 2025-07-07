use std::fmt::Debug;

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

impl Debug for DomainEvents {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			DomainEvents::EntityCreated { .. } => write!(f, "EntityCreated"),
			DomainEvents::EntityRemoved { .. } => write!(f, "EntityRemoved"),
			DomainEvents::StartListen { .. } => write!(f, "StartListen"),
		}
	}
}
