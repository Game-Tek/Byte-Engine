//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::{collections::HashMap, any::Any, marker::FnPtr};

use super::{Entity, entity::{EntityHandle, EntityWrapper}};
use crate::utils;

pub(crate) struct Tie {
	update_function: std::boxed::Box<dyn Fn(&HashMap<u32, EntityStorage>, &dyn Any)>,
	destination_system_handle: u32,
}

pub struct Orchestrator {
	pub(crate) systems_data: std::sync::RwLock<SystemsData>,
	pub(crate) listeners_by_class: std::sync::Mutex<HashMap<&'static str, Vec<(u32, fn(&Orchestrator, OrchestratorHandle, u32, EntityHandle<dyn Entity>) -> utils::BoxedFuture<()>)>>>,
	pub(crate) ties: std::sync::RwLock<HashMap<usize, Vec<Tie>>>,
}

unsafe impl Send for Orchestrator {}

pub type OrchestratorHandle = std::rc::Rc<std::cell::RefCell<Orchestrator>>;

type EntityStorage = EntityWrapper<dyn Entity + 'static>;

pub(crate) struct SystemsData {
	pub(crate) counter: u32,
	pub(crate) systems: HashMap<u32, EntityStorage>,
	pub(crate) systems_by_name: HashMap<&'static str, u32>,
}

pub enum PPP<T> {
	PostCreationFunction(std::boxed::Box<dyn Fn(&mut T, OrchestratorReference,)>),
}

/// Entity creation functions must return this type.
pub struct EntityReturn<'c, T: Entity> {
	// entity: T,
	pub(crate) create: std::boxed::Box<dyn FnOnce(OrchestratorReference) -> T + 'c>,
	pub(crate) post_creation_functions: Vec<std::boxed::Box<dyn Fn(&mut EntityHandle<T>, OrchestratorReference) + 'c>>,
	// listens_to: Vec<(&'static str, Box<dyn Fn(&Orchestrator, u32, EntityHandle<dyn Entity>)>)>,
	pub(crate) listens_to: Vec<(&'static str, fn(&Orchestrator, OrchestratorHandle, u32, EntityHandle<dyn Entity>) -> utils::BoxedFuture<()>)>,
}

