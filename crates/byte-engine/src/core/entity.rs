use utils::sync::{RwLock, Arc, RwLockReadGuard, RwLockWriteGuard};

use crate::core::spawn_as_child;
use crate::gameplay::space::{Space, Spawner};

/// The Entity trait is the base trait for all entities in the engine.
///
/// An entity is a type that can be spawned and managed by the engine.
/// The trait provides some convenience methods to interact with the entity.
pub trait Entity: downcast_rs::Downcast + std::any::Any + 'static {
	fn get_entity_trait(&self) -> EntityTrait {
		EntityTrait {
			trait_id: std::any::TypeId::of::<Self>(),
		}
	}

	fn get_traits(&self) -> Vec<EntityTrait> { vec![] }

	fn builder(self) -> EntityBuilder<'static, Self> where Self: Sized {
		EntityBuilder::new(self)
	}
}

pub unsafe fn get_entity_trait_for_type<T: ?Sized + 'static>() -> EntityTrait {
	EntityTrait {
		trait_id: std::any::TypeId::of::<T>(),
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EntityTrait {
	trait_id: std::any::TypeId,
}

pub trait SelfDestroyingEntity: Entity {
	fn destroy(&self);
}

downcast_rs::impl_downcast!(Entity);

pub type EntityWrapper<T> = Arc<RwLock<T>>;

#[derive(Debug,)]
pub struct EntityHandle<T: ?Sized> {
	pub(super) container: EntityWrapper<T>,
	pub(super) internal_id: u32,
}

pub struct WeakEntityHandle<T: ?Sized> {
	pub(super) container: std::sync::Weak<RwLock<T>>,
	// pub(super) internal_id: u32,
}

impl <T: ?Sized> WeakEntityHandle<T> {
	pub fn read_sync(&self) -> Option<RwLockReadGuard<T>> where T: Sized {
		// self.container.upgrade().map(|c| c.blocking_read())
		None
	}

	pub fn write_sync(&self) -> Option<RwLockWriteGuard<T>> {
		// self.container.upgrade().map(|c| c.blocking_write())
		None
	}
}

impl <T: ?Sized> From<EntityHandle<T>> for WeakEntityHandle<T> {
	fn from(handle: EntityHandle<T>) -> Self {
		Self {
			container: std::sync::Arc::downgrade(&handle.container),
			// internal_id: handle.internal_id,
		}
	}
}

pub type EntityHash = u32;

impl <T: ?Sized> From<&EntityHandle<T>> for EntityHash {
	fn from(handle: &EntityHandle<T>) -> Self {
		handle.internal_id
	}
}

impl <T: ?Sized> EntityHandle<T> {
	pub fn new(object: EntityWrapper<T>, internal_id: u32,) -> Self {
		Self {
			container: object,
			internal_id,
		}
	}

	pub fn downcast<U>(&self) -> Option<EntityHandle<U>> where T: std::any::Any {
		let down = downcast_inner::<T, U>(&self.container);
		Some(EntityHandle {
			container: down?,
			internal_id: self.internal_id,
		})
	}

	pub fn weak(&self) -> WeakEntityHandle<T> {
		WeakEntityHandle {
			container: std::sync::Arc::downgrade(&self.container),
			// internal_id: self.internal_id,
		}
	}
}

impl <T: ?Sized> PartialEq for EntityHandle<T> {
	fn eq(&self, other: &Self) -> bool {
		self.internal_id == other.internal_id
	}

	fn ne(&self, other: &Self) -> bool {
		self.internal_id != other.internal_id
	}
}

impl <T: ?Sized> Eq for EntityHandle<T> {}

impl <T: ?Sized> std::hash::Hash for EntityHandle<T> {
	fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
		self.internal_id.hash(state);
	}
}

fn downcast_inner<F: ?Sized, T>(decoder: &EntityWrapper<F>) -> Option<EntityWrapper<T>> {
	let raw: *const RwLock<F> = std::sync::Arc::into_raw(decoder.clone());
	let raw: *const RwLock<T> = raw.cast();

	// SAFETY: This is safe because the pointer orignally came from an Arc
	// with the same size and alignment since we've checked (via Any) that
	// the object within is the type being casted to.
	Some(unsafe { std::sync::Arc::from_raw(raw) })
}

