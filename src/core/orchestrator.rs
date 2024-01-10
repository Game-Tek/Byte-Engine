//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::{collections::HashMap, any::Any, ops::DerefMut, marker::FnPtr};
use smol::lock::RwLock;

use crate::utils;

use super::{Entity, entity::{EntityHandle, EntityWrapper}};

pub trait Component: Entity {
	// type Parameters<'a>: Send + Sync;
}

struct Tie {
	update_function: std::boxed::Box<dyn Fn(&HashMap<u32, EntityStorage>, &dyn Any)>,
	destination_system_handle: u32,
}

pub trait Event<T> {
	fn fire<'f>(&self, value: &'f T);
}

#[derive(Clone)]
pub struct EventImplementation<T, V> where T: Entity {
	entity: EntityHandle<T>,
	endpoint: fn(&mut T, &V),
}

impl <T: Entity, V: Clone + 'static> EventImplementation<T, V> {
	pub fn new(entity: EntityHandle<T>, endpoint: fn(&mut T, &V)) -> Self {
		Self {
			entity,
			endpoint,
		}
	}
}

impl <'a, T: Entity, V: Clone + 'static> Event<V> for EventImplementation<T, V> {
	fn fire<'f>(&self, value: &'f V) {
		let mut lock = self.entity.container.write_arc_blocking();

		(self.endpoint)(lock.deref_mut(), value);
	}
}

#[derive(Clone)]
pub struct FreeEventImplementation<V> {
	endpoint: fn(&V),
}

impl <V: Clone + 'static> FreeEventImplementation<V> {
	pub fn new(endpoint: fn(&V)) -> Self {
		Self {
			endpoint,
		}
	}
}

impl <'a, V: Clone + 'static> Event<V> for FreeEventImplementation<V> {
	fn fire<'f>(&self, value: &'f V) {
		(self.endpoint)(value);
	}
}

#[derive(Clone)]
pub struct AsyncEventImplementation<T, V, R> where T: Entity, R: std::future::Future {
	entity: EntityHandle<T>,
	endpoint: fn(&mut T, &V) -> R,
}

impl <T: Entity, V, R: std::future::Future> AsyncEventImplementation<T, V, R> {
	pub fn new(entity: EntityHandle<T>, endpoint: fn(&mut T, &V) -> R) -> Self {
		Self {
			entity,
			endpoint,
		}
	}
}

impl <T: Entity, V, R: std::future::Future> Event<V> for AsyncEventImplementation<T, V, R> {
	fn fire<'f>(&self, value: &'f V) {
		let mut lock = self.entity.container.write_arc_blocking();

		let endpoint = &self.endpoint;

		smol::block_on(endpoint(lock.deref_mut(), value));
	}
}

pub struct Orchestrator {
	systems_data: std::sync::RwLock<SystemsData>,
	listeners_by_class: std::sync::Mutex<HashMap<&'static str, Vec<(u32, fn(&Orchestrator, OrchestratorHandle, u32, EntityHandle<dyn Entity>) -> utils::BoxedFuture<()>)>>>,
	ties: std::sync::RwLock<HashMap<usize, Vec<Tie>>>,
}

unsafe impl Send for Orchestrator {}

pub type OrchestratorHandle = std::rc::Rc<std::cell::RefCell<Orchestrator>>;

type EntityStorage = EntityWrapper<dyn Entity + 'static>;

struct SystemsData {
	counter: u32,
	systems: HashMap<u32, EntityStorage>,
	systems_by_name: HashMap<&'static str, u32>,
}

pub enum PPP<T> {
	PostCreationFunction(std::boxed::Box<dyn Fn(&mut T, OrchestratorReference,)>),
}

