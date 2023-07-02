//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

use std::collections::HashMap;

/// System handle is a handle to a system in an [`Orchestrator`]
pub struct SystemHandle(u32);

/// A system is a collection of components and logic to operate on them.
pub trait System {
	fn class() -> &'static str { std::any::type_name::<Self>() }
}

#[derive(Clone, Copy)]
pub struct ComponentHandle<T> {
	internal_id: u32,
	external_id: u32,
	phantom: std::marker::PhantomData<T>,
}

impl <T> ComponentHandle<T> {
	pub fn get_external_key(&self) -> u32 { self.external_id }
	pub fn copy(&self) -> Self {
		Self {
			internal_id: self.internal_id,
			external_id: self.external_id,
			phantom: std::marker::PhantomData,
		}
	}
}

pub trait Component<T> {
	fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<T>;
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
	systems_data: SystemsData,
	tasks: Vec<Task>,
	ties: HashMap<usize, Vec<Tie>>,
}

unsafe impl Send for Orchestrator {}

struct SystemsData {
	systems: Vec<std::sync::Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync + 'static>>>,
	systems_by_name: HashMap<&'static str, u32>,
}

trait SystemLock<T> {
	fn pls(&self) -> std::sync::Mutex<&T>;
}

struct Task {
	function: std::boxed::Box<dyn Fn(&Orchestrator)>,
}

type OrchestratorHandle = std::sync::Arc<Orchestrator>;

pub enum Property<S, O, V> {
	System {
		getter: fn(&S, &ComponentHandle<O>) -> V,
		setter: fn(&mut S, &ComponentHandle<O>, V),
	},
	Component {
		getter: fn(&O) -> V,
		setter: fn(&mut O, V),
	},
}

