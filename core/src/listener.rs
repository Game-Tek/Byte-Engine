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

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
	use crate::entity::EntityBuilder;
	use crate::{spawn, spawn_as_child};
	
	#[test]
	fn listeners() {
		struct Component {
			name: String,
			value: u32,
		}
		
		impl Entity for Component {}
		
		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityBuilder<'c, System> {
				EntityBuilder::new(System {}).listen_to::<Component>()
			}
		}

		static mut COUNTER: u32 = 0;

		impl EntitySubscriber<Component> for System {
			fn on_create<'a>(&'a mut self, _: EntityHandle<Component>, _: &Component) -> utils::BoxedFuture<()> {
				Box::pin(async move {
					unsafe {
						COUNTER += 1;
					}
				})
			}
		}
		
		let listener_handle = spawn(BasicListener::new());

		let _: EntityHandle<System> = spawn_as_child(listener_handle.clone(), System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(listener_handle.clone(), Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	#[test]
	fn listen_for_traits() {
		trait Boo: Entity {
			fn get_name(&self) -> String;
			fn get_value(&self) -> u32;
		}

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {
			fn get_traits(&self) -> Vec<EntityTrait> { vec![unsafe { get_entity_trait_for_type::<dyn Boo>() }] }
			fn call_listeners(&self, listener: &BasicListener, handle: EntityHandle<Self>) where Self: Sized {
				// listener.invoke_for(handle);
				listener.invoke_for(handle as EntityHandle<dyn Boo>, self);
			}
		}

		impl Boo for Component {
			fn get_name(&self) -> String { self.name.clone() }
			fn get_value(&self) -> u32 { self.value }
		}

		let _: EntityHandle<Component> = spawn(EntityBuilder::new(Component { name: "test".to_string(), value: 1 }));

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new() -> EntityBuilder<'static, System> {
				EntityBuilder::new(System {}).listen_to::<dyn Boo>()
			}
		}

		static mut COUNTER: u32 = 0;

		impl EntitySubscriber<dyn Boo> for System {
			fn on_create<'a>(&'a mut self, _: EntityHandle<dyn Boo>, _: &(dyn Boo + 'static)) -> utils::BoxedFuture<'a, ()> {
				unsafe {
					COUNTER += 1;
				}
				Box::pin(async move { })
			}
		}
		
		let listener_handle = spawn(BasicListener::new());

		let _: EntityHandle<System> = spawn_as_child(listener_handle.clone(), System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(listener_handle.clone(), EntityBuilder::new(Component { name: "test".to_string(), value: 1 }));

		assert_eq!(unsafe { COUNTER }, 1);
	}
}