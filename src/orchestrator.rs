//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::{collections::HashMap, ops::Deref};

use crate::{Vector2, Vector3};

/// System handle is a handle to a system in an [`Orchestrator`]
pub struct SystemHandle(u32);

pub trait Entity {
	fn class() -> &'static str { std::any::type_name::<Self>() }
}

/// A system is a collection of components and logic to operate on them.
pub trait System : Entity {}

pub struct EntityHandle<T> {
	internal_id: u32,
	external_id: u32,
	phantom: std::marker::PhantomData<T>,
}

impl <T> std::hash::Hash for EntityHandle<T> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.internal_id.hash(state);
		self.external_id.hash(state);
	}
}

impl <T> PartialEq for EntityHandle<T> {
	fn eq(&self, other: &Self) -> bool {
		self.internal_id == other.internal_id && self.external_id == other.external_id
	}
}

impl <T> Eq for EntityHandle<T> {}

impl <T> EntityHandle<T> {
	pub fn get_external_key(&self) -> u32 { self.external_id }
	pub fn copy(&self) -> Self {
		Self {
			internal_id: self.internal_id,
			external_id: self.external_id,
			phantom: std::marker::PhantomData,
		}
	}
}

/// A component is a piece of data that is attached to an entity.
pub trait Component : Entity {
	type Parameters;
	fn new(orchestrator: OrchestratorReference, params: Self::Parameters) -> Self where Self: Sized;
}

pub trait OwnedComponent<T: Entity> : Entity {
}

enum UpdateFunctionTypes {
	Component(std::boxed::Box<dyn std::any::Any>),
	System(std::boxed::Box<dyn std::any::Any>),
}

struct Tie {
	update_function: UpdateFunctionTypes,
	destination_system_handle: u32,
}

/// An orchestrator is a collection of systems that are updated in parallel.
pub struct Orchestrator {
	sep: std::sync::Mutex<(crate::executor::Executor, crate::executor::Spawner)>,
	systems_data: std::sync::RwLock<SystemsData>,
	listeners_by_class: std::sync::Mutex<HashMap<&'static str, Vec<(u32, Box<dyn Fn(&Orchestrator, u32, (u32, u32))>)>>>,
	tasks: Vec<Task>,
	ties: std::sync::RwLock<HashMap<usize, Vec<Tie>>>,
}

unsafe impl Send for Orchestrator {}

type EntityStorage = std::sync::Arc<std::sync::RwLock<dyn std::any::Any + Send + 'static>>;

struct SystemsData {
	counter: u32,
	systems: HashMap<u32, EntityStorage>,
	systems_by_name: HashMap<&'static str, u32>,
}

trait SystemLock<T> {
	fn pls(&self) -> std::sync::Mutex<&T>;
}

struct Task {
	function: std::boxed::Box<dyn Fn(&Orchestrator)>,
}

type OrchestratorHandle = std::sync::Arc<Orchestrator>;

pub enum Property<S, E, V> {
	System {
		getter: fn(&S, &EntityHandle<E>) -> V,
		setter: fn(&mut S, &EntityHandle<E>, V),
	},
	Component {
		getter: fn(&E) -> V,
		setter: fn(&mut E, OrchestratorReference, V),
	},
}

impl Orchestrator {
	pub fn new() -> Orchestrator {
		Orchestrator {
			sep: std::sync::Mutex::new(crate::executor::new_executor_and_spawner()),
			systems_data: std::sync::RwLock::new(SystemsData { counter: 0, systems: HashMap::new(), systems_by_name: HashMap::new(), }),
			listeners_by_class: std::sync::Mutex::new(HashMap::new()),
			tasks: Vec::new(),
			ties: std::sync::RwLock::new(HashMap::new()),
		}
	}

	pub fn initialize(&self) {}
	pub fn deinitialize(&self) {}

	pub fn update(&mut self) {
		for task in self.tasks.iter() {
			(task.function)(self);
		}

		self.tasks.clear();
	}