impl <'c, T: Entity + 'static> EntityReturn<'c, T> {
	pub fn new(entity: T) -> Self {
		Self {
			create: std::boxed::Box::new(move |_| entity),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn new_from_function(function: impl FnOnce(OrchestratorReference) -> T + 'c) -> Self {
		Self {
			create: std::boxed::Box::new(function),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn new_from_closure<'a, F: FnOnce(OrchestratorReference) -> T + 'c>(function: F) -> Self {
		Self {
			create: std::boxed::Box::new(function),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn add_post_creation_function(mut self, function: impl Fn(&mut EntityHandle<T>, OrchestratorReference) + 'c) -> Self {
		self.post_creation_functions.push(Box::new(function));
		self
	}

	pub fn add_listener<C: Entity>(mut self,) -> Self where T: EntitySubscriber<C> {
		// TODO: Notify listener of the entities that existed before they started to listen.
		// Maybe add a parameter to choose whether to listen retroactively or not. With a default value of true.

		let b: fn(&Orchestrator, OrchestratorHandle, u32, EntityHandle<dyn Entity>) -> utils::BoxedFuture<()> = |orchestrator: &Orchestrator, orchestrator_handle: OrchestratorHandle, system_to_notify: u32, component_handle: EntityHandle<dyn Entity>| {
			Box::pin(async move {
				let systems_data = orchestrator.systems_data.read().unwrap();

				let mut lock_guard = systems_data.systems[&system_to_notify].write_arc().await;
				let system: &mut T = lock_guard.downcast_mut().unwrap();

				let orchestrator_reference = OrchestratorReference { handle: orchestrator_handle, internal_id: system_to_notify };

				let component = systems_data.systems[&component_handle.internal_id].read().await;
				let component: &C = component.downcast_ref().unwrap();

				if let Some(x) = component_handle.downcast() {
					system.on_create(orchestrator_reference, x, component).await;
				} else {
					panic!("Failed to downcast component");
				}
			})
		};

		self.listens_to.push((std::any::type_name::<C>(), b));

		self
	}
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
			systems_data: std::sync::RwLock::new(SystemsData { counter: 0, systems: HashMap::new(), systems_by_name: HashMap::new(), }),
			listeners_by_class: std::sync::Mutex::new(HashMap::new()),
			ties: std::sync::RwLock::new(HashMap::new()),
		}
	}

	pub fn new_handle() -> OrchestratorHandle {
		std::rc::Rc::new(std::cell::RefCell::new(Orchestrator::new()))
	}

	// Ties a property of a component to a property of another component.
	fn tie_internal<S: Entity + 'static, R: Entity, SV: Any + 'static, RV: From<SV> + Any + Clone + 'static,>(&self, receiver_internal_id: u32, i: fn() -> EventDescription<S, RV>, _sender_component_handle: &EntityHandle<R>, j: fn() -> EventDescription<R, SV>) {
		let property_function_pointer = j as *const (); // Use the property function pointer as a key to the ties hashmap.

		let property = i();

		let mut ties = self.ties.write().unwrap();

		let update_function = Box::new(move |systems: &HashMap<u32, EntityStorage>, value: &dyn Any| {
			// (property.setter)(systems[&receiver_internal_id].write().unwrap().downcast_mut::<S>().unwrap(), value.downcast_ref::<RV>().unwrap().clone())
		});

		if let std::collections::hash_map::Entry::Vacant(e) = ties.entry(property_function_pointer as usize) {
			let mut ties_new = Vec::new();
			ties_new.push(Tie { update_function, destination_system_handle: receiver_internal_id });
			e.insert(ties_new);
		} else {
			let ties = ties.get_mut(&(property_function_pointer as usize)).unwrap();

			if !ties.iter().any(|tie| tie.destination_system_handle == receiver_internal_id) {
				ties.push(Tie { update_function, destination_system_handle: receiver_internal_id });
			}
		}
	}

	pub fn set_property<C: Entity + 'static, V: Clone + Copy + 'static>(&self, component_handle: &EntityHandle<C>, function: fn() -> EventDescription<C, V>, value: V) {
		{
			let ties = self.ties.read().unwrap();

			if let Some(ties) = ties.get(&(function.addr() as usize)) {
				let systems_data = self.systems_data.read().unwrap();

				for tie in ties {
					(tie.update_function)(&systems_data.systems, &value);
				}
			}
		}
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

pub trait EntitySubscriber<T: Entity + ?Sized> {
	async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
	async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
}

#[cfg(test)]
mod tests {
	use crate::core::{spawn, property::{Property, DerivedProperty, SinkProperty}, event::{Event, EventImplementation}};

	use super::*;

	#[test]
	fn spawn_entities() {
		let mut orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let handle: EntityHandle<Component> = spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityReturn<'c, System> {
				EntityReturn::new(System {})
			}
		}

		impl EntitySubscriber<Component> for System {
			async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, component: &Component) {
			}

			async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, params: &Component) {}
		}
		
		let _: EntityHandle<System> = spawn(orchestrator.clone(), System::new());

		let component: EntityHandle<Component> = spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });
	}

	#[test]
	fn listeners() {
		let mut orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let handle: EntityHandle<Component> = spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityReturn<'c, System> {
				EntityReturn::new(System {}).add_listener::<Component>()
			}
		}

		static mut COUNTER: u32 = 0;

		impl EntitySubscriber<Component> for System {
			async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, component: &Component) {
				unsafe {
					COUNTER += 1;
				}
			}

			async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, params: &Component) {}
		}
		
		let _: EntityHandle<System> = spawn(orchestrator.clone(), System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let component: EntityHandle<Component> = spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	#[test]
	fn events() {
		let orchestrator_handle = Orchestrator::new_handle();

		struct MyComponent {
			name: String,
			value: u32,
			click: bool,

			events: Vec<Box<dyn Event<bool>>>,
		}

		impl MyComponent {
			pub fn set_click(&mut self, value: bool) {
				self.click = value;

				for event in &self.events {
					event.fire(&self.click);
				}
			}

			pub const fn click() -> EventDescription<MyComponent, bool> { EventDescription::new() }
			pub fn subscribe<E: Entity>(&mut self, subscriber: EntityHandle<E>,  endpoint: fn(&mut E, &bool)) {
				self.events.push(Box::new(EventImplementation::new(subscriber, endpoint)));
			}
		}

		impl Entity for MyComponent {}

		struct MySystem {

		}

		impl Entity for MySystem {}

		static mut COUNTER: u32 = 0;

		impl MySystem {
			fn new<'c>(component_handle: &EntityHandle<MyComponent>) -> EntityReturn<'c, MySystem> {
				EntityReturn::new(MySystem {})
			}

			fn on_event(&mut self, value: &bool) {
				unsafe {
					COUNTER += 1;
				}
			}
		}

		let mut component_handle: EntityHandle<MyComponent> = spawn(orchestrator_handle.clone(), MyComponent { name: "test".to_string(), value: 1, click: false, events: Vec::new() });

		let system_handle: EntityHandle<MySystem> = spawn(orchestrator_handle.clone(), MySystem::new(&component_handle));

		component_handle.map(|c| {
			let mut c = c.write_sync();
			c.subscribe(system_handle.clone(), MySystem::on_event);
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
		let mut orchestrator = Orchestrator::new_handle();

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

		let mut source_component_handle: EntityHandle<SourceComponent> = spawn(orchestrator.clone(), SourceComponent { value, derived });
		let receiver_component_handle: EntityHandle<ReceiverComponent> = spawn(orchestrator.clone(), ReceiverComponent { value: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::new(&mut c.value) }), derived: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::from_derived(&mut c.derived) })});

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
	// pub fn tie<'b, T: Entity + 'static, U: Entity + 'b, V: Any + Copy + 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> EventDescription<T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> EventDescription<U, V>) {
	// 	let orchestrator = self.handle.as_ref().borrow();
	// 	orchestrator.tie(receiver_component_handle, i, sender_component_handle, j);
	// }

	pub fn tie_self<T: Entity + 'static, U: Entity, V: Any + Copy + 'static>(&self, consuming_property: fn() -> EventDescription<T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> EventDescription<U, V>) {
		let orchestrator = self.handle.as_ref().borrow();
		orchestrator.tie_internal(self.internal_id, consuming_property, sender_component_handle, j);
	}

	pub fn get_handle(&self) -> OrchestratorHandle {
		self.handle.clone()
	}
}