impl Orchestrator {
	pub fn new() -> Orchestrator {
		Orchestrator {
			sep: std::sync::Mutex::new(crate::executor::new_executor_and_spawner()),
			systems_data: SystemsData { systems: Vec::new(), systems_by_name: HashMap::new() },
			tasks: Vec::new(),
			ties: HashMap::new(),
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

	pub fn add_system<T: System + Sync + Send + 'static>(&mut self, system: T) -> ComponentHandle<T> {
		let system_handle = self.systems_data.systems.len() as u32;
		self.systems_data.systems.push(std::sync::Arc::new(std::sync::Mutex::new(system)));
		self.systems_data.systems_by_name.insert(T::class(), system_handle);
		ComponentHandle::<T> { internal_id: system_handle, external_id: 0, phantom: std::marker::PhantomData }
	}

	pub fn make<C: Send + Sync + 'static>(&self, name: &'static str, external_id: u32) -> ComponentHandle<C> {
		let internal_id = self.systems_data.systems_by_name[name];
		ComponentHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData }
	}

	pub fn make_object<C: Send + Sync + 'static>(&mut self, c: C) -> ComponentHandle<C> {
		let obj = std::sync::Arc::new(std::sync::Mutex::new(c));
		let internal_id = self.systems_data.systems.len() as u32;
		self.systems_data.systems.push(obj);
		let external_id = 0;
		ComponentHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData }
	}

	pub fn make_object_with_handle<C: Send + Sync + 'static>(&mut self, name: &'static str, c: impl Fn(ComponentHandle<C>) -> C) -> ComponentHandle<C> {
		let internal_id = self.systems_data.systems.len() as u32;
		let external_id = 0;
		let handle = ComponentHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData };
		self.systems_data.systems.push(std::sync::Arc::new(std::sync::Mutex::new(c(ComponentHandle::<C> { internal_id, external_id, phantom: std::marker::PhantomData }))));
		self.systems_data.systems_by_name.insert(name, internal_id);
		handle
	}

	pub fn make_handle<S, C>(&self, component_handle: &ComponentHandle<S>, external_id: u32) -> ComponentHandle<C> {
		ComponentHandle::<C> { internal_id: component_handle.internal_id, external_id, phantom: std::marker::PhantomData }
	}

	/// Ties a property of a component to a property of another component.
	pub fn tie<'a, 'b, T: 'static, U: 'b, V: 'static, S0: 'static, S1: 'static>(&mut self, receiver_component_handle: &ComponentHandle<T>, i: fn() -> Property<S0, T, V>, sender_component_handle: &ComponentHandle<U>, j: fn() -> Property<S1, U, V>) {
		let property_function_pointer = j as *const (); // Use the property function pointer as a key to the ties hashmap.

		let property = i();

		let update_function = match property {
			Property::Component { getter, setter } => UpdateFunctionTypes::Component(std::boxed::Box::new(setter)),
			Property::System { getter, setter } => UpdateFunctionTypes::System(std::boxed::Box::new(setter)),
		};

		if self.ties.contains_key(&(property_function_pointer as usize)) {
			let ties = self.ties.get_mut(&(property_function_pointer as usize)).unwrap();
			ties.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
		} else {
			let mut ties = Vec::new();
			ties.push(Tie { update_function, destination_system_handle: receiver_component_handle.internal_id });
			self.ties.insert(property_function_pointer as usize, ties);
		}
	}

	pub fn notify<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &ComponentHandle<C>, function: fn() -> Property<S, C, V>, value: V) {
		let po = function as *const ();
		let ties = self.ties.get(&(po as usize)).unwrap();

		for tie in ties {
			unsafe {
				match tie.update_function {
					UpdateFunctionTypes::Component(ref setter) => {
						let mut component = self.systems_data.systems[tie.destination_system_handle as usize].lock().unwrap();
						let setter = setter.downcast_ref_unchecked::<fn(&mut C, V)>();
						(setter)(component.downcast_mut_unchecked::<C>(), value);
					},
					UpdateFunctionTypes::System(ref setter) => {
						let mut component = self.systems_data.systems[tie.destination_system_handle as usize].lock().unwrap();
						let setter = setter.downcast_ref_unchecked::<fn(&mut S, &ComponentHandle<C>, V)>();
						(setter)(component.downcast_mut_unchecked::<S>(), component_handle, value);
					},
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

	// pub fn execute_task_2<F: Send + 'static, R>(&self, task: F) where F: FnOnce(std::sync::RwLock<Self>) -> R, R: std::future::Future<Output = ()> + Send {
	// 	self.sep.lock().unwrap().1.spawn(async move {
	// 		task(std::sync::RwLock::new(self)).await;
	// 	});
	// }

	pub fn execute_task_sync<F: Fn(&Orchestrator) + 'static>(&mut self, task: F) {
		self.tasks.push(Task { function: std::boxed::Box::new(task) });
	}

	pub fn get_and<C: 'static, F, R>(&self, component_handle: &ComponentHandle<C>, function: F) -> R where F: FnOnce(&C) -> R {
		let component = self.systems_data.systems[component_handle.internal_id as usize].lock().unwrap();
		function(component.downcast_ref::<C>().unwrap())
	}

	pub fn get_mut_and<C: 'static, F, R>(&self, component_handle: &ComponentHandle<C>, function: F) -> R where F: FnOnce(&mut C) -> R {
		let mut component = self.systems_data.systems[component_handle.internal_id as usize].lock().unwrap();
		function(component.downcast_mut::<C>().unwrap())
	}

	pub fn get_2_mut_and<C0: 'static, C1: 'static, F, R>(&self, component_handle_0: &ComponentHandle<C0>, component_handle_1: &ComponentHandle<C1>, function: F) -> R where F: FnOnce(&mut C0, &mut C1) -> R {
		let mut component_0 = self.systems_data.systems[component_handle_0.internal_id as usize].lock().unwrap();
		let mut component_1 = self.systems_data.systems[component_handle_1.internal_id as usize].lock().unwrap();

		function(component_0.downcast_mut::<C0>().unwrap(), component_1.downcast_mut::<C1>().unwrap())
	}

	pub fn get_mut_by_name_and<C: 'static, F, R>(&self, name: &'static str, function: F) -> R where F: FnOnce(&mut C) -> R {
		let mut component = self.systems_data.systems[self.systems_data.systems_by_name[name] as usize].lock().unwrap();
		function(component.downcast_mut::<C>().unwrap())
	}

	pub fn get_property<C: 'static, V: Clone + Copy + 'static, S: 'static>(&self, component_handle: &ComponentHandle<C>, function: fn() -> Property<S, C, V>) -> V {
		let property = function();

		match property {
			Property::Component { getter, setter: _ } => {
				let component = self.systems_data.systems[component_handle.internal_id as usize].lock().unwrap();
				let getter = getter as *const ();
				let getter = unsafe { std::mem::transmute::<*const (), fn(&C) -> V>(getter) };
				(getter)(component.downcast_ref::<C>().unwrap())
			},
			Property::System { getter, setter: _ } => {
				let component = self.systems_data.systems[component_handle.internal_id as usize].lock().unwrap();
				let getter = getter as *const ();
				let getter = unsafe { std::mem::transmute::<*const (), fn(&S, &ComponentHandle<C>) -> V>(getter) };
				(getter)(component.downcast_ref::<S>().unwrap(), component_handle)
			},
		}
	}

	// pub fn get_system<T>(&self, system_handle: SystemHandle) -> Arc<T> where T: System + Send {
	// 	self.systems_data.systems[system_handle.0 as usize].jesus()
	// }
}