	pub fn create_entity<C: Send + Sync + 'static>(&mut self, c: C) -> EntityHandle<C> {
		let obj = std::sync::Arc::new(std::sync::RwLock::new(c));

		let internal_id = {
			let mut systems_data = self.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		{
			let mut systems_data = self.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj);
		}

		let external_id = 0;

		EntityHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData }
	}

	pub fn spawn_entity<T, P, F>(&self, function: F) -> EntityHandle<T>
		where
			T: Entity + Send + 'static,
			F: IntoHandler<P, T> 
	{
		let handle = function.call(self);

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(T::class()) {
				for listener in listeners {
					(listener.1)(self, listener.0, (handle.internal_id, handle.external_id));
				}
			}
		}

		handle
	}

	pub fn spawn_component<C: Component + Send + 'static>(&self, parameters: C::Parameters) -> EntityHandle<C> {
		let internal_id = {
			let mut systems_data = self.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = std::sync::Arc::new(std::sync::RwLock::new(C::new(OrchestratorReference { orchestrator: self, internal_id }, parameters)));

		{
			let mut systems_data = self.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj);
			systems_data.systems_by_name.insert(C::class(), internal_id);
		}

		let external_id = 0;

		let handle = EntityHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData };

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(C::class()) {
				for listener in listeners {
					(listener.1)(self, listener.0, (handle.internal_id, handle.external_id));
				}
			}
		}

		handle
	}

	fn create_owned_entity<T: System + 'static, C: Clone + Send + 'static>(&self, internal_id: u32, id: u32) -> EntityHandle<C> {
		EntityHandle::<C> { internal_id, external_id: id, phantom: std::marker::PhantomData }
	}

	/// Ties a property of a component to a property of another component.
	pub fn tie<T: 'static, U, V: 'static, S0: 'static, S1: 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> Property<S0, T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		let property_function_pointer = j as *const (); // Use the property function pointer as a key to the ties hashmap.

		let property = i();

		let update_function = match property {
			Property::Component { getter, setter } => UpdateFunctionTypes::Component(std::boxed::Box::new(setter)),
			Property::System { getter, setter } => UpdateFunctionTypes::System(std::boxed::Box::new(setter)),
		};

		let mut ties = self.ties.write().unwrap();

		if ties.contains_key(&(property_function_pointer as usize)) {
			let ties = ties.get_mut(&(property_function_pointer as usize)).unwrap();

			if !ties.iter().any(|tie| tie.destination_system_handle == receiver_component_handle.internal_id) {
				ties.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
			}
		} else {
			let mut ties_new = Vec::new();
			ties_new.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
			ties.insert(property_function_pointer as usize, ties_new);
		}
	}

	/// Subscribes an entity to notifications of operations related to a entity class.
	fn subscribe_to_class<S: System + 'static, T: Component + 'static, F: Fn(&mut S, OrchestratorReference, EntityHandle<T>, &T) + 'static>(&self, internal_id: u32, function: F) {
		{
			let mut listeners = self.listeners_by_class.lock().unwrap();
			
			let listeners = listeners.entry(T::class()).or_insert(Vec::new());

			listeners.push((internal_id, Box::new(move |orchestrator: &Orchestrator, entity_to_notify: u32, ha: (u32, u32)| {
				let systems_data = orchestrator.systems_data.read().unwrap();
				let mut system = systems_data.systems[&entity_to_notify].write().unwrap();
				let system = system.downcast_mut::<S>().unwrap();
				let orchestrator_reference = OrchestratorReference { orchestrator, internal_id: entity_to_notify };

				let component = systems_data.systems[&ha.0].read().unwrap();
				let entity = component.downcast_ref::<T>().unwrap();

				function(system, orchestrator_reference, EntityHandle::<T> { internal_id: ha.0, external_id: ha.1, phantom: std::marker::PhantomData }, entity);
			})));
		}
	}

	pub fn set_property<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &EntityHandle<C>, function: fn() -> Property<S, C, V>, value: V) {
		let po = function as *const ();
		let ties = self.ties.read().unwrap();
		
		if let Some(ties) = ties.get(&(po as usize)) {
			let systems_data = self.systems_data.read().unwrap();

			for tie in ties {
				unsafe {
					match tie.update_function {
						UpdateFunctionTypes::Component(ref setter) => {
							let mut component = systems_data.systems[&tie.destination_system_handle].write().unwrap();
							let setter = setter.downcast_ref_unchecked::<fn(&mut C, OrchestratorReference, V)>();
							(setter)(component.downcast_mut_unchecked::<C>(), OrchestratorReference { orchestrator: self, internal_id: tie.destination_system_handle }, value);
						},
						UpdateFunctionTypes::System(ref setter) => {
							let mut component = systems_data.systems[&tie.destination_system_handle].write().unwrap();
							let setter = setter.downcast_ref_unchecked::<fn(&mut S, &EntityHandle<C>, V)>();
							(setter)(component.downcast_mut_unchecked::<S>(), component_handle, value);
						},
					}
				}
			}
		}
	}

	pub fn execute_task_standalone(&self, task: impl std::future::Future<Output = ()> + Send + 'static) {
		self.sep.lock().unwrap().1.spawn(task);
	}

	pub fn execute_task<S: System + Sync + Send, F: Sync + Send + 'static, R>(&self, task: F) where F: FnOnce(std::sync::Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync + 'static>>, OrchestratorHandle) -> R, R: std::future::Future<Output = ()> + Send + 'static {
		self.sep.lock().unwrap().1.spawn(async move {
		});
	}

	pub fn execute_task_sync<F: Fn(&Orchestrator) + 'static>(&mut self, task: F) {
		self.tasks.push(Task { function: std::boxed::Box::new(task) });
	}

	pub fn get_and<C: 'static, F, R>(&self, component_handle: &EntityHandle<C>, function: F) -> R where F: FnOnce(&C) -> R {
		let systems_data = self.systems_data.read().unwrap();
		let component = systems_data.systems[&component_handle.internal_id].read().unwrap();
		function(component.downcast_ref::<C>().unwrap())
	}

	pub fn get_mut_and<C: 'static, F, R>(&self, component_handle: &EntityHandle<C>, function: F) -> R where F: FnOnce(&mut C) -> R {
		let systems_data = self.systems_data.read().unwrap();
		let mut component = systems_data.systems[&component_handle.internal_id].write().unwrap();
		function(component.downcast_mut::<C>().unwrap())
	}

	pub fn get_2_mut_and<C0: 'static, C1: 'static, F, R>(&self, component_handle_0: &EntityHandle<C0>, component_handle_1: &EntityHandle<C1>, function: F) -> R where F: FnOnce(&mut C0, &mut C1) -> R {
		let systems_data = self.systems_data.read().unwrap();
		let mut component_0 = systems_data.systems[&component_handle_0.internal_id].write().unwrap();
		let mut component_1 = systems_data.systems[&component_handle_1.internal_id].write().unwrap();

		function(component_0.downcast_mut::<C0>().unwrap(), component_1.downcast_mut::<C1>().unwrap())
	}

	// pub fn get_mut_by_class_and<C: System + 'static, F, R>(&self, function: F) -> R where F: FnOnce(&mut C) -> R {
	// 	let systems_data = self.systems_data.read().unwrap();
	// 	let mut component = systems_data.systems[systems_data.systems_by_name[C::class()] as usize].write().unwrap();
	// 	function(component.downcast_mut::<C>().unwrap())
	// }

	pub fn get_property<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &EntityHandle<C>, function: fn() -> Property<S, C, V>) -> V {
		let systems_data = self.systems_data.read().unwrap();

		let property = function();

		match property {
			Property::Component { getter, setter: _ } => {
				let component = systems_data.systems[&component_handle.internal_id].read().unwrap();
				let getter = getter as *const ();
				let getter = unsafe { std::mem::transmute::<*const (), fn(&C) -> V>(getter) };
				(getter)(component.downcast_ref::<C>().unwrap())
			},
			Property::System { getter, setter: _ } => {
				let component = systems_data.systems[&component_handle.internal_id].read().unwrap();
				let getter = getter as *const ();
				let getter = unsafe { std::mem::transmute::<*const (), fn(&S, &EntityHandle<C>) -> V>(getter) };
				(getter)(component.downcast_ref::<S>().unwrap(), component_handle)
			},
		}
	}

	pub fn set_owned_property<C: Clone + 'static, V: Clone + Copy + 'static, S: 'static>(&self, internal_id: u32, component_handle: InternalId, function: fn() -> Property<S, C, V>, value: V) {
		let systems_data = self.systems_data.read().unwrap();

		let property = function();

		let ties = self.ties.read().unwrap();

		if let Some(ties) = ties.get(&(function as *const () as usize)) {
			for tie in ties {
				unsafe {
					match tie.update_function {
						UpdateFunctionTypes::Component(ref setter) => {
							let mut component = systems_data.systems[&tie.destination_system_handle].write().unwrap();
							let setter = setter.downcast_ref_unchecked::<fn(&mut C, OrchestratorReference, V)>();
							(setter)(component.downcast_mut_unchecked::<C>(), OrchestratorReference { orchestrator: self, internal_id: tie.destination_system_handle }, value);
						},
						UpdateFunctionTypes::System(ref setter) => {
							let mut component = systems_data.systems[&tie.destination_system_handle].write().unwrap();
							let setter = setter.downcast_ref_unchecked::<fn(&mut S, &EntityHandle<C>, V)>();
							(setter)(component.downcast_mut_unchecked::<S>(), &EntityHandle::<C>{ internal_id, external_id: component_handle.0, phantom: std::marker::PhantomData }, value);
						},
					}
				}
			}
		}
	}

	pub fn invoke<E: Entity + 'static>(&self, handle: EntityHandle<E>, function: fn(&E, &Orchestrator)) {
		let systems_data = self.systems_data.read().unwrap();
		let component = systems_data.systems[&handle.internal_id].read().unwrap();
		let component = component.downcast_ref::<E>().unwrap();
		function(component, self);
	}

	pub fn invoke_mut<E: Entity + 'static>(&self, handle: EntityHandle<E>, function: fn(&mut E, OrchestratorReference)) {
		let systems_data = self.systems_data.read().unwrap();
		let mut component = systems_data.systems[&handle.internal_id].write().unwrap();
		let component = component.downcast_mut::<E>().unwrap();
		function(component, OrchestratorReference { orchestrator: self, internal_id: handle.external_id });
	}

	pub fn get_by_class<S: System + 'static>(&self) -> EntityReference<S> {
		let systems_data = self.systems_data.read().unwrap();
		EntityReference { lock: systems_data.systems[&systems_data.systems_by_name[S::class()]].clone(), phantom: std::marker::PhantomData }
	}
}

