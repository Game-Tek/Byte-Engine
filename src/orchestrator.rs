//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::{collections::HashMap, any::Any};

use log::{trace, warn};

/// System handle is a handle to a system in an [`Orchestrator`]
pub struct SystemHandle(u32);

pub trait Entity: Any + Send + 'static {
	fn to_subclass(&self) -> &dyn Any where Self: Sized {
		self
	}
}

/// A system is a collection of components and logic to operate on them.
pub trait System: Entity + Any {}

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
	pub fn get_contents(&self) -> (u32, u32) {
		(self.internal_id, self.external_id)
	}
	pub fn from_contents(contents: (u32, u32)) -> Self {
		Self {
			internal_id: contents.0,
			external_id: contents.1,
			phantom: std::marker::PhantomData,
		}
	}
}

/// A component is a piece of data that is attached to an entity.
pub trait Component : Entity {
	// type Parameters<'a>: Send + Sync;
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
	// parameters: std::sync::RwLock<HashMap<u32, std::sync::Arc<dyn std::any::Any + Send + Sync + 'static>>>,
}

unsafe impl Send for Orchestrator {}

// type EntityStorage = std::sync::Arc<std::sync::RwLock<dyn Entity>>;
type EntityStorage = std::sync::Arc<std::sync::RwLock<dyn Any + Send + 'static>>;

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

pub enum PPP<T> {
	PostCreationFunction(std::boxed::Box<dyn Fn(&mut T, OrchestratorReference,)>),
}

/// Entity creation functions must return this type.
pub struct EntityReturn<T> {
	// entity: T,
	create: std::boxed::Box<dyn FnOnce(OrchestratorReference) -> T>,
	post_creation_functions: Vec<std::boxed::Box<dyn Fn(&mut T, OrchestratorReference)>>,
	listens_to: Vec<(&'static str, Box<dyn Fn(&Orchestrator, u32, (u32, u32))>)>,
}

impl <T: 'static> EntityReturn<T> {
	// pub fn new(entity: T) -> Self {
	// 	Self {
	// 		entity,
	// 		post_creation_functions: Vec::new(),
	// 		listens_to: Vec::new(),
	// 	}
	// }

