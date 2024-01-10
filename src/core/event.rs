use std::ops::DerefMut;

use super::{Entity, EntityHandle};

pub trait Event<T> {
	fn fire<'f>(&self, value: &'f T);
}

#[derive(Clone)]
pub struct EventImplementation<T, V> where T: Entity {
	entity: EntityHandle<T>,
	endpoint: fn(&mut T, &V),
}

impl <T: Entity, V: Clone + 'static> EventImplementation<T, V> {
	pub fn new(entity: EntityHandle<T>, endpoint: fn(&mut T, &V)) -> Self {
		Self {
			entity,
			endpoint,
		}
	}
}

impl <'a, T: Entity, V: Clone + 'static> Event<V> for EventImplementation<T, V> {
	fn fire<'f>(&self, value: &'f V) {
		let mut lock = self.entity.container.write_arc_blocking();

		(self.endpoint)(lock.deref_mut(), value);
	}
}

#[derive(Clone)]
pub struct FreeEventImplementation<V> {
	endpoint: fn(&V),
}

impl <V: Clone + 'static> FreeEventImplementation<V> {
	pub fn new(endpoint: fn(&V)) -> Self {
		Self {
			endpoint,
		}
	}
}

impl <'a, V: Clone + 'static> Event<V> for FreeEventImplementation<V> {
	fn fire<'f>(&self, value: &'f V) {
		(self.endpoint)(value);
	}
}

#[derive(Clone)]
pub struct AsyncEventImplementation<T, V, R> where T: Entity, R: std::future::Future {
	entity: EntityHandle<T>,
	endpoint: fn(&mut T, &V) -> R,
}

impl <T: Entity, V, R: std::future::Future> AsyncEventImplementation<T, V, R> {
	pub fn new(entity: EntityHandle<T>, endpoint: fn(&mut T, &V) -> R) -> Self {
		Self {
			entity,
			endpoint,
		}
	}
}

impl <T: Entity, V, R: std::future::Future> Event<V> for AsyncEventImplementation<T, V, R> {
	fn fire<'f>(&self, value: &'f V) {
		let mut lock = self.entity.container.write_arc_blocking();

		let endpoint = &self.endpoint;

		smol::block_on(endpoint(lock.deref_mut(), value));
	}
}