pub struct EntityReference<T> {
	lock: std::sync::Arc<std::sync::RwLock<dyn std::any::Any + Send + 'static>>,
	phantom: std::marker::PhantomData<T>,
}

impl <T> EntityReference<T> {
	pub fn get(&self) -> std::sync::RwLockReadGuard<dyn std::any::Any + Send + 'static> {
		self.lock.read().unwrap()
	}

	pub fn get_mut(&self) -> std::sync::RwLockWriteGuard<dyn std::any::Any + Send + 'static> {
		self.lock.write().unwrap()
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
	use super::*;

	struct TestSystem {
		value: u32,
	}

	impl Entity for TestSystem {}
	impl System for TestSystem {}

	impl TestSystem {
		fn new() -> TestSystem {
			TestSystem { value: 0 }
		}

		async fn update(&mut self) {
			self.value += 1;
		}

		async fn task(&self, orchestrator: OrchestratorHandle) {
			println!("{}", self.value);
		}

		fn get_value(&self) -> u32 {
			self.value
		}
	}

	#[test]
	fn test_one_off_task() {
		let mut orchestrator = Orchestrator::new();

		let mut test_system_handle = TestSystem::new();

		orchestrator.execute_task_standalone(async move {
			assert_eq!(test_system_handle.get_value(), 0);
			test_system_handle.update().await;
			assert_eq!(test_system_handle.get_value(), 1);
		});

		orchestrator.update();
	}

	#[test]
	fn test_systems() {
		let mut orchestrator = Orchestrator::new();

		let test_system = TestSystem::new();

		let system_handle = orchestrator.create_entity(test_system);

		//orchestrator.execute_task(TestSystem::task);

		orchestrator.update();
	}

	#[test]
	fn sync_task() {
		let mut orchestrator = Orchestrator::new();

		let test_system = TestSystem::new();

		let system_handle = orchestrator.create_entity(test_system);

		fn task(o: &Orchestrator) {
			println!("howdy!");
		}

		orchestrator.execute_task_sync(task);

		orchestrator.update();
	}

	#[test]
	fn tie() {
		let mut orchestrator = Orchestrator::new();

		#[derive(Clone)]
		struct Sender {
			value: u32,
		}

		impl Sender {
			fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Sender> {
				orchestrator.create_entity(Sender { value: 0 })
			}

			fn get_value(&self) -> u32 {
				self.value
			}

			fn set_value(&mut self, orchestrator: OrchestratorReference, value: u32) {
				self.value = value;
			}

			const fn send() -> Property<(), Sender, u32> {
				Property::Component { getter: Sender::get_value, setter: Sender::set_value }
			}
		}

		#[derive(Clone)]
		struct Receiver {
			value: u32,
		}

		impl Receiver {
			fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Receiver> {
				orchestrator.create_entity(Receiver { value: 0 })
			}

			fn read_value(&self) -> u32 {
				self.value
			}

			fn set_value(&mut self, orchestrator: OrchestratorReference, value: u32) {
				self.value = value;
			}

			const fn value() -> Property<(), Receiver, u32> {
				Property::Component { getter: Receiver::read_value, setter: Receiver::set_value }
			}
		}

		let sender_handle = Sender::new(&mut orchestrator);

		let receiver_handle = Receiver::new(&mut orchestrator);

		let value = orchestrator.get_property(&receiver_handle, Receiver::value);

		assert_eq!(value, 0);

		orchestrator.tie(&receiver_handle, Receiver::value, &sender_handle, Sender::send);

		orchestrator.set_property(&sender_handle, Sender::send, 5);

		let value = orchestrator.get_and(&receiver_handle, |r| r.value);

		assert_eq!(value, 5);
	}

	// #[test]
	// fn system_owned_components() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	struct Sender {
	// 		value: u32,
	// 	}

	// 	impl Sender {
	// 		fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Sender> {
	// 			orchestrator.make_object(Sender { value: 0 })
	// 		}

	// 		fn get_value(&self) -> u32 { self.value }
	// 		fn set_value(&mut self, value: u32) { self.value = value; }

	// 		const fn send() -> Property<(), Sender, u32> { Property::Component { getter: Sender::get_value, setter: Sender::set_value } }
	// 	}

	// 	struct Component {
	// 		// No data in component as it is managed/owned by the system.	
	// 	}

	// 	struct System {
	// 		handle: EntityHandle<System>,
	// 		data: Vec<u32>,
	// 	}

	// 	impl System {
	// 		fn new(orchestrator: &mut Orchestrator) -> EntityHandle<System> {
	// 			orchestrator.make_object_with_id("System", |handle| {
	// 				System { handle: handle, data: Vec::new() }
	// 			})
	// 		}

	// 		fn create_component(orchestrator: &mut Orchestrator, value: u32) -> EntityHandle<Component> {
	// 			orchestrator.get_mut_by_name_and("System", |system: &mut System| {
	// 				let external_id = system.data.len() as u32;
	// 				system.data.push(value);
	// 				orchestrator.make_handle(&system.handle, external_id)
	// 			})
	// 		}

	// 		fn set_component_value(&mut self, component_handle: &EntityHandle<Component>, value: u32) {
	// 			self.data[component_handle.external_id as usize] = value;
	// 		}

	// 		fn get_component_value(&self, component_handle: &EntityHandle<Component>) -> u32 {
	// 			self.data[component_handle.external_id as usize]
	// 		}
	// 	}

	// 	impl super::System for System {}

	// 	impl Component {
	// 		fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Component> {
	// 			System::create_component(orchestrator, 0)
	// 		}

	// 		const fn value() -> Property<System, Component, u32> {
	// 			Property::System { getter: System::get_component_value, setter: System::set_component_value }
	// 		}
	// 	}

	// 	let sender_handle = Sender::new(&mut orchestrator);

	// 	System::new(&mut orchestrator);

	// 	let component_handle = Component::new(&mut orchestrator);

	// 	let value = orchestrator.get_property(&component_handle, Component::value);

	// 	assert_eq!(value, 0);

	// 	orchestrator.tie(&component_handle, Component::value, &sender_handle, Sender::send);

	// 	orchestrator.notify(&sender_handle, Sender::send, 5);

	// 	let value = orchestrator.get_property(&component_handle, Component::value);

	// 	assert_eq!(value, 5);
	// }
}

