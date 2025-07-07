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
	use std::assert_matches::assert_matches;

	use super::*;
	use crate::{application::Events, core::{domain::DomainEvents, entity::EntityBuilder, spawn, spawn_as_child}, gameplay::space::Space};

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

		impl Listener<CreateEvent<Component>> for System {
			fn handle(&mut self, event: &CreateEvent<Component>) -> () {
			}
		}

		impl Listener<DeleteEvent<Component>> for System {
			fn handle(&mut self, event: &DeleteEvent<Component>) -> () {
			}
		}

		let domain: EntityHandle<dyn Domain> = spawn(Space::new());

		let _: EntityHandle<System> = domain.spawn(System::new().builder());

		let events = domain.write().get_events();

		assert_matches!(events[0], DomainEvents::StartListen { .. });

		let _: EntityHandle<Component> = domain.spawn(Component { name: "test".to_string(), value: 1 }.builder());

		let events = domain.write().get_events();

		assert_matches!(events[0], DomainEvents::EntityCreated { .. });
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

		impl Listener<CreateEvent<dyn Boo>> for System {
			fn handle(&mut self, event: &CreateEvent<dyn Boo>) -> () {
			}
		}

		impl Listener<DeleteEvent<dyn Boo>> for System {
			fn handle(&mut self, event: &DeleteEvent<dyn Boo>) -> () {
			}
		}

		let _: EntityHandle<System> = domain.spawn(System::new().builder());

		let events = domain.write().get_events();

		assert_matches!(events[0], DomainEvents::StartListen { .. });

		let _: EntityHandle<Component> = domain.spawn(Component { name: "test".to_string(), value: 1 }.builder());

		let events = domain.write().get_events();

		assert_matches!(events[0], DomainEvents::EntityCreated { .. });
	}
}
