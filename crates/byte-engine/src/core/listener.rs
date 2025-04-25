use std::ops::DerefMut;

use utils::{sync::RwLock, BoxedFuture};

use crate::gameplay::space::Spawn;

use super::{domain::Domain, entity::{get_entity_trait_for_type, EntityTrait}, spawn_as_child, Entity, EntityHandle, SpawnHandler};

pub trait Listener: Entity {
	/// Notifies all listeners of the given type that the given entity has been created.
	fn invoke_for<'a, T: ?Sized + 'static>(&'a self, handle: EntityHandle<T>, reference: &'a T) -> ();

	/// Subscribes the given listener `L` to the given entity type `T`.
	fn add_listener<T: Entity + ?Sized>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>);
}

pub trait EntitySubscriber<T: ?Sized> {
	fn on_create<'a>(&'a mut self, handle: EntityHandle<T>, params: &'a T) -> ();
}

pub struct BasicListener {
	listeners: RwLock<std::collections::HashMap<EntityTrait, Box<dyn std::any::Any>>>,
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
		self.listeners.iter().for_each(move |l| {
			l.write().deref_mut().on_create(listenee_handle.clone(), listenee);
		});
	}

	fn push(&mut self, f: EntityHandle<dyn EntitySubscriber<T>>) {
		self.listeners.push(f);
	}
}

impl BasicListener {
	pub fn new() -> Self {
		BasicListener {
			listeners: RwLock::new(std::collections::HashMap::new()),
		}
	}
}

impl Listener for BasicListener  {
	fn invoke_for<'a, T: ?Sized + 'static>(&'a self, handle: EntityHandle<T>, reference: &'a T) -> () {
		let listeners = self.listeners.read();

		if let Some(listeners) = listeners.get(&unsafe { get_entity_trait_for_type::<T>() }) {
			if let Some(listeners) = listeners.downcast_ref::<List<T>>() {
				listeners.invoke_for(handle, reference);
			}
		}
	}

	fn add_listener<T: Entity + ?Sized>(&self, listener: EntityHandle<dyn EntitySubscriber<T>>) {
		let mut listeners = self.listeners.write();

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

impl Domain for BasicListener {}

impl Spawn for EntityHandle<BasicListener> {
	type Domain = BasicListener;

	fn spawn<E: Entity>(&self, spawner: impl SpawnHandler<E>) -> EntityHandle<E> {
		spawn_as_child(self.clone(), spawner)
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
	use crate::core::{spawn, spawn_as_child, entity::EntityBuilder};

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
			fn on_create<'a>(&'a mut self, _: EntityHandle<Component>, _: &Component) -> () {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		let listener_handle = spawn(BasicListener::new());

		let _: EntityHandle<System> = listener_handle.spawn(System::new());

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
			fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> () where Self: Sized {
				// listener.invoke_for(handle);
				listener.invoke_for(handle as EntityHandle<dyn Boo>, self);
			}
		}

		impl Boo for Component {
			fn get_name(&self) -> String { self.name.clone() }
			fn get_value(&self) -> u32 { self.value }
		}

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

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
			fn on_create<'a>(&'a mut self, _: EntityHandle<dyn Boo>, _: &(dyn Boo + 'static)) -> () {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		let listener_handle = spawn(BasicListener::new());

		let _: EntityHandle<System> = spawn_as_child(listener_handle.clone(), System::new());

		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(listener_handle.clone(), EntityBuilder::new(Component { name: "test".to_string(), value: 1 }));

		assert_eq!(unsafe { COUNTER }, 1);
	}
}