	pub fn new(entity: T) -> Self {
		Self {
			create: std::boxed::Box::new(move |_| entity),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn new_from_function(function: fn(OrchestratorReference) -> T) -> Self {
		Self {
			create: std::boxed::Box::new(function),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn new_from_closure<'a, F: FnOnce(OrchestratorReference) -> T + 'static>(function: F) -> Self {
		Self {
			create: std::boxed::Box::new(function),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn add_post_creation_function(mut self, function: std::boxed::Box<dyn Fn(&mut T, OrchestratorReference)>) -> Self {
		self.post_creation_functions.push(function);
		self
	}

	pub fn add_listener<C: Component>(mut self,) -> Self where T: EntitySubscriber<C> {
		// TODO: Notify listener of the entities that existed before they started to listen.
		// Maybe add a parameter to choose whether to listen retroactively or not. With a default value of true.

		let b = Box::new(move |orchestrator: &Orchestrator, entity_to_notify: u32, ha: (u32, u32)| {
			let systems_data = orchestrator.systems_data.read().unwrap();

			let mut lock_guard = systems_data.systems[&entity_to_notify].write().unwrap();
			let system: &mut T = lock_guard.downcast_mut().unwrap();
			let orchestrator_reference = OrchestratorReference { orchestrator, internal_id: entity_to_notify };

			let component = systems_data.systems[&ha.0].read().unwrap();
			let component: &C = component.downcast_ref().unwrap();

			T::on_create(system, orchestrator_reference, EntityHandle::<C> { internal_id: ha.0, external_id: ha.1, phantom: std::marker::PhantomData }, component);
		});

		self.listens_to.push((std::any::type_name::<C>(), b));

		self
	}
}

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
			// parameters: std::sync::RwLock::new(HashMap::new()),
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

	/// Spawn entity is a function that spawns an entity and returns a handle to it.
	pub fn spawn_entity<T, P, F>(&self, function: F) -> Option<EntityHandle<T>> where T: Entity + Send + 'static, F: IntoHandler<P, T> {
		let handle = function.call(self)?;

		trace!("{}", std::any::type_name::<T>());

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(std::any::type_name::<T>()) {
				for listener in listeners {
					(listener.1)(self, listener.0, (handle.internal_id, handle.external_id));
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

		{
			let mut systems_data = self.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, std::sync::Arc::new(std::sync::RwLock::new(component)));
			systems_data.systems_by_name.insert(std::any::type_name::<C>(), internal_id);

			// self.parameters.write().unwrap().insert(internal_id, std::sync::Arc::new(component));
		}

		let external_id = 0;

		let handle = EntityHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData };

		{
			let systems_data = self.listeners_by_class.lock().unwrap();
			if let Some(listeners) = systems_data.get(std::any::type_name::<C>()) {
				for listener in listeners {
					(listener.1)(self, listener.0, (handle.internal_id, handle.external_id));
				}
			} else {
				warn!("No listeners for {}", std::any::type_name::<C>());
			}
		}

		handle
	}

	/// Ties a property of a component to a property of another component.
	pub fn tie<T: 'static, U, V: 'static, S0: 'static, S1: 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> Property<S0, T, V>, _sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		let property_function_pointer = j as *const (); // Use the property function pointer as a key to the ties hashmap.

		let property = i();

		let update_function = match property {
			Property::Component { getter: _, setter } => UpdateFunctionTypes::Component(std::boxed::Box::new(setter)),
			Property::System { getter: _, setter } => UpdateFunctionTypes::System(std::boxed::Box::new(setter)),
		};

		let mut ties = self.ties.write().unwrap();

		if let std::collections::hash_map::Entry::Vacant(e) = ties.entry(property_function_pointer as usize) {
			let mut ties_new = Vec::new();
			ties_new.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
			e.insert(ties_new);
		} else {
			let ties = ties.get_mut(&(property_function_pointer as usize)).unwrap();

			if !ties.iter().any(|tie| tie.destination_system_handle == receiver_component_handle.internal_id) {
				ties.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
			}
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

	pub fn execute_task<S: System + Sync + Send, F: Sync + Send + 'static, R>(&self, _task: F) where F: FnOnce(std::sync::Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync + 'static>>, OrchestratorHandle) -> R, R: std::future::Future<Output = ()> + Send + 'static {
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

		let _property = function();

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

	pub fn get_by_class<S: System + ?Sized + 'static>(&self) -> EntityReference<S> {
		let systems_data = self.systems_data.read().unwrap();
		EntityReference { lock: systems_data.systems[&systems_data.systems_by_name[std::any::type_name::<S>()]].clone(), phantom: std::marker::PhantomData }
	}
}

pub struct EntityReference<T> where T: ?Sized {
	lock: std::sync::Arc<std::sync::RwLock<dyn std::any::Any + Send + 'static>>,
	phantom: std::marker::PhantomData<T>,
}

impl <T: ?Sized> EntityReference<T> {
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

pub trait EntitySubscriber<T: Entity + Component> {
	fn on_create(&mut self, orchestrator: OrchestratorReference, handle: EntityHandle<T>, params: &T);
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

		async fn task(&self, _orchestrator: OrchestratorHandle) {
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
	fn spawn() {
		let mut orchestrator = Orchestrator::new();

		struct Component {
			name: String,
			value: u32,
		}

		// struct ComponentParameters {
		// 	name: String,
		// }

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
			fn new() -> EntityReturn<System> {
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
		}
		
		orchestrator.spawn_entity(System::new());
		
		assert_eq!(unsafe { COUNTER }, 0);

		let component: EntityHandle<Component> = orchestrator.spawn(Component { name: "test".to_string(), value: 1 });

		assert_eq!(unsafe { COUNTER }, 1);
	}

	// #[test]
	// fn test_systems() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	let test_system = TestSystem::new();

	// 	let _system_handle = orchestrator.create_entity(test_system);

	// 	//orchestrator.execute_task(TestSystem::task);

	// 	orchestrator.update();
	// }

	// #[test]
	// fn sync_task() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	let test_system = TestSystem::new();

	// 	let _system_handle = orchestrator.spawn_system(test_system);

	// 	fn task(_o: &Orchestrator) {
	// 		println!("howdy!");
	// 	}

	// 	orchestrator.execute_task_sync(task);

	// 	orchestrator.update();
	// }

	// #[test]
	// fn tie() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	#[derive(Clone)]
	// 	struct Sender {
	// 		value: u32,
	// 	}

	// 	impl Entity for Sender {}

	// 	impl Sender {
	// 		fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Sender> {
	// 			orchestrator.create_entity(Sender { value: 0 })
	// 		}

	// 		fn get_value(&self) -> u32 {
	// 			self.value
	// 		}

	// 		fn set_value(&mut self, _orchestrator: OrchestratorReference, value: u32) {
	// 			self.value = value;
	// 		}

	// 		const fn send() -> Property<(), Sender, u32> {
	// 			Property::Component { getter: Sender::get_value, setter: Sender::set_value }
	// 		}
	// 	}

	// 	#[derive(Clone)]
	// 	struct Receiver {
	// 		value: u32,
	// 	}

	// 	impl Entity for Receiver {}

	// 	impl Receiver {
	// 		fn new(orchestrator: &mut Orchestrator) -> EntityHandle<Receiver> {
	// 			orchestrator.create_entity(Receiver { value: 0 })
	// 		}

	// 		fn read_value(&self) -> u32 {
	// 			self.value
	// 		}

	// 		fn set_value(&mut self, _orchestrator: OrchestratorReference, value: u32) {
	// 			self.value = value;
	// 		}

	// 		const fn value() -> Property<(), Receiver, u32> {
	// 			Property::Component { getter: Receiver::read_value, setter: Receiver::set_value }
	// 		}
	// 	}

	// 	let sender_handle = Sender::new(&mut orchestrator);

	// 	let receiver_handle = Receiver::new(&mut orchestrator);

	// 	let value = orchestrator.get_property(&receiver_handle, Receiver::value);

	// 	assert_eq!(value, 0);

	// 	orchestrator.tie(&receiver_handle, Receiver::value, &sender_handle, Sender::send);

	// 	orchestrator.set_property(&sender_handle, Sender::send, 5);

	// 	let value = orchestrator.get_and(&receiver_handle, |r| r.value);

	// 	assert_eq!(value, 5);
	// }

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
	pub fn tie<'b, T: 'static, U: 'b, V: 'static, S0: 'static, S1: 'static>(&self, receiver_component_handle: &EntityHandle<T>, i: fn() -> Property<S0, T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		self.orchestrator.tie(receiver_component_handle, i, sender_component_handle, j);
	}

	pub fn tie_self<T: 'static, U, V: 'static, S0: 'static, S1: 'static>(&self, consuming_property: fn() -> Property<S0, T, V>, sender_component_handle: &EntityHandle<U>, j: fn() -> Property<S1, U, V>) {
		self.orchestrator.tie(&EntityHandle::<T>{ internal_id: self.internal_id, external_id: 0, phantom: std::marker::PhantomData }, consuming_property, sender_component_handle, j);
	}

	pub fn spawn_entity<T, P, F>(&self, function: F) -> Option<EntityHandle<T>> where T: Entity + Send + 'static, F: IntoHandler<P, T> {
		self.orchestrator.spawn_entity::<T, P, F>(function)
	}

	pub fn spawn<C: Component>(&self, component: C) -> EntityHandle<C> {
		self.orchestrator.spawn::<C>(component)
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
	fn call(self, orchestrator: &Orchestrator,) -> Option<EntityHandle<R>>;
}

impl <R: Entity + Send + 'static> IntoHandler<(), R> for EntityReturn<R> {
    fn call(self, orchestrator: &Orchestrator,) -> Option<EntityHandle<R>> {
		let internal_id = {
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			let internal_id = systems_data.counter;
			systems_data.counter += 1;
			internal_id
		};

		let entity = (self.create)(OrchestratorReference { orchestrator, internal_id });
		let obj = std::sync::Arc::new(std::sync::RwLock::new(entity));

		{
			let mut systems_data = orchestrator.systems_data.write().unwrap();
			systems_data.systems.insert(internal_id, obj.clone());
			systems_data.systems_by_name.insert(std::any::type_name::<R>(), internal_id);
		}

		{
			let mut obj = obj.write().unwrap();

			for f in self.post_creation_functions {
				f(&mut obj, OrchestratorReference { orchestrator, internal_id });
			}
		}

		{
			// let mut obj = obj.write().unwrap();

			for (type_id, f) in self.listens_to {
				let mut listeners = orchestrator.listeners_by_class.lock().unwrap();
			
				let listeners = listeners.entry(type_id).or_insert(Vec::new());

				listeners.push((internal_id, f));
			}
		}

		Some(EntityHandle::<R> { internal_id, external_id: 0, phantom: std::marker::PhantomData })
    }
}