/// Entity creation functions must return this type.
pub struct EntityReturn<'c, T: Entity> {
	// entity: T,
	create: std::boxed::Box<dyn FnOnce(OrchestratorReference) -> T + 'c>,
	post_creation_functions: Vec<std::boxed::Box<dyn Fn(&mut EntityHandle<T>, OrchestratorReference) + 'c>>,
	// listens_to: Vec<(&'static str, Box<dyn Fn(&Orchestrator, u32, EntityHandle<dyn Entity>)>)>,
	listens_to: Vec<(&'static str, fn(&Orchestrator, OrchestratorHandle, u32, EntityHandle<dyn Entity>) -> utils::BoxedFuture<()>)>,
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

	pub fn add_listener<C: Component>(mut self,) -> Self where T: EntitySubscriber<C> {
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

pub struct EntityReference<T> where T: ?Sized {
	lock: EntityWrapper<T>,
}

impl <T: ?Sized> EntityReference<T> {
	pub fn get(&self) -> smol::lock::futures::Read<'_, T> {
		self.lock.read()
	}

	pub fn get_mut(&self) -> smol::lock::futures::Write<'_, T> {
		self.lock.write()
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

pub trait EntitySubscriber<T: Entity + Component + ?Sized> {
	async fn on_create<'a>(&'a mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
	async fn on_update(&'static mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spawn() {
		let mut orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		impl super::Component for Component {
			// type Parameters<'a> = ComponentParameters;
		}

		let handle: EntityHandle<Component> = super::spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

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
		
		let _: EntityHandle<System> = super::spawn(orchestrator.clone(), System::new());

		let component: EntityHandle<Component> = super::spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });
	}

	#[test]
	fn listeners() {
		let mut orchestrator = Orchestrator::new_handle();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		impl super::Component for Component {
			// type Parameters<'a> = ComponentParameters;
		}

		let handle: EntityHandle<Component> = super::spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

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
		
		let _: EntityHandle<System> = super::spawn(orchestrator.clone(), System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let component: EntityHandle<Component> = super::spawn(orchestrator.clone(), Component { name: "test".to_string(), value: 1 });

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

		impl Component for MyComponent {}

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

		let mut component_handle: EntityHandle<MyComponent> = super::spawn(orchestrator_handle.clone(), MyComponent { name: "test".to_string(), value: 1, click: false, events: Vec::new() });

		let system_handle: EntityHandle<MySystem> = super::spawn(orchestrator_handle.clone(), MySystem::new(&component_handle));

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
		impl Component for SourceComponent {}

		impl Entity for ReceiverComponent {}
		impl Component for ReceiverComponent {}

		let mut value = Property::new(1);
		let derived = DerivedProperty::new(&mut value, |value| value.to_string());

		let mut source_component_handle: EntityHandle<SourceComponent> = super::spawn(orchestrator.clone(), SourceComponent { value, derived });
		let receiver_component_handle: EntityHandle<ReceiverComponent> = super::spawn(orchestrator.clone(), ReceiverComponent { value: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::new(&mut c.value) }), derived: source_component_handle.map(|c| { let mut c = c.write_sync(); SinkProperty::from_derived(&mut c.derived) })});

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
	handle: OrchestratorHandle,
	internal_id: u32,
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

pub struct InternalId(pub u32);

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait IntoHandler<P, R: Entity> {
	fn call(self, orchestrator_handle: OrchestratorHandle,) -> Option<EntityHandle<R>>;
}

impl <R: Entity + 'static> IntoHandler<(), R> for R {
    fn call(self, orchestrator_handle: OrchestratorHandle,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = std::sync::Arc::new(RwLock::new(self));

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		let handle = EntityHandle::<R>::new(obj, internal_id, 0);

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut listeners = orchestrator.listeners_by_class.lock().unwrap();
			let listeners = listeners.entry(std::any::type_name::<R>()).or_insert(Vec::new());
			for (internal_id, f) in listeners {
				smol::block_on(f(&orchestrator, orchestrator_handle.clone(), *internal_id, handle.clone()));
			}
		}


		Some(handle)
    }
}

impl <R: Entity + 'static> IntoHandler<(), R> for EntityReturn<'_, R> {
    fn call(self, orchestrator_handle: OrchestratorHandle,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let entity = (self.create)(OrchestratorReference { handle: orchestrator_handle.clone(), internal_id });

		let obj = std::sync::Arc::new(RwLock::new(entity));

		{
			let orchestrator = orchestrator_handle.as_ref().borrow();
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		let mut handle = EntityHandle::<R>::new(obj, internal_id, 0);

		{
			

			for f in self.post_creation_functions {
				f(&mut handle, OrchestratorReference { handle: orchestrator_handle.clone(), internal_id });
			}
		}

		{
			for (type_id, f) in self.listens_to {
				let orchestrator = orchestrator_handle.as_ref().borrow();

				let mut listeners = orchestrator.listeners_by_class.lock().unwrap();
			
				let listeners = listeners.entry(type_id).or_insert(Vec::new());

				listeners.push((internal_id, f));
			}
		}

		Some(handle)
    }
}

struct PropertyState<T> {
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

pub trait PropertyLike<T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>);
	fn get_value(&self) -> T;
}

/// A property is a piece of data that can be read and written, that signals when it is written to.
pub struct Property<T> {
	value: T,
	internal_state: std::rc::Rc<std::sync::RwLock<PropertyState<T>>>,
}