impl <T: ?Sized> Clone for EntityHandle<T> {
	fn clone(&self) -> Self {
		Self {
			container: self.container.clone(),
			internal_id: self.internal_id,
		}
	}
}

use std::{marker::Unsize, ops::Deref};
use std::ops::CoerceUnsized;

use super::domain::Domain;
use super::event::Event;
use super::{listener::Listener, SpawnHandler};

impl<T, U> CoerceUnsized<EntityHandle<U>> for EntityHandle<T>
where
    T: Unsize<U> + ?Sized,
    U: ?Sized {}

impl <T: ?Sized> EntityHandle<T> {
	pub fn get_mut<R>(&self, function: impl FnOnce(&mut T) -> R) -> R {
		let mut lock = self.container.write();
		function(std::ops::DerefMut::deref_mut(&mut lock))
	}

	pub fn get_lock<'a>(&self) -> EntityWrapper<T> {
		self.container.clone()
	}

	pub fn read(&self) -> RwLockReadGuard<'_, T> {
		self.container.read()
	}

	pub fn write(&self) -> RwLockWriteGuard<'_, T> {
		self.container.write()
	}

	pub fn try_read(&self) -> Option<RwLockReadGuard<'_, T>> {
		self.container.try_read()
	}

	pub fn map<'a, R>(&self, function: impl FnOnce(&Self) -> R) -> R {
		function(self)
	}
}

pub type DomainType = EntityHandle<dyn Domain>;

pub trait PostCreationFunction<T> = FnOnce(DomainType, EntityHandle<T>);

/// Entity creation functions must return this type.
pub struct EntityBuilder<'c, T: 'c> {
	pub create: Box<dyn FnOnce(DomainType) -> T + 'c>,
	pub post_creation_functions: Vec<std::boxed::Box<dyn PostCreationFunction<T> + 'c>>,
	pub listens_to: Vec<Box<dyn Fn(DomainType, EntityHandle<T>) + 'c>>,
}

impl <'c, T: 'c> EntityBuilder<'c, T> {
	fn default(create: impl FnOnce(DomainType) -> T + 'c) -> Self {
		Self {
			create: Box::new(create),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
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

	pub fn r#as<E: Entity + ?Sized>(self) -> Self {
		todo!("Implement Entity trait for EntityBuilder");
		self
	}

	pub fn listen_to<C: Event>(mut self) -> Self where T: Listener<C> + 'static {
		self.listens_to.push(Box::new(move |domain, e| {
			let domain = domain.read();

			todo!("Implement Entity trait for EntityBuilder");
		}));

		self
	}
}

impl <'c, T> From<T> for EntityBuilder<'c, T> {
	fn from(entity: T) -> Self {
		Self::new(entity)
	}
}

pub trait MapAndCollectAsAvailable<T: ?Sized, U> {
	/// Maps the entities in the vector and collects them into a new vector but skips taken locks until they are available.
	/// This avoids stalling the thread if a lock is taken.
	/// Order of the elements is **not** preserved.
	fn map_and_collect_as_available(&self, function: impl FnMut(&T) -> U) -> Vec<U>;
}

impl <T: ?Sized, U> MapAndCollectAsAvailable<T, U> for Vec<EntityHandle<T>> {
	fn map_and_collect_as_available(&self, mut function: impl FnMut(&T) -> U) -> Vec<U> {
		let mut source = (0..self.len()).collect::<Vec<_>>();
		let mut res = Vec::with_capacity(self.len());

		while !source.is_empty() {
			source.retain(|i| {
				let e = &self[*i];

				if let Some(b) = e.try_read() {
					res.push(function(&b));
					false
				} else {
					true
				}
			});
		}
	
		res
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use super::*;
	use crate::core::{listener::CreateEvent, spawn};

	#[test]
	fn spawn_entities() {
		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> System {
				System {}
			}
		}

		impl Listener<CreateEvent<Component>> for System {
			fn handle(&mut self, event: &CreateEvent<Component>) {
				println!("Component created");
			}
		}

		let _: EntityHandle<System> = spawn(System::new());

		let _: EntityHandle<Component> = spawn(Component { name: "test".to_string(), value: 1 });
	}
}
