//! The orchestrator synchronizes and manages most of the application data.
//! It contains systems and task to accomplish that feat.

// pub struct Arc<T: ?Sized> {
// 	inner: *mut u8,
// 	p: std::marker::PhantomData<T>,
// }

// impl <T: ?Sized> Arc<T> {
// 	fn new<U: Sized>(inner: U) -> Arc<T> {
// 		unsafe {
// 			let u = std::boxed::Box::new(inner);
// 			Arc { 
// 				inner: std::boxed::Box::into_raw(u) as *mut u8,
// 				p: std::marker::PhantomData,
// 			}
// 		}
// 	}

// 	fn jesus<U>(&self) -> Arc<U> {
// 		unsafe {
// 			Arc {
// 				inner: self.inner,
// 				p: std::marker::PhantomData,
// 			}
// 		}
// 	}
// }

// impl <T> std::ops::Deref for Arc<T> {
// 	type Target = T;

// 	fn deref(&self) -> &Self::Target {
// 		unsafe { &*(self.inner as *const T) }
// 	}
// }

// TODO: implement Drop

/// System handle is a handle to a system in an [`Orchestrator`]
pub struct SystemHandle(u32);

/// A system is a collection of components and logic to operate on them.
pub trait System {
	/// Casts a system to a [`std::any::Any`] reference.
	fn as_any(&self) -> &dyn std::any::Any;
}

/// An orchestrator is a collection of systems that are updated in parallel.
pub struct Orchestrator {
	sep: std::sync::Mutex<(crate::executor::Executor, crate::executor::Spawner)>,
	systems_data: SystemsData,
	tasks: Vec<Task>,
}

unsafe impl Send for Orchestrator {}

struct SystemsData {
	systems: Vec<std::sync::Arc<std::sync::Mutex<dyn std::any::Any + Send + Sync + 'static>>>,
}

trait SystemLock<T> {
	fn pls(&self) -> std::sync::Mutex<&T>;
}

struct Task {
	function: std::boxed::Box<dyn Fn(&Orchestrator)>,
}

type OrchestratorHandle = std::sync::Arc<Orchestrator>;

impl Orchestrator {
	pub fn new() -> Orchestrator {
		Orchestrator {
			sep: std::sync::Mutex::new(crate::executor::new_executor_and_spawner()),
			systems_data: SystemsData { systems: Vec::new() },
			tasks: Vec::new(),
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

	pub fn add_system<T: System + Sync + Send + 'static>(&mut self, system: T) -> SystemHandle {
		let system_handle = SystemHandle(self.systems_data.systems.len() as u32);
		self.systems_data.systems.push(std::sync::Arc::new(std::sync::Mutex::new(system)));
		system_handle
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

	impl System for TestSystem {
		fn as_any(&self) -> &dyn std::any::Any {
			self
		}
	}

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
}