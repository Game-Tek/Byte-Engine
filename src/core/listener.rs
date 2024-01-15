use std::ops::Deref;

use intertrait::cast::CastRef;

use super::{EntityHandle, orchestrator::EntitySubscriber};

pub trait Listener {
	fn invoke_for<T: 'static>(&self, handle: EntityHandle<T>);
	fn add_listener<L, T: 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static;
}

pub struct BasicListener {
	listeners: std::sync::RwLock<std::collections::HashMap<std::any::TypeId, Box<dyn std::any::Any>>>,
}

struct List<T: ?Sized> {
	l: Vec<Box<dyn Fn(EntityHandle<T>)>>,
}

impl <T: ?Sized> List<T> {
	fn new() -> Self {
		List {
			l: Vec::new(),
		}
	}

	fn invoke_for(&self, listenee: EntityHandle<T>) {
		for f in &self.l {
			f(listenee.clone());
		}
	}

	fn push(&mut self, f: Box<dyn Fn(EntityHandle<T>)>) {
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
	fn invoke_for<T: std::any::Any + 'static>(&self, handle: EntityHandle<T>) {
		let type_id = std::any::TypeId::of::<T>();
		
		let listeners = self.listeners.read().unwrap();

		if let Some(listeners) = listeners.get(&type_id) {
			if let Some(listeners) = listeners.downcast_ref::<List<T>>() {
				listeners.invoke_for(handle);
			}
		}
	}

	fn add_listener<L, T: std::any::Any + 'static>(&self, listener: EntityHandle<L>) where L: EntitySubscriber<T> + 'static {
		let type_id = std::any::TypeId::of::<T>();
		
		let mut listeners = self.listeners.write().unwrap();

		let listeners = listeners.entry(type_id).or_insert_with(|| Box::new(List::<T>::new()));

		if let Some(listeners) = listeners.downcast_mut::<List<T>>() {
			listeners.push(Box::new(
				move |handle| {
					let handle = handle.downcast().unwrap();
					smol::block_on(listener.write_sync().on_create(handle.clone(), handle.read_sync().deref()));
				}
			));
		}
	}
}