pub struct OrchestratorReference<'a> {
	orchestrator: &'a Orchestrator,
	internal_id: u32,
}

impl <'a> OrchestratorReference<'a> {
	pub fn spawn_component<C: Component + Send + 'static>(&self, parameters: C::Parameters) -> EntityHandle<C> {
		self.orchestrator.spawn_component(parameters)
	}

	pub fn tie<'b, T: 'static, U: 'b, V: 'static, S0: 'static, S1: 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> Property<S0, T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		self.orchestrator.tie(receiver_component_handle, i, sender_component_handle, j);
	}

	pub fn tie_self<T: 'static, U, V: 'static, S0: 'static, S1: 'static>(&self, consuming_property: fn() -> Property<S0, T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		self.orchestrator.tie(&EntityHandle::<T>{ internal_id: self.internal_id, external_id: 0, phantom: std::marker::PhantomData }, consuming_property, sender_component_handle, j);
	}

	pub fn create_owned_entity<T: System + 'static, C: Clone + Send + 'static>(&mut self, id: u32) -> EntityHandle<C> {
		self.orchestrator.create_owned_entity::<T, C>(self.internal_id, id)
	}

	pub fn subscribe_to_class<T: System + 'static, C: Component + 'static, F: Fn(&mut T, OrchestratorReference, EntityHandle<C>, &C) + 'static>(&mut self, function: F) {
		self.orchestrator.subscribe_to_class::<T, C, F>(self.internal_id, function);
	}

	pub fn spawn_entity<T, P, F>(&self, function: F) -> EntityHandle<T>
		where
			T: Entity + Send + 'static,
			F: IntoHandler<P, T> 
	{
		self.orchestrator.spawn_entity::<T, P, F>(function)
	}

	pub fn set_property<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &EntityHandle<C>, property: fn() -> Property<S, C, V>, value: V) {
		self.orchestrator.set_property::<C, V, S>(component_handle, property, value);
	}

	pub fn set_owned_property<T: Copy + 'static, S: 'static, E: Clone + 'static>(&self, internal_id: InternalId, property: fn() -> Property<S, E, T>, value: T) {
		self.orchestrator.set_owned_property::<E, T, S>(self.internal_id, internal_id, property, value);
	}

	pub fn get_by_class<S: System + 'static>(&self) -> EntityReference<S> {
		self.orchestrator.get_by_class::<S>()
	}

	pub fn get_property<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &EntityHandle<C>, property: fn() -> Property<S, C, V>) -> V {
		self.orchestrator.get_property::<C, V, S>(component_handle, property)
	}
}

