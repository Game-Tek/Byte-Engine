//! Strong and weak handles for shared engine entities.
//!
//! Use [`Handle`] when a subsystem must keep an entity alive and [`WeakHandle`]
//! for caches or relationships that must not extend its lifetime.

use std::{
	any::{Any, TypeId},
	marker::Unsize,
	ops::{CoerceUnsized, Deref},
	sync::Arc,
};

pub type EntityWrapper<T> = Arc<T>;

#[derive(Debug)]
/// The [`Handle`] struct provides shared ownership of an entity across engine
/// systems.
pub struct Handle<T: ?Sized> {
	pub(super) container: EntityWrapper<T>,
}

/// The [`WeakHandle`] struct references an entity without extending its lifetime.
pub struct WeakHandle<T: ?Sized> {
	pub(super) container: std::sync::Weak<T>,
}

impl<T: ?Sized> WeakHandle<T> {
	pub fn upgrade(&self) -> Option<Handle<T>> {
		self.container.upgrade().map(|c| Handle { container: c })
	}
}

impl<T: ?Sized> From<Handle<T>> for WeakHandle<T> {
	fn from(handle: Handle<T>) -> Self {
		Self {
			container: std::sync::Arc::downgrade(&handle.container),
		}
	}
}

impl<T: ?Sized> Handle<T> {
	pub fn new(object: EntityWrapper<T>) -> Self {
		Self { container: object }
	}

	pub fn downcast<U>(&self) -> Option<Handle<U>>
	where
		T: Any,
		U: Any,
	{
		if self.container.as_ref().type_id() != TypeId::of::<U>() {
			return None;
		}

		let down = downcast_inner::<T, U>(&self.container);
		Some(Handle { container: down? })
	}

	pub fn weak(&self) -> WeakHandle<T> {
		WeakHandle {
			container: std::sync::Arc::downgrade(&self.container),
		}
	}
}

impl<T: Sized> From<T> for Handle<T> {
	fn from(value: T) -> Self {
		Self {
			container: EntityWrapper::new(value),
		}
	}
}

impl<T: ?Sized> PartialEq for Handle<T> {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.container, &other.container)
	}
}

impl<T: ?Sized> Eq for Handle<T> {}

fn downcast_inner<F: ?Sized, T>(decoder: &EntityWrapper<F>) -> Option<EntityWrapper<T>> {
	let raw: *const F = std::sync::Arc::into_raw(decoder.clone());
	let raw: *const T = raw.cast();

	// SAFETY: This is safe because the pointer orignally came from an Arc
	// with the same size and alignment since we've checked (via Any) that
	// the object within is the type being casted to.
	#[allow(unsafe_code)]
	Some(unsafe { std::sync::Arc::from_raw(raw) })
}

impl<T: ?Sized> Clone for Handle<T> {
	fn clone(&self) -> Self {
		Self {
			container: self.container.clone(),
		}
	}
}

impl<T, U> CoerceUnsized<Handle<U>> for Handle<T>
where
	T: Unsize<U> + ?Sized,
	U: ?Sized,
{
}

impl<T: ?Sized> Handle<T> {
	pub fn get_lock<'a>(&self) -> EntityWrapper<T> {
		self.container.clone()
	}

	pub fn try_map_mut<'a, R>(&mut self, function: impl FnOnce(&mut T) -> R) -> Option<R> {
		Arc::get_mut(&mut self.container).map(function)
	}
}

impl<T: ?Sized> Deref for Handle<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.container
	}
}

#[cfg(test)]
mod tests {
	use std::any::Any;

	use super::Handle;

	#[test]
	fn equality_tracks_entity_identity_not_value_equality() {
		let first = Handle::from(String::from("entity"));
		let clone = first.clone();
		let separate = Handle::from(String::from("entity"));

		assert_eq!(first, clone);
		assert_ne!(first, separate);
	}

	#[test]
	fn downcast_accepts_the_concrete_type_and_rejects_other_types() {
		let concrete = Handle::from(String::from("mesh"));
		let erased: Handle<dyn Any> = concrete.clone();

		let restored = erased.downcast::<String>().expect("matching concrete type");
		assert_eq!(restored.as_str(), "mesh");
		assert_eq!(restored, concrete);
		assert!(erased.downcast::<u64>().is_none());
	}

	#[test]
	fn weak_handles_do_not_extend_lifetime() {
		let weak = {
			let strong = Handle::from(42u32);
			let weak = strong.weak();
			assert_eq!(*weak.upgrade().expect("strong handle is alive"), 42);
			weak
		};

		assert!(weak.upgrade().is_none());
	}

	#[test]
	fn mutable_access_requires_unique_ownership() {
		let mut handle = Handle::from(vec![1, 2]);
		assert_eq!(handle.try_map_mut(|values| values.push(3)), Some(()));
		assert_eq!(&*handle, &[1, 2, 3]);

		let clone = handle.clone();
		assert_eq!(handle.try_map_mut(|values| values.clear()), None);
		assert_eq!(&*clone, &[1, 2, 3]);
	}
}
