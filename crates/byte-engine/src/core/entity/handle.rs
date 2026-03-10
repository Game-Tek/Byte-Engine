use std::{
	marker::Unsize,
	ops::{CoerceUnsized, Deref},
	sync::Arc,
};

pub type EntityWrapper<T> = Arc<T>;

#[derive(Debug)]
pub struct Handle<T: ?Sized> {
	pub(super) container: EntityWrapper<T>,
}

pub struct WeakHandle<T: ?Sized> {
	pub(super) container: std::sync::Weak<T>,
}

impl<T: ?Sized> WeakHandle<T> {
	pub fn upgrade(&self) -> Option<Handle<T>>
	where
		T: Sized,
	{
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
		T: std::any::Any,
	{
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
		panic!()
	}
}

fn downcast_inner<F: ?Sized, T>(decoder: &EntityWrapper<F>) -> Option<EntityWrapper<T>> {
	let raw: *const F = std::sync::Arc::into_raw(decoder.clone());
	let raw: *const T = raw.cast();

	// SAFETY: This is safe because the pointer orignally came from an Arc
	// with the same size and alignment since we've checked (via Any) that
	// the object within is the type being casted to.
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
		if let Some(e) = Arc::get_mut(&mut self.container) {
			Some(function(e))
		} else {
			None
		}
	}
}

impl<T: ?Sized> Deref for Handle<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&*self.container
	}
}
