//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::{collections::HashMap, any::Any, marker::FnPtr};

use super::{Entity, entity::{EntityHandle, EntityWrapper}};
use crate::utils;

pub struct Orchestrator {
}

unsafe impl Send for Orchestrator {}

pub type OrchestratorHandle = std::rc::Rc<std::cell::RefCell<Orchestrator>>;

type EntityStorage = EntityWrapper<dyn std::any::Any + 'static>;

pub(crate) struct SystemsData {
	pub(crate) systems: HashMap<u32, EntityStorage>,
	pub(crate) systems_by_name: HashMap<&'static str, u32>,
}

pub struct EventDescription<E: Entity, V> {
	phantom_e: std::marker::PhantomData<E>,
	phantom_v: std::marker::PhantomData<V>,
}

impl <E: Entity, V> EventDescription<E, V> {
	pub const fn new() -> Self {
		Self {
			phantom_e: std::marker::PhantomData,
			phantom_v: std::marker::PhantomData,
		}
	}
}

impl Orchestrator {
	pub fn new() -> Orchestrator {
		Orchestrator {
		}
	}

	pub fn new_handle() -> OrchestratorHandle {
		std::rc::Rc::new(std::cell::RefCell::new(Orchestrator::new()))
	}
}

trait Parameter where Self: Sized {
	fn call<F: FnOnce(Self)>(orchestrator: &Orchestrator, closure: F);
}

pub trait TaskFunction<'a, PS> {
	fn call(self, orchestrator: &Orchestrator);
}

impl <'a, F, P0> TaskFunction<'a, (P0,)> for F where
	P0: Parameter,
	F: Fn(P0)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { (self)(p0) });
	}
}

impl <'a, F, P0, P1> TaskFunction<'a, (P0, P1)> for F where
	P0: Parameter,
	P1: Parameter,
	F: Fn(P0, P1)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { P1::call(orchestrator, move |p1| { (self)(p0, p1) }); });
	}
}

impl <'a, F, P0, P1, P2> TaskFunction<'a, (P0, P1, P2)> for F where
	P0: Parameter,
	P1: Parameter,
	P2: Parameter,
	F: Fn(P0, P1, P2)
{
	fn call(self, orchestrator: &Orchestrator) {
		P0::call(orchestrator, move |p0| { P1::call(orchestrator, move |p1| { P2::call(orchestrator, move |p2| { (self)(p0, p1, p2) }); }); });
	}
}

#[cfg(test)]
mod tests {
	use std::ops::{DerefMut, Deref};

	use crate::core::{entity::{get_entity_trait_for_type, EntityBuilder, EntityTrait}, event::{Event, EventLike,}, listener::{BasicListener, EntitySubscriber, Listener}, property::{DerivedProperty, Property, SinkProperty}, spawn, spawn_as_child};

	use super::*;

	#[test]
	fn spawn_entities() {
		let orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let handle: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityBuilder<'c, System> {
				EntityBuilder::new(System {})
			}
		}

		impl EntitySubscriber<Component> for System {
			fn on_create<'a>(&'a mut self, handle: EntityHandle<Component>, component: &Component) -> utils::BoxedFuture<'a, ()> {
				println!("Component created: {} {}", component.name, component.value);
				Box::pin(async move { })
			}
		}
		
		let _: EntityHandle<System> = spawn(System::new());

		let component: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });
	}

	#[test]
	fn listeners() {
		let orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let handle: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

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

		let component: EntityHandle<Component> = spawn_as_child(listener_handle.clone(), Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	#[test]
	fn listen_for_traits() {
		let orchestrator = Orchestrator::new_handle();

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

		let handle: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

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

		let component: EntityHandle<Component> = spawn_as_child(listener_handle.clone(), Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	#[test]
	fn events() {
		let orchestrator_handle = Orchestrator::new_handle();

		struct MyComponent {
			name: String,
			value: u32,
			click: bool,

			event: Event<bool>,
		}

		impl MyComponent {
			pub fn set_click(&mut self, value: bool) {
				self.click = value;

				self.event.ocurred(&self.click);
			}

			pub fn click(&mut self) -> &mut Event<bool> { &mut self.event }
		}

		impl Entity for MyComponent {}

		struct MySystem {

		}

		impl Entity for MySystem {}

		static mut COUNTER: u32 = 0;

		impl MySystem {
			fn new<'c>(component_handle: &EntityHandle<MyComponent>) -> EntityBuilder<'c, MySystem> {
				EntityBuilder::new(MySystem {})
			}

			fn on_event(&mut self, value: &bool) {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		let component_handle: EntityHandle<MyComponent> = spawn(MyComponent { name: "test".to_string(), value: 1, click: false, event: Default::default() });

		let system_handle: EntityHandle<MySystem> = spawn(MySystem::new(&component_handle));

		component_handle.map(|c| {
			let mut c = c.write_sync();
			c.click().subscribe(system_handle.clone(), MySystem::on_event);
		});

		assert_eq!(unsafe { COUNTER }, 0);

		component_handle.map(|c| {
			let mut c = c.write_sync();
			c.set_click(true);
		});

		assert_eq!(unsafe { COUNTER }, 1);
	}

	#[test]
	fn reactivity() {
		let orchestrator = Orchestrator::new_handle();

		struct SourceComponent {
			value: Property<u32>,
			derived: DerivedProperty<u32, String>,
		}

		struct ReceiverComponent {
			value: SinkProperty<u32>,
			derived: SinkProperty<String>,
		}

		impl Entity for SourceComponent {}

		impl Entity for ReceiverComponent {}

		let mut value = Property::new(1);
		let derived = DerivedProperty::new(&mut value, |value| value.to_string());

		let source_component_handle: EntityHandle<SourceComponent> = spawn(SourceComponent { value, derived });
		let receiver_component_handle: EntityHandle<ReceiverComponent> = spawn(ReceiverComponent { value: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::new(&mut c.value) }), derived: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::from_derived(&mut c.derived) })});

		assert_eq!(source_component_handle.map(|c| { let c = c.read_sync(); c.value.get() }), 1);
		assert_eq!(source_component_handle.map(|c| { let c = c.read_sync(); c.derived.get() }), "1");
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read_sync(); c.value.get() }), 1);
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read_sync(); c.derived.get() }), "1");

		source_component_handle.map(|c| { let mut c = c.write_sync(); c.value.set(|_| 2) });

		assert_eq!(source_component_handle.map(|c| { let c = c.read_sync(); c.value.get() }), 2);
		assert_eq!(source_component_handle.map(|c| { let c = c.read_sync(); c.derived.get() }), "2");
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read_sync(); c.value.get() }), 2);
		assert_eq!(receiver_component_handle.map(|c| { let c = c.read_sync(); c.derived.get() }), "2");
	}
}

pub struct OrchestratorReference {
	pub(crate) handle: OrchestratorHandle,
	pub(crate) internal_id: u32,
}

impl <'a> OrchestratorReference {
	pub fn get_handle(&self) -> OrchestratorHandle {
		self.handle.clone()
	}
}