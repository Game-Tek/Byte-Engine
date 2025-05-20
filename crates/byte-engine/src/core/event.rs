use downcast_rs::{impl_downcast, Downcast};
use utils::{hash::{HashMap, HashMapExt}, sync::RwLock};

use super::{listener::Listener, Entity, EntityHandle};

/// Trait for an event-like object.
/// Allows an event object to be subscribed to and to be triggered.
pub trait Event: Downcast {
}

impl_downcast!(Event);

pub struct EventRegistry {
	map: RwLock<HashMap<std::any::TypeId, Vec<Box<dyn Fn(&dyn std::any::Any)>>>>,
}

impl EventRegistry {
	pub fn new() -> Self {
		Self { map: RwLock::new(HashMap::with_capacity(1024)) }
	}

	pub fn subscribe<E: Event + 'static, T: Listener<E> + 'static>(&self, subscriber: EntityHandle<T>) {
		let type_id = std::any::TypeId::of::<E>();
		let mut map = self.map.write();
		let subscribers = map.entry(type_id).or_insert_with(|| Vec::with_capacity(8));
		subscribers.push(Box::new(move |event| {
			let event = unsafe {
				event.downcast_ref_unchecked::<E>()
			};

			subscriber.write().handle(event);
		}));
	}
	
	pub fn broadcast<T: Event + 'static>(&self, event: T) {
		let type_id = std::any::TypeId::of::<T>();
		let map = self.map.read();
		if let Some(event_entry) = map.get(&type_id) {
			for subscriber in event_entry {
				subscriber(&event);
			}
		}
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use crate::core::{entity::EntityBuilder, spawn, Entity};

	use super::*;
}
