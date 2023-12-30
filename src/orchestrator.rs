//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::borrow::Borrow;
use std::{collections::HashMap, any::Any};
use downcast_rs::{impl_downcast, Downcast};
use intertrait::*;
use log::{trace, warn};

pub trait Entity: CastFrom + Downcast + Any + 'static {}

impl_downcast!(Entity);

/// A system is a collection of components and logic to operate on them.
pub trait System: Entity + Any {}

#[cfg(feature="multithreading")]
type EntityWrapper<T: Entity> = std::sync::Arc<std::sync::RwLock<T>>;

#[cfg(not(feature="multithreading"))]
type EntityWrapper<T: Entity> = std::rc::Rc<std::cell::RefCell<T>>;

#[derive(Debug,)]
pub struct EntityHandle<T: Entity + ?Sized> {
	container: EntityWrapper<T>,
	internal_id: u32,
	external_id: u32,
}

pub type EntityHash = u32;

impl <T: Entity + ?Sized> From<&EntityHandle<T>> for EntityHash {
	fn from(handle: &EntityHandle<T>) -> Self {
		handle.internal_id
	}
}

impl <T: Entity> EntityHandle<T> {
	pub fn new(object: EntityWrapper<T>, internal_id: u32, external_id: u32) -> Self {
		Self {
			container: object,
			internal_id: internal_id,
			external_id: external_id,
		}
	}

	pub fn get_external_key(&self) -> u32 {
		self.external_id
	}
}

fn downcast_inner<U: Entity>(decoder: &EntityWrapper<dyn Entity>) -> Option<EntityWrapper<U>> {
	let raw: *const std::cell::RefCell<dyn Entity> = std::rc::Rc::into_raw(decoder.clone());
	let raw: *const std::cell::RefCell<U> = raw.cast();
	
	// SAFETY: This is safe because the pointer orignally came from an Arc
	// with the same size and alignment since we've checked (via Any) that
	// the object within is the type being casted to.
	Some(unsafe { std::rc::Rc::from_raw(raw) })
}

impl EntityHandle<dyn Entity> {
	fn downcast<U: Entity>(&self) -> Option<EntityHandle<U>> {
		let down = downcast_inner::<U>(&self.container);
		Some(EntityHandle {
			container: down?,
			internal_id: self.internal_id,
			external_id: self.external_id,
		})
	}
}

impl <T: Entity + ?Sized> Clone for EntityHandle<T> {
	fn clone(&self) -> Self {
		Self {
			container: self.container.clone(),
			internal_id: self.internal_id,
			external_id: self.external_id,
		}
	}
}

use std::marker::{Unsize, FnPtr};
use std::ops::{CoerceUnsized, DerefMut, Deref};
impl<T: Entity, U: Entity> CoerceUnsized<EntityHandle<U>> for EntityHandle<T>
where
    T: Unsize<U> + ?Sized,
    U: ?Sized {}

impl <T: Entity> EntityHandle<T> {
	pub fn spawn<F, P>(orchestrator: &Orchestrator, function: F) -> Option<EntityHandle<T>> where T: Entity + 'static, F: IntoHandler<P, T> {
		let handle = function.call(orchestrator)?;

		trace!("{}", std::any::type_name::<T>());

		{
			let systems_data = orchestrator.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(std::any::type_name::<T>()) {
				for listener in listeners {
					(listener.1)(orchestrator, listener.0, handle.clone());
				}
			}
		}

		Some(handle)
	}
}

impl <T: Entity + ?Sized> EntityHandle<T> {
	pub fn get<R>(&self, function: impl FnOnce(&T) -> R) -> R {
		#[cfg(feature="multithreading")]
		let lock = &self.container.read().unwrap();

		#[cfg(not(feature="multithreading"))]
		let lock = &self.container.as_ref().borrow();

		function(lock.deref())
	}

	pub fn get_mut<R>(&mut self, function: impl FnOnce(&mut T) -> R) -> R {
		#[cfg(feature="multithreading")]
		let mut lock = self.container.write().unwrap();

		#[cfg(not(feature="multithreading"))]
		let mut lock = self.container.as_ref().borrow_mut();

		function(lock.deref_mut())
	}
}

/// A component is a piece of data that is attached to an entity.
pub trait Component : Entity {
	// type Parameters<'a>: Send + Sync;
}

struct Tie {
	update_function: std::boxed::Box<dyn Fn(&HashMap<u32, EntityStorage>, &dyn Any)>,
	destination_system_handle: u32,
}

pub trait Event<T> {
	fn fire(&self, value: &T);
}

pub struct EventImplementation<T, V> where T: Entity {
	entity: EntityHandle<T>,
	function: fn(&mut T, &V),
}