pub struct InternalId(pub u32);

/// Handles extractor pattern for most functions passed to the orchestrator.
pub trait IntoHandler<P, R> {
	fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R>;
}

impl <F, P0, P1, P2, R: Send + 'static> IntoHandler<(P0, P1, P2), R> for F where
	F: Fn(OrchestratorReference, &mut P0, &mut P1, &mut P2) -> InternalId + 'static,
	P0: Entity + 'static,
	P1: Entity + 'static,
	P2: Entity + 'static,
{
	fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R> {
		let systems_data = orchestrator.systems_data.read().unwrap();

		let internal_id = systems_data.systems_by_name[P0::class()];

		let mut system0_lock = systems_data.systems[&systems_data.systems_by_name[P0::class()]].write().unwrap();
		let system0 = system0_lock.downcast_mut::<P0>().unwrap();

		let mut system1_lock = systems_data.systems[&systems_data.systems_by_name[P1::class()]].write().unwrap();
		let system1 = system1_lock.downcast_mut::<P1>().unwrap();

		let mut system2_lock = systems_data.systems[&systems_data.systems_by_name[P2::class()]].write().unwrap();
		let system2 = system2_lock.downcast_mut::<P2>().unwrap();

		let obj = (self)(OrchestratorReference { orchestrator, internal_id }, system0, system1, system2);
		
		EntityHandle::<R> { internal_id, external_id: obj.0, phantom: std::marker::PhantomData }
    }
}

