use std::ops::DerefMut;

use utils::{sync::RwLock, BoxedFuture};

use crate::gameplay::space::{Destroyer, Spawner};

use super::{domain::Domain, entity::{get_entity_trait_for_type, EntityTrait}, event::Event, spawn_as_child, Entity, EntityHandle, SpawnHandler};

pub trait Listener<T: Event + 'static>: Entity {
	fn handle(&mut self, event: &T);
}

/// An event that is triggered when an entity of the type `T` is created in the domain.
/// This event is sent to all listener/subscribers of the event.
pub struct CreateEvent<T: ?Sized + 'static> {
	handle: EntityHandle<T>,
}

impl <T: ?Sized + 'static> CreateEvent<T> {
	pub(crate) fn new(handle: EntityHandle<T>) -> Self {
		CreateEvent { handle }
	}

	pub fn handle(&self) -> &EntityHandle<T> {
		&self.handle
	}
}

impl <T: ?Sized + 'static> Event for CreateEvent<T> {
}

/// An event that is triggered when an entity of the type `T` is deleted in the domain.
/// This event is sent to all listener/subscribers of the event.
pub struct DeleteEvent<T: ?Sized + 'static> {
	handle: EntityHandle<T>,
}

impl <T: ?Sized + 'static> DeleteEvent<T> {
	pub(crate) fn new(handle: EntityHandle<T>) -> Self {
		DeleteEvent { handle }
	}

	pub fn handle(&self) -> &EntityHandle<T> {
		&self.handle
	}
}

impl <T: ?Sized + 'static> Event for DeleteEvent<T> {
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
	use crate::{core::{entity::EntityBuilder, spawn, spawn_as_child}, gameplay::space::Space};

	#[test]
	fn listeners() {
		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		struct System {

		}

		impl Entity for System {
			fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
				EntityBuilder::new(self).listen_to::<CreateEvent<Component>>()
			}
		}

		impl System {
			fn new<'c>() -> System {
				System {}
			}
		}

		static mut COUNTER: u32 = 0;

		impl Listener<CreateEvent<Component>> for System {
			fn handle(&mut self, event: &CreateEvent<Component>) -> () {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		impl Listener<DeleteEvent<Component>> for System {
			fn handle(&mut self, event: &DeleteEvent<Component>) -> () {
				unsafe {
					COUNTER -= 1;
				}
			}
		}

		let domain: EntityHandle<dyn Domain> = spawn(Space::new());

		let _: EntityHandle<System> = domain.spawn(System::new().builder());

		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(domain.clone(), Component { name: "test".to_string(), value: 1 }.builder());

		let events = domain.write().get_events();

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
			fn builder(self) -> EntityBuilder<'static, Self> {
				EntityBuilder::new(Component { name: String::new(), value: 0 }).r#as(|h| h as EntityHandle<dyn Boo>)
			}
		}

		impl Boo for Component {
			fn get_name(&self) -> String { self.name.clone() }
			fn get_value(&self) -> u32 { self.value }
		}

		let domain: EntityHandle<dyn Domain> = spawn(Space::new());

		struct System {

		}

		impl Entity for System {
			fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
				EntityBuilder::new(self).listen_to::<CreateEvent<dyn Boo>>()
			}
		}

		impl System {
			fn new() -> System {
				System {}
			}
		}

		static mut COUNTER: u32 = 0;

		impl Listener<CreateEvent<dyn Boo>> for System {
			fn handle(&mut self, event: &CreateEvent<dyn Boo>) -> () {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		impl Listener<DeleteEvent<dyn Boo>> for System {
			fn handle(&mut self, event: &DeleteEvent<dyn Boo>) -> () {
				unsafe {
					COUNTER -= 1;
				}
			}
		}

		let _: EntityHandle<System> = spawn_as_child(domain.clone(), System::new().builder());

		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(domain.clone(), Component { name: "test".to_string(), value: 1 }.builder());

		let events = domain.write().get_events();

		assert_eq!(unsafe { COUNTER }, 1);
	}
}