impl <T: Entity, V: Clone + Copy + 'static> EventImplementation<T, V> {
	pub fn new(entity: EntityHandle<T>, function: fn(&mut T, &V)) -> Self {
		Self {
			entity,
			function,
		}
	}
}

impl <T: Entity, V: Clone + Copy + 'static> Event<V> for EventImplementation<T, V> {
	fn fire(&self, value: &V) {
		#[cfg(feature="multithreading")]
		let mut lock = self.entity.container.write().unwrap();

		#[cfg(not(feature="multithreading"))]
		let mut lock = self.entity.container.as_ref().borrow_mut();

		(self.function)(lock.deref_mut(), value);
	}
}

/// An orchestrator is a collection of systems that are updated in parallel.
pub struct Orchestrator {
	systems_data: std::sync::RwLock<SystemsData>,
	listeners_by_class: std::sync::Mutex<HashMap<&'static str, Vec<(u32, Box<dyn Fn(&Orchestrator, u32, EntityHandle<dyn Entity>)>)>>>,
	ties: std::sync::RwLock<HashMap<usize, Vec<Tie>>>,
}

unsafe impl Send for Orchestrator {}

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
	post_creation_functions: Vec<std::boxed::Box<dyn Fn(&EntityHandle<T>, OrchestratorReference) + 'c>>,
	// listens_to: Vec<(&'static str, Box<dyn Fn(&Orchestrator, u32, EntityHandle<dyn Entity>)>)>,
	listens_to: Vec<(&'static str, Box<dyn Fn(&Orchestrator, u32, EntityHandle<dyn Entity>)>)>,
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

	pub fn add_post_creation_function(mut self, function: impl Fn(&EntityHandle<T>, OrchestratorReference) + 'c) -> Self {
		self.post_creation_functions.push(Box::new(function));
		self
	}

	pub fn add_listener<C: Component>(mut self,) -> Self where T: EntitySubscriber<C> {
		// TODO: Notify listener of the entities that existed before they started to listen.
		// Maybe add a parameter to choose whether to listen retroactively or not. With a default value of true.

		let b = Box::new(move |orchestrator: &Orchestrator, system_to_notify: u32, component_handle: EntityHandle<dyn Entity>| {
			let systems_data = orchestrator.systems_data.read().unwrap();

			#[cfg(feature="multithreading")]
			let mut lock_guard = systems_data.systems[&system_to_notify].write().unwrap();
			#[cfg(feature="multithreading")]
			let system: &mut T = lock_guard.downcast_mut().unwrap();

			#[cfg(not(feature="multithreading"))]
			let mut lock = systems_data.systems[&system_to_notify].as_ref().borrow_mut();
			let system: &mut T = lock.downcast_mut().unwrap();

			let orchestrator_reference = OrchestratorReference { orchestrator, internal_id: system_to_notify };

			#[cfg(feature="multithreading")]
			let component = systems_data.systems[&component_handle.internal_id].read().unwrap();
			#[cfg(feature="multithreading")]
			let component: &C = component.downcast_ref().unwrap();

			#[cfg(not(feature="multithreading"))]
			let lock = systems_data.systems[&component_handle.internal_id].as_ref().borrow();
			let component: &C = lock.downcast_ref().unwrap();

			if let Some(x) = component_handle.downcast() {
				system.on_create(orchestrator_reference, x, component);
			} else {
				panic!("Failed to downcast component");
			}
		});

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
			// parameters: std::sync::RwLock::new(HashMap::new()),
		}
	}

	pub fn initialize(&self) {}
	pub fn deinitialize(&self) {}

	/// Spawn entity is a function that spawns an entity and returns a handle to it.
	pub fn spawn_entity<'c, T, P, F: 'c>(&self, function: F) -> Option<EntityHandle<T>> where T: Entity + 'static, F: IntoHandler<P, T> {
		let handle = function.call(self)?;

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(std::any::type_name::<T>()) {
				for listener in listeners {
					(listener.1)(self, listener.0, handle.clone());
				}
			}
		}

		Some(handle)
	}

	/// Spawns a component and returns a handle to it.
	pub fn spawn<C: Component>(&self, component: C) -> EntityHandle<C> {
		let internal_id = {
			let mut systems_data = self.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		// let obj = std::sync::Arc::new(std::sync::RwLock::new(C::new(OrchestratorReference { orchestrator: self, internal_id }, parameters)));

		let object = {
			#[cfg(feature="multithreading")]
			let object = std::sync::Arc::new(std::sync::RwLock::new(component));

			#[cfg(not(feature="multithreading"))]
			let object = std::rc::Rc::new(std::cell::RefCell::new(component));

			let mut systems_data = self.systems_data.write().unwrap();

			systems_data.systems.insert(internal_id, object.clone());

			systems_data.systems_by_name.insert(std::any::type_name::<C>(), internal_id);

			// self.parameters.write().unwrap().insert(internal_id, std::sync::Arc::new(component));

			object
		};

		let external_id = 0;

		let handle = EntityHandle::new(object, internal_id, external_id);

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(std::any::type_name::<C>()) {
				for listener in listeners {
					(listener.1)(self, listener.0, handle.clone());
				}
			} else {
				warn!("No listeners for {}", std::any::type_name::<C>());
			}
		}

		handle
	}

	/// Ties a property of a component to a property of another component.
	pub fn tie<R: Entity + 'static, S: Entity, SV: Any + 'static, RV: From<SV> + Any + Clone + 'static>(&self, receiver_component_handle: &EntityHandle<R>, i: fn() -> EventDescription<R, RV>, _sender_component_handle: &EntityHandle<S>, j: fn() -> EventDescription<S, SV>) {
		// self.tie_internal(receiver_component_handle.internal_id, i, _sender_component_handle, j)
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

	// pub fn get_property<C: Entity + 'static, V: Clone + Copy + 'static>(&self, component_handle: &EntityHandle<C>, function: fn() -> EventDescription<C, V>) -> V {
	// 	let systems_data = self.systems_data.read().unwrap();

	// 	let property = function();

	// 	let component = systems_data.systems[&component_handle.internal_id].read().unwrap();
	// 	let getter = property.getter as *const ();
	// 	let getter = unsafe { std::mem::transmute::<*const (), fn(&C) -> V>(getter) };
	// 	(getter)(component.downcast_ref::<C>().unwrap())
	// }

	pub fn invoke_mut<E: Entity + 'static>(&self, handle: &mut EntityHandle<E>, function: fn(&mut E, OrchestratorReference)) {
		let external_id = handle.external_id;

		handle.get_mut(|c| {
			function(c, OrchestratorReference { orchestrator: self, internal_id: external_id });
		})
	}

	pub fn get_entity<S: System + ?Sized + 'static>(&self, entity_handle: &EntityHandle<S>) -> EntityReference<S> {
		let systems_data = self.systems_data.read().unwrap();
		EntityReference { lock: std::rc::Rc::clone(&entity_handle.container) }
	}
}