impl <F, R> IntoHandler<(), R> for (R, F) where
	R: Entity + Send + 'static,
	F: Fn(OrchestratorReference, R) -> R
{
	fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R> {
		let internal_id = {
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = (self.1)(OrchestratorReference { orchestrator, internal_id }, self.0);
		let obj = std::sync::Arc::new(std::sync::RwLock::new(obj));
		
		{
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj);
			systems_data.systems_by_name.insert(R::class(), internal_id);
		}

		EntityHandle::<R> { internal_id, external_id: 0, phantom: std::marker::PhantomData }
    }
}

impl <F, P0, P1, P2, R: Send> IntoHandler<P0, R> for (F, P1, P2) where
	F: Fn(OrchestratorReference, &mut P0, P1, P2) -> InternalId,
	P0: Entity + 'static,
{
	fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R> {
		let systems_data = orchestrator.systems_data.read().unwrap();
		let internal_id = systems_data.systems_by_name[P0::class()];

		let mut system0_lock = systems_data.systems[&systems_data.systems_by_name[P0::class()]].write().unwrap();
		let system0 = system0_lock.downcast_mut::<P0>().unwrap();

		let obj = (self.0)(OrchestratorReference { orchestrator, internal_id }, system0, self.1, self.2);
		
		EntityHandle::<R> { internal_id, external_id: obj.0, phantom: std::marker::PhantomData }
    }
}

