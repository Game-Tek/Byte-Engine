use super::{property::Subscriber, EntityHandle};

/// Trait for an event-like object.
/// Allows an event object to be subscribed to and to be triggered.
pub trait Event {
}

pub struct EventRegistry {
	
}

impl EventRegistry {
	pub fn new() -> Self {
		Self {}
	}

	pub fn subscribe<E: Event, T>(&self, subscriber: EntityHandle<T>) {
		todo!("Implement EventRegistry::subscribe");
	}
	
	pub fn broadcast<T: Event>(&self, event: T) {
		todo!("Implement EventRegistry::broadcast");
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use crate::core::{entity::EntityBuilder, spawn, Entity};

	use super::*;
}