pub struct EntityReference<T> where T: ?Sized {
	lock: EntityWrapper<T>,
}

impl <T: ?Sized> EntityReference<T> {
	#[cfg(feature="multithreading")]
	pub fn get(&self) -> std::sync::RwLockReadGuard<dyn std::any::Any + Send + 'static> {
		self.lock.read().unwrap()
	}

	#[cfg(not(feature="multithreading"))]
	pub fn get(&self) -> std::cell::Ref<T> {
		self.lock.as_ref().borrow()
	}

	#[cfg(feature="multithreading")]
	pub fn get_mut(&self) -> std::sync::RwLockWriteGuard<dyn std::any::Any + Send + 'static> {
		self.lock.write().unwrap()
	}

	#[cfg(not(feature="multithreading"))]
	pub fn get_mut(&self) -> std::cell::RefMut<T> {
		self.lock.as_ref().borrow_mut()
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
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
	fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spawn() {
		let mut orchestrator = Orchestrator::new();

		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		impl super::Component for Component {
			// type Parameters<'a> = ComponentParameters;
		}

		let handle: EntityHandle<Component> = orchestrator.spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}
		impl super::System for System {}

		impl System {
			fn new<'c>() -> EntityReturn<'c, System> {
				EntityReturn::new(System {}).add_listener::<Component>()
			}
		}

		static mut COUNTER: u32 = 0;

		impl EntitySubscriber<Component> for System {
			fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, component: &Component) {
				unsafe {
					COUNTER += 1;
				}
			}

			fn on_update(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<Component>, params: &Component) {}
		}
		
		let _: Option<EntityHandle<System>> = orchestrator.spawn_entity(System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let component: EntityHandle<Component> = orchestrator.spawn(Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	// #[test]
	// fn events() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	struct MyComponent {
	// 		name: String,
	// 		value: u32,
	// 		click: bool,
	// 	}

	// 	impl MyComponent {
	// 		pub const fn click() -> EventDescription<MyComponent, bool> { EventDescription::new() }
	// 	}

	// 	impl Entity for MyComponent {}

	// 	impl Component for MyComponent {}

	// 	struct MySystem {

	// 	}

	// 	impl Entity for MySystem {}
	// 	impl System for MySystem {}

	// 	static mut COUNTER: u32 = 0;

	// 	impl MySystem {
	// 		fn new<'c>(component_handle: &EntityHandle<MyComponent>) -> EntityReturn<'c, MySystem> {
	// 			EntityReturn::new(MySystem {})
	// 		}

	// 		fn on_event(&mut self, _: OrchestratorReference, value: bool) {
	// 			unsafe {
	// 				COUNTER += 1;
	// 			}
	// 		}
	// 	}

	// 	let component_handle: EntityHandle<MyComponent> = orchestrator.spawn(MyComponent { name: "test".to_string(), value: 1, click: false });

	// 	let system_handle: EntityHandle<MySystem> = orchestrator.spawn_entity(MySystem::new(&component_handle)).unwrap();

	// 	component_handle.

	// 	orchestrator.subscribe_to(&system_handle, &component_handle, MyComponent::click, MySystem::on_event);

	// 	assert_eq!(unsafe { COUNTER }, 0);

	// 	orchestrator.trigger(&component_handle, MyComponent::click, true);

	// 	assert_eq!(unsafe { COUNTER }, 1);
	// }

	#[test]
	fn reactivity() {
		let mut orchestrator = Orchestrator::new();

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

		let mut source_component_handle: EntityHandle<SourceComponent> = orchestrator.spawn(SourceComponent { value, derived });
		let receiver_component_handle: EntityHandle<ReceiverComponent> = orchestrator.spawn(ReceiverComponent { value: source_component_handle.get_mut(|c| SinkProperty::new(&mut c.value)), derived: source_component_handle.get_mut(|c| SinkProperty::from_derived(&mut c.derived))});

		assert_eq!(source_component_handle.get(|c| c.value.get()), 1);
		assert_eq!(source_component_handle.get(|c| c.derived.get()), "1");
		assert_eq!(receiver_component_handle.get(|c| c.value.get()), 1);
		assert_eq!(receiver_component_handle.get(|c| c.derived.get()), "1");

		source_component_handle.get_mut(|c| c.value.set(|_| 2));

		assert_eq!(source_component_handle.get(|c| c.value.get()), 2);
		assert_eq!(source_component_handle.get(|c| c.derived.get()), "2");
		assert_eq!(receiver_component_handle.get(|c| c.value.get()), 2);
		assert_eq!(receiver_component_handle.get(|c| c.derived.get()), "2");
	}
}

pub struct OrchestratorReference<'a> {
	orchestrator: &'a Orchestrator,
	internal_id: u32,
}

