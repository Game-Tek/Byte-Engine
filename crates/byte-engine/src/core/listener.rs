use std::ops::DerefMut;

use utils::{sync::RwLock, BoxedFuture};

use crate::gameplay::space::{Destroyer, Spawner};

use super::{domain::Domain, entity::{get_entity_trait_for_type, EntityTrait}, event::Event, spawn_as_child, Entity, EntityHandle, SpawnHandler};

pub trait Listener<T: Event>: Entity {
	fn handle(&mut self, event: &T);
}

/// An event that is triggered when an entity of the type `T` is created in the domain.
/// This event is sent to all listener/subscribers of the event.
pub struct CreateEvent<T: ?Sized> {
	handle: EntityHandle<T>,
}

impl <T: ?Sized> CreateEvent<T> {
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
pub struct DeleteEvent<T: ?Sized> {
	handle: EntityHandle<T>,
}

impl <T: ?Sized> DeleteEvent<T> {
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

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityBuilder<'c, System> {
				EntityBuilder::new(System {}).listen_to::<CreateEvent<Component>>()
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

		let _: EntityHandle<System> = domain.spawn(System::new());

		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(domain.clone(), Component { name: "test".to_string(), value: 1 });

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
		}

		impl Boo for Component {
			fn get_name(&self) -> String { self.name.clone() }
			fn get_value(&self) -> u32 { self.value }
		}

		let domain: EntityHandle<dyn Domain> = spawn(Space::new());

		let _: EntityHandle<Component> = domain.spawn(EntityBuilder::new(Component { name: "test".to_string(), value: 1 }).r#as::<dyn Boo>());

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new() -> EntityBuilder<'static, System> {
				EntityBuilder::new(System {}).listen_to::<CreateEvent<dyn Boo>>()
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

		let _: EntityHandle<System> = spawn_as_child(domain.clone(), System::new());

		assert_eq!(unsafe { COUNTER }, 0);

		let _: EntityHandle<Component> = spawn_as_child(domain.clone(), EntityBuilder::new(Component { name: "test".to_string(), value: 1 }));

		assert_eq!(unsafe { COUNTER }, 1);
	}
}
