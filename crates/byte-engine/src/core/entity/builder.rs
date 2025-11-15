use crate::core::{Entity, domain::DomainEvents, entity::{DomainType, EntityEvents, PostCreationFunction, handle::Handle}, event::Event, listener::{CreateEvent, Listener}};

/// Entity creation functions must return this type.
pub struct Builder<'c, T: 'c> {
	pub(crate) create: Box<dyn FnOnce(DomainType) -> T + 'c>,
	pub(crate) post_creation_functions: Vec<Box<dyn PostCreationFunction<T> + 'c>>,
	pub(crate) events: Vec<EntityEvents<T>>,
}

impl <'c, T: 'c> Builder<'c, T> {
	fn default(create: impl FnOnce(DomainType) -> T + 'c) -> Self {
		Self {
			create: Box::new(create),
			post_creation_functions: Vec::new(),
			events: Vec::new(),
		}
	}

	pub fn new(entity: T) -> Self {
		Self::default(move |_| entity)
	}

	pub fn new_from_function(function: impl FnOnce() -> T + 'c) -> Self {
		Self::default(move |_| function())
	}

	pub fn new_from_closure_with_parent<'a, F>(function: F) -> Self where F: FnOnce(DomainType) -> T + 'c {
		Self::default(move |parent| { function(parent) })
	}

	pub fn then(mut self, function: impl PostCreationFunction<T> + 'c) -> Self {
		self.post_creation_functions.push(Box::new(function));
		self
	}

	pub fn r#as<E>(mut self, cast: fn(Handle<T>) -> Handle<E>) -> Self where E: Entity + ?Sized + 'static, T: Entity + 'static {
		self.events.push(EntityEvents::As { f: Box::new(move |handle, events| {
			events.push(DomainEvents::EntityCreated { f: Box::new(move |executor| {
				executor.broadcast_event(CreateEvent::<E>::new(cast(handle)));
			}) });
		}) });

		self
	}

	pub fn listen_to<E: Event + 'static>(mut self) -> Self where T: Listener<E> + 'static {
		self.events.push(EntityEvents::Listen { f: Box::new(move |handle, events| {
			events.push(DomainEvents::StartListen { f: Box::new(move |executor| {
				executor.add_task_for_event::<E, T>(handle);
			}) });
		}) });

		self
	}
}

impl <'c, T> From<T> for Builder<'c, T> {
	fn from(entity: T) -> Self {
		Self::new(entity)
	}
}
