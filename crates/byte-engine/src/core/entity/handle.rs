use std::{marker::Unsize, ops::CoerceUnsized, sync::Arc};

use utils::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

pub type EntityWrapper<T> = Arc<RwLock<T>>;

#[derive(Debug,)]
pub struct Handle<T: ?Sized> {
	pub(super) container: EntityWrapper<T>,
	pub(super) internal_id: u32,
}

pub struct WeakHandle<T: ?Sized> {
	pub(super) container: std::sync::Weak<RwLock<T>>,
	pub(super) internal_id: u32,
}

impl <T: ?Sized> WeakHandle<T> {
	pub fn upgrade(&self) -> Option<Handle<T>> where T: Sized {
		self.container.upgrade().map(|c| Handle {
			container: c,
			internal_id: self.internal_id,
		})
	}
}

impl <T: ?Sized> From<Handle<T>> for WeakHandle<T> {
	fn from(handle: Handle<T>) -> Self {
		Self {
			container: std::sync::Arc::downgrade(&handle.container),
			internal_id: handle.internal_id,
		}
	}
}

pub type EntityHash = u32;

impl <T: ?Sized> From<&Handle<T>> for EntityHash {
	fn from(handle: &Handle<T>) -> Self {
		handle.internal_id
	}
}

impl <T: ?Sized> Handle<T> {
	pub fn new(object: EntityWrapper<T>, internal_id: u32,) -> Self {
		Self {
			container: object,
			internal_id,
		}
	}

	pub fn downcast<U>(&self) -> Option<Handle<U>> where T: std::any::Any {
		let down = downcast_inner::<T, U>(&self.container);
		Some(Handle {
			container: down?,
			internal_id: self.internal_id,
		})
	}

	pub fn weak(&self) -> WeakHandle<T> {
		WeakHandle {
			container: std::sync::Arc::downgrade(&self.container),
			internal_id: self.internal_id,
		}
	}
}

impl <T: ?Sized> PartialEq for Handle<T> {
	fn eq(&self, other: &Self) -> bool {
		self.internal_id == other.internal_id
	}

	fn ne(&self, other: &Self) -> bool {
		self.internal_id != other.internal_id
	}
}

impl <T: ?Sized> Eq for Handle<T> {}

impl <T: ?Sized> std::hash::Hash for Handle<T> {
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

impl <T: ?Sized> Clone for Handle<T> {
	fn clone(&self) -> Self {
		Self {
			container: self.container.clone(),
			internal_id: self.internal_id,
		}
	}
}

impl<T, U> CoerceUnsized<Handle<U>> for Handle<T>
where
	T: Unsize<U> + ?Sized,
	U: ?Sized {}

impl <T: ?Sized> Handle<T> {
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
