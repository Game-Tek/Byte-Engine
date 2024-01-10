pub trait Entity: intertrait::CastFrom + downcast_rs::Downcast + std::any::Any + 'static {}

downcast_rs::impl_downcast!(Entity);

pub(super) type EntityWrapper<T> = std::sync::Arc<smol::lock::RwLock<T>>;

#[derive(Debug,)]
pub struct EntityHandle<T: Entity + ?Sized> {
	pub(super) container: EntityWrapper<T>,
	pub(super) internal_id: u32,
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

impl <T: Entity + ?Sized> PartialEq for EntityHandle<T> {
	fn eq(&self, other: &Self) -> bool {
		self.internal_id == other.internal_id
	}

	fn ne(&self, other: &Self) -> bool {
		self.internal_id != other.internal_id
	}
}

fn downcast_inner<U: Entity>(decoder: &EntityWrapper<dyn Entity>) -> Option<EntityWrapper<U>> {
	let raw: *const smol::lock::RwLock<dyn Entity> = std::sync::Arc::into_raw(decoder.clone());
	let raw: *const smol::lock::RwLock<U> = raw.cast();
	
	// SAFETY: This is safe because the pointer orignally came from an Arc
	// with the same size and alignment since we've checked (via Any) that
	// the object within is the type being casted to.
	Some(unsafe { std::sync::Arc::from_raw(raw) })
}

impl EntityHandle<dyn Entity> {
	pub fn downcast<U: Entity>(&self) -> Option<EntityHandle<U>> {
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

use std::marker::Unsize;
use std::ops::CoerceUnsized;

impl<T: Entity, U: Entity> CoerceUnsized<EntityHandle<U>> for EntityHandle<T>
where
    T: Unsize<U> + ?Sized,
    U: ?Sized {}

impl <T: Entity> EntityHandle<T> {
	// pub fn sync_get<'a, R>(&self, function: impl FnOnce(&'a T) -> R) -> R {
	// 	let lock = self.container.read_arc_blocking();
	// 	function(lock.deref())
	// }

	// pub fn sync_get_mut<'a, R>(&self, function: impl FnOnce(&'a mut T) -> R) -> R {
	// 	let mut lock = self.container.write_arc_blocking();
	// 	function(lock.deref_mut())
	// }

	pub fn get_lock<'a>(&self) -> EntityWrapper<T> {
		self.container.clone()
	}

	pub fn read(&self) -> smol::lock::futures::ReadArc<'_, T> {
		self.container.read_arc()
	}

	pub fn read_sync<'a>(&self) -> smol::lock::RwLockReadGuardArc<T> {
		self.container.read_arc_blocking()
	}

	pub fn write(&self) -> smol::lock::futures::WriteArc<'_, T> {
		self.container.write_arc()
	}

	pub fn write_sync<'a>(&self) -> smol::lock::RwLockWriteGuardArc<T> {
		self.container.write_arc_blocking()
	}

	pub fn map<'a, R>(&self, function: impl FnOnce(&Self) -> R) -> R {
		function(self)
	}
}