//! Shared ownership handles for engine entities.
//!
//! Use [`Handle`] when a subsystem must keep an entity alive and compare entities by identity.

use std::{
	marker::Unsize,
	ops::{CoerceUnsized, Deref},
	sync::Arc,
};

#[derive(Debug)]
/// The [`Handle`] struct provides shared ownership of an entity across engine
/// systems.
pub struct Handle<T: ?Sized> {
	pub(super) container: Arc<T>,
}

impl<T: Sized> From<T> for Handle<T> {
	fn from(value: T) -> Self {
		Self {
			container: Arc::new(value),
		}
	}
}

impl<T: ?Sized> PartialEq for Handle<T> {
	fn eq(&self, other: &Self) -> bool {
		Arc::ptr_eq(&self.container, &other.container)
	}
}

impl<T: ?Sized> Eq for Handle<T> {}

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

impl<T: ?Sized> Deref for Handle<T> {
	type Target = T;

	fn deref(&self) -> &Self::Target {
		&self.container
	}
}
