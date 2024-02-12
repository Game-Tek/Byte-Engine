use std::ops::DerefMut;

use super::{entity::{get_entity_trait_for_type, EntityTrait}, Entity, EntityHandle};

pub trait Listener: Entity {
	/// Notifies all listeners of the given type that the given entity has been created.
	fn invoke_for<T: ?Sized + 'static>(&self, handle: EntityHandle<T>, reference: &T);

	/// Subscribes the given listener `L` to the given entity type `T`.
	fn add_listener<T: Entity + ?Sized>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>);
}

pub trait EntitySubscriber<T: ?Sized>: Entity {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<T>, params: &'a T) -> utils::BoxedFuture<'a, ()>;
}

pub struct BasicListener {
	listeners: std::sync::RwLock<std::collections::HashMap<EntityTrait, Box<dyn std::any::Any>>>,
}

struct List<T: ?Sized> {
	/// List of listeners for a given entity type. (EntityHandle<dyn EntitySubscriber<T>>)
	listeners: Vec<EntityHandle<dyn EntitySubscriber<T>>>,
}

impl <T: ?Sized + 'static> List<T> {
	fn new() -> Self {
		List {
			listeners: Vec::new(),
		}
	}

	fn invoke_for(&self, listenee_handle: EntityHandle<T>, listenee: &T) {
		for f in &self.listeners {
			smol::block_on(f.write_sync().deref_mut().on_create(listenee_handle.clone(), listenee));
		}
	}

	fn push(&mut self, f: EntityHandle<dyn EntitySubscriber<T>>) {
		self.listeners.push(f);
	}
}

impl BasicListener {
	pub fn new() -> Self {
		BasicListener {
			listeners: std::sync::RwLock::new(std::collections::HashMap::new()),
		}
	}
}

impl Listener for BasicListener  {
	fn invoke_for<T: ?Sized + 'static>(&self, handle: EntityHandle<T>, reference: &T) {
		let listeners = self.listeners.read().unwrap();

		if let Some(listeners) = listeners.get(&unsafe { get_entity_trait_for_type::<T>() }) {
			if let Some(listeners) = listeners.downcast_ref::<List<T>>() {
				listeners.invoke_for(handle, reference);
			}
		}
	}

	fn add_listener<T: Entity + ?Sized>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>) {
		let mut listeners = self.listeners.write().unwrap();

		let listeners = listeners.entry(unsafe { get_entity_trait_for_type::<T>() }).or_insert_with(|| Box::new(List::<T>::new()));

		if let Some(listeners) = listeners.downcast_mut::<List<T>>() {
			listeners.push(listener,);
		}
	}
}

impl Entity for BasicListener {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(self)
	}
}