impl <'a> OrchestratorReference<'a> {
	pub fn tie<'b, T: Entity + 'static, U: Entity + 'b, V: Any + Copy + 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> EventDescription<T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> EventDescription<U, V>) {
		self.orchestrator.tie(receiver_component_handle, i, sender_component_handle, j);
	}

	pub fn tie_self<T: Entity + 'static, U: Entity, V: Any + Copy + 'static>(&self, consuming_property: fn() -> EventDescription<T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> EventDescription<U, V>) {
		self.orchestrator.tie_internal(self.internal_id, consuming_property, sender_component_handle, j);
	}

	pub fn spawn_entity<'c, T, P, F: 'c>(&self, function: F) -> Option<EntityHandle<T>> where T: Entity + 'static, F: IntoHandler<P, T> {
		self.orchestrator.spawn_entity::<'c, T, P, F>(function)
	}

	pub fn spawn<C: Component>(&self, component: C) -> EntityHandle<C> {
		self.orchestrator.spawn::<C>(component)
	}

	pub fn set_property<C: Entity + 'static, V: Clone + Copy + 'static>(&self, component_handle: &EntityHandle<C>, property: fn() -> EventDescription<C, V>, value: V) {
		self.orchestrator.set_property::<C, V>(component_handle, property, value);
	}

	pub fn get_entity<S: System + ?Sized + 'static>(&self, entity_handle: &EntityHandle<S>) -> EntityReference<S> {
		self.orchestrator.get_entity::<S>(entity_handle)
	}
}

pub struct InternalId(pub u32);

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait IntoHandler<P, R: Entity> {
	fn call(self, orchestrator: &Orchestrator,) -> Option<EntityHandle<R>>;
}

impl <R: Entity + 'static> IntoHandler<(), R> for EntityReturn<'_, R> {
    fn call(self, orchestrator: &Orchestrator,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let entity = (self.create)(OrchestratorReference { orchestrator, internal_id });

		#[cfg(feature="multithreading")]
		let obj = std::sync::Arc::new(std::sync::RwLock::new(entity));

		#[cfg(not(feature="multithreading"))]
		let obj = std::rc::Rc::new(std::cell::RefCell::new(entity));

		{
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		let handle = EntityHandle::<R>::new(obj, internal_id, 0);

		{
			// let mut obj = obj.write().unwrap();

			for f in self.post_creation_functions {
				f(&handle, OrchestratorReference { orchestrator, internal_id });
			}
		}

		{
			for (type_id, f) in self.listens_to {
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