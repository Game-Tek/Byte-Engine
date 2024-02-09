use std::ops::{Deref, DerefMut};

use intertrait::cast::CastMut;

use super::{entity::{get_entity_trait_for_type, EntityTrait, TraitObject}, Entity, EntityHandle};

pub trait Listener: Entity {
	/// Notifies all listeners of the given type that the given entity has been created.
	fn invoke_for<T: Entity + 'static>(&self, handle: EntityHandle<T>);
	fn invoke_for_trait<T: Entity + 'static>(&self, handle: EntityHandle<T>, r#type: EntityTrait);

	/// Subscribes the given listener `L` to the given entity type `T`.
	fn add_listener<L, T: Entity + 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static;
}

pub trait EntitySubscriber<T: ?Sized> {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<T>, params: &T) -> impl std::future::Future<Output = ()>;
	fn on_update(&'static mut self, handle: EntityHandle<T>, params: &T) -> impl std::future::Future<Output = ()>;
}

pub struct BasicListener {
	listeners: std::sync::RwLock<std::collections::HashMap<EntityTrait, List>>,
}

struct List {
	l: Vec<Box<dyn Fn(EntityHandle<dyn Entity>)>>,
}

impl List {
	fn new() -> Self {
		List {
			l: Vec::new(),
		}
	}

	fn invoke_for<T: Entity>(&self, listenee: EntityHandle<T>) {
		for f in &self.l {
			f(listenee.clone());
		}
	}

	fn push(&mut self, f: Box<dyn Fn(EntityHandle<dyn Entity>)>) {
		self.l.push(f);
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
	fn invoke_for<T: Entity + std::any::Any + 'static>(&self, handle: EntityHandle<T>) {		
		let listeners = self.listeners.read().unwrap();

		if let Some(listeners) = listeners.get(&unsafe { get_entity_trait_for_type::<T>() }) {
			listeners.invoke_for(handle);
		}
	}

	fn invoke_for_trait<T: Entity + 'static>(&self, handle: EntityHandle<T>, r#type: EntityTrait) {
		let listeners = self.listeners.read().unwrap();

		if let Some(listeners) = listeners.get(&r#type) {
			listeners.invoke_for(handle);
		}
	}

	fn add_listener<L, T: Entity + 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static {
		let mut listeners = self.listeners.write().unwrap();

		let listeners = listeners.entry(unsafe { get_entity_trait_for_type::<T>() }).or_insert_with(|| List::new());

		listeners.push(Box::new(
			move |handle| {
				if let Some(cast_handle) = handle.downcast::<T>() {
					let s = cast_handle.read_sync();
					let s = s.deref();
					smol::block_on(listener.write_sync().deref_mut().on_create(cast_handle, s));
				}
			}
		));
	}
}

impl Entity for BasicListener {
	fn get_listener(&self) -> Option<&BasicListener> {
		Some(self)
	}
}