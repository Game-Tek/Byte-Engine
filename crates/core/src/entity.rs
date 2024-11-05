/// The Entity trait is the base trait for all entities in the engine.
///
/// An entity is a type that can be spawned and managed by the engine.
/// The trait provides some convenience methods to interact with the entity.
pub trait Entity: downcast_rs::Downcast + std::any::Any + 'static {
	/// Exposes an optional feature of the entity, which is the listener.
	/// This is used to allow entities to listen to other entities.
	fn get_listener(&self) -> Option<&BasicListener> {
		None
	}

	fn get_entity_trait(&self) -> EntityTrait {
		EntityTrait {
			trait_id: std::any::TypeId::of::<Self>(),
		}
	}

	fn get_traits(&self) -> Vec<EntityTrait> { vec![] }

	fn call_listeners<'a>(&'a self, listener: &'a BasicListener, handle: EntityHandle<Self>) -> BoxedFuture<'a, ()> where Self: Sized { listener.invoke_for(handle, self) }
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

pub struct TraitObject {
	pub data: *mut (),
	pub vtable: *mut (),
}

/// The SpawnerEntity trait is a trait that allows an entity to spawn other entities.
pub trait SpawnerEntity<P: Entity>: Entity {
	fn get_parent(&self) -> EntityHandle<P>;

	fn spawn<T: Entity>(&self, entity: impl SpawnHandler<T>) -> impl Future<Output = EntityHandle<T>> where Self: Sized {
		crate::spawn_as_child(self.get_parent(), entity)
	}
}

pub trait SelfDestroyingEntity: Entity {
	fn destroy(&self);
}

downcast_rs::impl_downcast!(Entity);

pub type EntityWrapper<T> = std::sync::Arc<RwLock<T>>;

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

use std::future::Future;
use std::{marker::Unsize, ops::Deref};
use std::ops::CoerceUnsized;

use utils::r#async::{RwLock, RwLockReadGuard, RwLockWriteGuard};
use utils::BoxedFuture;

use super::{listener::{BasicListener, EntitySubscriber, Listener}, SpawnHandler};

impl<T, U> CoerceUnsized<EntityHandle<U>> for EntityHandle<T>
where
    T: Unsize<U> + ?Sized,
    U: ?Sized {}

impl <T: ?Sized> EntityHandle<T> {
	// pub fn sync_get<'a, R>(&self, function: impl FnOnce(&'a T) -> R) -> R {
	// 	let lock = self.container.read_arc_blocking();
	// 	function(lock.deref())
	// }

	pub fn sync_get_mut<R>(&self, function: impl FnOnce(&mut T) -> R) -> R {
		let mut lock = self.container.blocking_write();
		function(std::ops::DerefMut::deref_mut(&mut lock))
	}

	pub fn get_lock<'a>(&self) -> EntityWrapper<T> {
		self.container.clone()
	}

	pub fn read(&self) -> impl Future<Output = RwLockReadGuard<'_, T>> where T: Sized {
		self.container.read()
	}

	pub fn read_sync<'a>(&self) -> RwLockReadGuard<T> where T: Sized {
		utils::r#async::spawn_blocking_local(|| {
			self.container.blocking_read()
		})
	}

	pub fn write(&self) -> impl Future<Output = RwLockWriteGuard<'_, T>> {
		self.container.write()
	}

	pub fn write_sync<'a>(&self) -> RwLockWriteGuard<T> {
		utils::r#async::spawn_blocking_local(|| {
			self.container.blocking_write()
		})
	}

	pub fn map<'a, R>(&self, function: impl FnOnce(&Self) -> R) -> R {
		function(self)
	}
}

// pub type DomainType<'a> = &'a dyn Entity;
pub type DomainType = EntityHandle<dyn Entity>;

/// Entity creation functions must return this type.
pub struct EntityBuilder<'c, T: 'c> {
	pub create: Box<dyn FnOnce(Option<DomainType>) -> BoxedFuture<'c, T> + 'c>,
	pub post_creation_functions: Vec<std::boxed::Box<dyn Fn(&mut EntityHandle<T>,) + 'c>>,
	pub listens_to: Vec<Box<dyn Fn(DomainType, EntityHandle<T>) + 'c>>,
}

impl <'c, T: Entity + 'c> EntityBuilder<'c, T> {
	fn default(create: impl FnOnce(Option<DomainType>) -> BoxedFuture<'c, T> + 'c) -> Self {
		Self {
			create: Box::new(create),
			post_creation_functions: Vec::new(),
			listens_to: Vec::new(),
		}
	}

	pub fn new(entity: T) -> Self {
		Self::default(move |_| Box::pin(async move { entity }))
	}

	pub fn new_from_function(function: impl FnOnce() -> T + 'c) -> Self {
		Self::default(move |_| Box::pin(async move { function() }))
	}

	pub fn new_from_async_function<R>(function: impl FnOnce() -> R + 'c) -> Self where R: Future<Output = T> + 'c {
		Self::default(move |_| Box::pin(function()))
	}

	pub fn new_from_async_function_with_parent<R>(function: impl FnOnce(DomainType) -> R + 'c) -> Self where R: Future<Output = T> + 'c {
		Self::default(move |parent| Box::pin(function(parent.unwrap())))
	}

	pub fn new_from_closure<'a, F>(function: F) -> Self where F: FnOnce() -> T + 'c {
		Self::default(move |_| { Box::pin(async move { function() }) })
	}

	pub fn new_from_closure_with_parent<'a, F>(function: F) -> Self where F: FnOnce(DomainType) -> T + 'c {
		Self::default(move |parent| { Box::pin(async move { function(parent.unwrap()) }) })
	}

	pub fn then(mut self, function: impl Fn(&mut EntityHandle<T>,) + 'c) -> Self {
		self.post_creation_functions.push(Box::new(function));
		self
	}

	pub fn listen_to<C: Entity + ?Sized + 'static>(mut self,) -> Self where T: EntitySubscriber<C> {
		self.listens_to.push(Box::new(move |domain_handle, e| {
			let l = domain_handle.write_sync();
			let l = l.deref();

			if let Some(l) = l.get_listener() {
				l.add_listener(e);
			} else {
				// log::error!("Entity listens to but wasn't spawned in a domain.");
			}
		}));
		self
	}
}

impl <T: Entity> From<T> for EntityBuilder<'static, T> {
	fn from(entity: T) -> Self {
		Self::new(entity)
	}
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use utils::r#async::block_on;

	use super::*;
	use crate::spawn;

	#[test]
	fn spawn_entities() {
		struct Component {
			name: String,
			value: u32,
		}

		impl Entity for Component {}

		let _: EntityHandle<Component> = block_on(spawn(Component { name: "test".to_string(), value: 1 }));

		struct System {

		}

		impl Entity for System {}

		impl System {
			fn new<'c>() -> EntityBuilder<'c, System> {
				EntityBuilder::new(System {})
			}
		}

		impl EntitySubscriber<Component> for System {
			fn on_create<'a>(&'a mut self, _: EntityHandle<Component>, component: &Component) -> utils::BoxedFuture<'a, ()> where Self: Sized + Send + Sync {
				println!("Component created: {} {}", component.name, component.value);
				Box::pin(async move {})
			}
		}

		let _: EntityHandle<System> = block_on(spawn(System::new()));

		let _: EntityHandle<Component> = block_on(spawn(Component { name: "test".to_string(), value: 1 }));
	}
}