impl <F, R: Entity + Send + 'static> IntoHandler<(), R> for F where
    F: Fn(OrchestratorReference) -> R,
{
    fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R> {
		let internal_id = {
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = (self)(OrchestratorReference { orchestrator, internal_id });
		let obj = std::sync::Arc::new(std::sync::RwLock::new(obj));

		{
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj);
			systems_data.systems_by_name.insert(R::class(), internal_id);
		}

		EntityHandle::<R> { internal_id, external_id: 0, phantom: std::marker::PhantomData }
    }
}

impl <F, P0, R: Send + 'static> IntoHandler<P0, R> for F where
	P0: Entity + 'static,
	R: Entity + Send + 'static,
	F: Fn(OrchestratorReference, &mut P0) -> R,
{
    fn call(self, orchestrator: &Orchestrator,) -> EntityHandle<R> {
		let internal_id = {
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let obj = {
			let systems_data = orchestrator.systems_data.read().unwrap();
			let mut system0_lock = systems_data.systems[&systems_data.systems_by_name[P0::class()]].write().unwrap();
			let system0 = system0_lock.downcast_mut::<P0>().unwrap();

			(self)(OrchestratorReference { orchestrator, internal_id }, system0)
		};

		{
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let obj = std::sync::Arc::new(std::sync::RwLock::new(obj));
			systems_data.systems.insert(internal_id, obj);
			systems_data.systems_by_name.insert(R::class(), internal_id);
		}

		EntityHandle::<R> { internal_id, external_id: 0, phantom: std::marker::PhantomData }
    }
}