impl <T: Clone + 'static> Property<T> {
	/// Creates a new property with the given value.
	pub fn new(value: T) -> Self {
		Self {
			internal_state: std::rc::Rc::new(std::sync::RwLock::new(PropertyState { subscribers: Vec::new() })),
			value,
		}
	}

	pub fn link_to<S: Entity>(&mut self, handle: EntityHandle<S>, destination_property: fn() -> EventDescription<S, T>) {
		let mut internal_state = self.internal_state.write().unwrap();
		
		// internal_state.receivers.push(Box::new(SinkPropertyReceiver { handle, property: destination_property }));
	}

	pub fn get(&self) -> T where T: Copy {
		self.value
	}

	/// Sets the value of the property.
	pub fn set(&mut self, setter: impl FnOnce(&T) -> T) {
		self.value = setter(&self.value);

		let mut internal_state = self.internal_state.write().unwrap();

		for subscriber in &mut internal_state.subscribers {
			let mut subscriber = subscriber.write().unwrap();
			subscriber.update(&self.value);
		}
	}
}

pub struct DerivedProperty<F, T> {
	internal_state: std::rc::Rc<std::sync::RwLock<DerivedPropertyState<F, T>>>,
}

struct DerivedPropertyState<F, T> {
	deriver: fn(&F) -> T,
	value: T,
	subscribers: Vec<std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>>,
}

impl <F, T> Subscriber<F> for DerivedPropertyState<F, T> {
	fn update(&mut self, value: &F) {
		self.value = (self.deriver)(value);

		for receiver in &mut self.subscribers {
			let mut receiver = receiver.write().unwrap();
			receiver.update(&self.value);
		}
	}
}

impl <F: Clone + 'static, T: Clone + 'static> DerivedProperty<F, T> {
	pub fn new(source_property: &mut Property<F>, deriver: fn(&F) -> T) -> Self {
		let h = std::rc::Rc::new(std::sync::RwLock::new(DerivedPropertyState { subscribers: Vec::new(), value: deriver(&source_property.value), deriver }));

		source_property.add_subscriber(h.clone());

		Self {
			internal_state: h,
		}
	}

	pub fn link_to<S: Entity>(&mut self, subscriber: &SinkProperty<T>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber.internal_state.clone());
	}

	pub fn get(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

pub struct SinkProperty<T> {
	internal_state: std::rc::Rc<std::sync::RwLock<SinkPropertyState<T>>>,
}

pub struct SinkPropertyState<T> {
	value: T,
}

impl <T: Clone + 'static> Subscriber<T> for SinkPropertyState<T> {
	fn update(&mut self, value: &T) {
		self.value = value.clone();
	}
}

impl <T: Clone + 'static> SinkProperty<T> {
	pub fn new(source_property: &mut impl PropertyLike<T>) -> Self {
		let internal_state = std::rc::Rc::new(std::sync::RwLock::new(SinkPropertyState { value: source_property.get_value() }));

		source_property.add_subscriber(internal_state.clone());

		Self {
			internal_state: internal_state.clone(),
		}
	}

	pub fn from_derived<F: Clone + 'static>(source_property: &mut DerivedProperty<F, T>) -> Self {
		let internal_state = std::rc::Rc::new(std::sync::RwLock::new(SinkPropertyState { value: source_property.get() }));

		let mut source_property_internal_state = source_property.internal_state.write().unwrap();
		source_property_internal_state.subscribers.push(internal_state.clone());

		Self {
			internal_state: internal_state.clone(),
		}
	}

	pub fn get(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

trait Subscriber<T> {
	fn update(&mut self, value: &T);
}

impl <T: Clone + 'static> PropertyLike<T> for Property<T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get_value(&self) -> T {
		self.value.clone()
	}
}

impl <F: Clone + 'static, T: Clone + 'static> PropertyLike<T> for DerivedProperty<F, T> {
	fn add_subscriber(&mut self, subscriber: std::rc::Rc<std::sync::RwLock<dyn Subscriber<T>>>) {
		let mut internal_state = self.internal_state.write().unwrap();
		internal_state.subscribers.push(subscriber);
	}

	fn get_value(&self) -> T {
		let internal_state = self.internal_state.read().unwrap();
		internal_state.value.clone()
	}
}

pub fn spawn<E: Entity>(orchestrator_handle: OrchestratorHandle, entity: impl IntoHandler<(), E>) -> EntityHandle<E> {
	entity.call(orchestrator_handle,).unwrap()
}