#[cfg(test)]
mod tests {
	use super::*;

	struct TestSystem {
		value: u32,
	}

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

		let system_handle = orchestrator.add_system(test_system);

		//orchestrator.execute_task(TestSystem::task);

		orchestrator.update();
	}

	// #[test]
	// fn test_parameters() {
	// 	let mut orchestrator = Orchestrator::new();

	// 	let mut test_system_handle = TestSystem::new();

	// 	let task = || async move {
	// 		assert_eq!(test_system_handle.get_value(), 0);

	// 		test_system_handle.update().await;

	// 		assert_eq!(test_system_handle.get_value(), 1);
	// 	};

	// 	orchestrator.execute_task_1(task);

	// 	orchestrator.update();
	// }

	#[test]
	fn sync_task() {
		let mut orchestrator = Orchestrator::new();

		let test_system = TestSystem::new();

		let system_handle = orchestrator.add_system(test_system);

		fn task(o: &Orchestrator) {
			println!("howdy!");
		}

		orchestrator.execute_task_sync(task);

		orchestrator.update();
	}

	#[test]
	fn tie() {
		let mut orchestrator = Orchestrator::new();

		struct Sender {
			value: u32,
		}

		impl Sender {
			fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Sender> {
				orchestrator.make_object(Sender { value: 0 })
			}

			fn get_value(&self) -> u32 {
				self.value
			}

			fn set_value(&mut self, value: u32) {
				self.value = value;
			}

			const fn send() -> Property<(), Sender, u32> {
				Property::Component { getter: Sender::get_value, setter: Sender::set_value }
			}
		}

		struct Receiver {
			value: u32,
		}

		impl Receiver {
			fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Receiver> {
				orchestrator.make_object(Receiver { value: 0 })
			}

			fn read_value(&self) -> u32 {
				self.value
			}

			fn set_value(&mut self, value: u32) {
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

		orchestrator.notify(&sender_handle, Sender::send, 5);

		let value = orchestrator.get_and(&receiver_handle, |r| r.value);

		assert_eq!(value, 5);
	}

	#[test]
	fn system_owned_components() {
		let mut orchestrator = Orchestrator::new();

		struct Sender {
			value: u32,
		}

		impl Sender {
			fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Sender> {
				orchestrator.make_object(Sender { value: 0 })
			}

			fn get_value(&self) -> u32 { self.value }
			fn set_value(&mut self, value: u32) { self.value = value; }

			const fn send() -> Property<(), Sender, u32> { Property::Component { getter: Sender::get_value, setter: Sender::set_value } }
		}

		struct Component {
			// No data in component as it is managed/owned by the system.	
		}

		struct System {
			handle: ComponentHandle<System>,
			data: Vec<u32>,
		}

		impl System {
			fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<System> {
				orchestrator.make_object_with_handle("System", |handle| {
					System { handle: handle, data: Vec::new() }
				})
			}

			fn create_component(orchestrator: &mut Orchestrator, value: u32) -> ComponentHandle<Component> {
				orchestrator.get_mut_by_name_and("System", |system: &mut System| {
					let external_id = system.data.len() as u32;
					system.data.push(value);
					orchestrator.make_handle(&system.handle, external_id)
				})
			}

			fn set_component_value(&mut self, component_handle: &ComponentHandle<Component>, value: u32) {
				self.data[component_handle.external_id as usize] = value;
			}

			fn get_component_value(&self, component_handle: &ComponentHandle<Component>) -> u32 {
				self.data[component_handle.external_id as usize]
			}
		}

		impl super::System for System {}

		impl Component {
			fn new(orchestrator: &mut Orchestrator) -> ComponentHandle<Component> {
				System::create_component(orchestrator, 0)
			}

			const fn value() -> Property<System, Component, u32> {
				Property::System { getter: System::get_component_value, setter: System::set_component_value }
			}
		}

		let sender_handle = Sender::new(&mut orchestrator);

		System::new(&mut orchestrator);

		let component_handle = Component::new(&mut orchestrator);

		let value = orchestrator.get_property(&component_handle, Component::value);

		assert_eq!(value, 0);

		orchestrator.tie(&component_handle, Component::value, &sender_handle, Sender::send);

		orchestrator.notify(&sender_handle, Sender::send, 5);

		let value = orchestrator.get_property(&component_handle, Component::value);

		assert_eq!(value, 5);
	}
}