use core::task;
use std::{ops::Deref, sync::Arc};

use downcast_rs::Downcast;
use utils::{hash::{HashMap, HashMapExt as _}, sync::Mutex};

use super::{entity::EntityBuilder, event::Event, listener::{CreateEvent, Listener}, Entity, EntityHandle};

pub enum Interval {
	Time(std::time::Duration),
	Frames(u32),
}

impl Interval {
	pub fn is_now(&self, elapsed_time: std::time::Duration, dt: std::time::Duration, frame: u64) -> bool {
		match self {
			Interval::Time(duration) => elapsed_time.as_secs_f64() % duration.as_secs_f64() < dt.as_secs_f64(),
			Interval::Frames(frames) => frame % *frames as u64 == 0,
		}
	}
}

impl Into<Interval> for std::time::Duration {
	fn into(self) -> Interval {
		Interval::Time(self)
	}
}

impl Into<Interval> for u32 {
	fn into(self) -> Interval {
		Interval::Frames(self)
	}
}

/// A task is a unit of work that can be executed by the engine.
pub struct Task {
	f: Box<dyn FnMut()>,
	every: Option<Interval>,
	lifetime: Option<Interval>,
	delay: Option<Interval>,
}

impl Task {
	/// Creates a new task.
	pub fn new(f: impl Fn() + 'static) -> Self {
		Task {
			f: Box::new(f),
			every: None,
			lifetime: None,
			delay: None,
		}
	}

	pub fn tick(f: impl Fn() + 'static) -> Self {
		Self::new(f)
	}

	pub fn every(interval: impl Into<Interval>, f: impl Fn() + 'static) -> Self {
		Self { f: Box::new(f), every: Some(interval.into()), lifetime: None, delay: None }
	}

	pub fn once(f: impl Fn() + 'static) -> Self {
		Self { f: Box::new(f), every: None, lifetime: Some(Interval::Frames(1)), delay: None }
	}

	pub fn r#in(interval: impl Into<Interval>, f: impl Fn() + 'static) -> Self {
		Self { f: Box::new(f), every: None, lifetime: None, delay: Some(interval.into()) }
	}

	pub fn method<T: 'static>(handle: EntityHandle<T>, method: impl Fn(&mut T) + 'static) -> Self {
		let f = move || {
			let mut obj = handle.write();
			method(&mut obj);
		};
		Self::new(f)
	}
}

impl Entity for Task {}

/// Running gets delegated to this object because the `TaskExecutor` can be accessed by tasks during their execution which would otherwise lead to a deadlock if the `TaskExecutor` was used directly.
pub struct Execution {
	events: Arc<Mutex<HashMap<std::any::TypeId, Vec<Box<dyn Fn(&dyn Event) + 'static>>>>>,
	runnables: Vec<Runnable>,
}

impl Execution {
	pub fn run(self) {
		for runnable in self.runnables {
			match runnable {
				Runnable::Function(f) => f(),
				Runnable::Task(task) => {
					let mut task = task.write();
					(task.f)();
				}
				Runnable::Event(event) => {
					let type_id = event.type_id();
					if let Some(handlers) = self.events.lock().get(&type_id) {
						for handler in handlers {
							handler(event.as_ref());
						}
					} else {
						eprintln!("No handlers for event type: {:?}", type_id);
					}
				}
			}
		}
	}
}

enum Runnable {
	Function(Box<dyn FnOnce()>),
	Task(EntityHandle<Task>),
	Event(Box<dyn Event>)
}

pub struct TaskExecutor {
	tasks: Vec<EntityHandle<Task>>,
	events: Arc<Mutex<HashMap<std::any::TypeId, Vec<Box<dyn Fn(&dyn Event) + 'static>>>>>,

	to_run: Vec<Runnable>,
}

impl TaskExecutor {
	fn new() -> Self {
		TaskExecutor {
			tasks: Vec::with_capacity(8192),
			events: Arc::new(Mutex::new(HashMap::with_capacity(4096))),

			to_run: Vec::with_capacity(8192),
		}
	}

	pub fn create() -> EntityBuilder<'static, Self> {
		EntityBuilder::new(Self::new()).listen_to::<CreateEvent<Task>>()
	}

	pub fn add_task(&mut self, task: EntityHandle<Task>) {
		self.tasks.push(task);
	}

	pub fn add_task_for_event<E: Event, T: Listener<E> + 'static>(&mut self, callee: EntityHandle<T>) {
		self.events.lock().entry(std::any::TypeId::of::<E>()).or_default().push(Box::new(move |event| {
			let mut callee = callee.write();
			let event = event.downcast_ref::<E>().expect("Event type mismatch");
			callee.handle(event);
		}));
	}

	pub fn broadcast_event<E: Event + 'static>(&mut self, event: E) {
		let type_id = std::any::TypeId::of::<E>();

		self.to_run.push(Runnable::Event(Box::new(event)));
	}

	pub fn get_execution(&mut self, elapsed_time: std::time::Duration, dt: std::time::Duration, frame: u64) -> Execution {
		let to_run = self.to_run.drain(..);

		let to_run = to_run.chain(self.tasks.iter().filter_map(|task| {
			let run = {
				let task = task.read();
				
				let interval = if let Some(interval) = &task.every {
					interval.is_now(elapsed_time, dt, frame)
				} else {
					true
				};
	
				let delay = if let Some(delay) = &task.delay {
					match delay {
						Interval::Time(duration) => {
							duration.as_secs_f64() <= dt.as_secs_f64()
						}
						Interval::Frames(frames) => {
							*frames as u64 == 0
						}
					}
				} else {
					true
				};
	
				interval && delay
			};

			if run {
				Some(Runnable::Task(task.clone()))
			} else {
				None
			}
		}));

		Execution {
			runnables: to_run.collect(),
			events: self.events.clone(),
		}
	}

	pub fn update_tasks(&mut self, elapsed_time: std::time::Duration, dt: std::time::Duration, frame: u64) {
		for task in &mut self.tasks {
			let mut task = task.write();

			if let Some(lifetime) = &mut task.lifetime {
				match lifetime {
					Interval::Time(duration) => {
						*duration -= dt;
					}
					Interval::Frames(frames) => {
						*frames -= 1;
					}
				}
			}

			if let Some(delay) = &mut task.delay {
				match delay {
					Interval::Time(duration) => {
						if *duration >= dt {
							*duration -= dt;
						} else {
							*duration = std::time::Duration::ZERO;
						}
					}
					Interval::Frames(frames) => {
						*frames -= 1;
					}
				}
			}
		}

		self.tasks.retain(|task| {
			let task = task.read();

			let lifetime = if let Some(lifetime) = &task.lifetime {
				match lifetime {
					Interval::Time(duration) => {
						*duration > std::time::Duration::ZERO
					}
					Interval::Frames(frames) => {
						*frames > 0
					}
				}
			} else {
				true
			};

			let delay = if let Some(delay) = &task.delay {
				match delay {
					Interval::Time(duration) => {
						*duration > std::time::Duration::ZERO
					}
					Interval::Frames(frames) => {
						*frames > 0
					}
				}
			} else {
				true
			};

			lifetime && delay
		});
	}
}

impl Entity for TaskExecutor {}

impl Listener<CreateEvent<Task>> for TaskExecutor {
	fn handle(&mut self, event: &CreateEvent<Task>) {
		let handle = event.handle();
		self.add_task(handle.clone());
	}
}