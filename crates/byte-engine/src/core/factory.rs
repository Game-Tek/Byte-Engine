//! Creation messages with stable handles.
//!
//! A [`Factory`] is the standard boundary between code that creates an object
//! and systems that mirror it. World factories use this pattern to notify
//! rendering and physics without giving those systems ownership of gameplay
//! objects.

/// The `Factory` struct creates values with stable handles for subscribed systems.
///
/// Register each consuming system with [`Self::listener`] before calling
/// [`Self::create`]. Use [`Self::derive`] when another representation must keep
/// the same logical handle.
#[derive(Clone)]
pub struct Factory<T: Clone + ?Sized> {
	channel: DefaultChannel<CreateMessage<T>>,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The `Creator` trait provides fluent creation through an owning API boundary.
///
/// Use [`Self::create`] for the first representation of an entity, then chain
/// [`Creation::with`] for each additional representation that must share its handle.
pub trait Creator<T> {
	/// Creates a value in the owner's matching factory and starts a shared-handle creation chain.
	fn create(&mut self, value: T) -> Creation<'_, Self>
	where
		Self: Sized,
	{
		let handle = self.publish(None, value);
		Creation { creator: self, handle }
	}

	/// Publishes a value with a new handle or the supplied shared handle.
	#[doc(hidden)]
	fn publish(&mut self, handle: Option<Handle>, value: T) -> Handle;
}

/// The `Creation` struct keeps one stable handle while an owner creates multiple entity representations.
///
/// Chain [`Self::with`] to publish another representation, then convert the
/// result into [`Handle`] when another API needs the entity identity.
pub struct Creation<'creator, C: ?Sized> {
	creator: &'creator mut C,
	handle: Handle,
}

impl<C: ?Sized> Creation<'_, C> {
	/// Publishes another representation through the same owner under this creation's handle.
	pub fn with<T>(self, value: T) -> Self
	where
		C: Creator<T>,
	{
		let published_handle = self.creator.publish(Some(self.handle), value);
		debug_assert_eq!(published_handle, self.handle);
		self
	}

	/// Returns the stable handle shared by every representation in this chain.
	pub fn handle(&self) -> Handle {
		self.handle
	}
}

impl<C: ?Sized> From<Creation<'_, C>> for Handle {
	fn from(creation: Creation<'_, C>) -> Self {
		creation.handle
	}
}

impl<T: Clone> Default for Factory<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Clone> Factory<T> {
	/// Creates an empty creation stream.
	///
	/// Next, call [`Self::listener`] for each system that mirrors created values,
	/// then publish values through [`Self::create`].
	pub fn new() -> Self {
		Factory {
			channel: DefaultChannel::new(),
		}
	}

	/// Publishes a value with a new stable handle.
	///
	/// Consumers read the resulting [`CreateMessage`] from listeners created by
	/// [`Self::listener`]. Pass the returned handle to [`Self::derive`] when a
	/// second factory publishes another representation of the same entity.
	pub fn create(&mut self, data: T) -> Handle {
		let handle = Handle::new();
		let message = CreateMessage::new(handle, data);

		self.channel.send(message);

		handle
	}

	/// Creates multiple entities in a single statically-sized batch.
	///
	/// Returns an array of [`Handle`]s corresponding to the created entities.
	/// May be more efficient than calling [`Self::create`] multiple times.
	pub fn create_array<const N: usize>(&mut self, data: [T; N]) -> [Handle; N] {
		let mut handles = [Handle(0); N];
		for (i, d) in data.into_iter().enumerate() {
			handles[i] = self.create(d);
		}
		handles
	}

	/// Publishes a value with an existing stable handle.
	///
	/// Use this after [`Self::create`] when another system-specific representation
	/// must retain the original entity identity.
	pub fn derive(&self, handle: Handle, data: T) {
		let message = CreateMessage::new(handle, data);

		self.channel.send(message);
	}

	/// Creates a consumer for current and future creation messages.
	///
	/// Next, call [`Self::create`] or [`Self::derive`] and drain the messages
	/// through [`crate::core::listener::Listener::read`].
	pub fn listener(&self) -> DefaultListener<CreateMessage<T>> {
		self.channel.listener()
	}
}

#[derive(Debug, Clone)]
/// The [`CreateMessage`] struct carries a created value and the stable handle
/// shared by systems that mirror it.
pub struct CreateMessage<T: Clone> {
	handle: Handle,
	data: T,
}

impl<T: Clone> CreateMessage<T> {
	fn new(handle: Handle, data: T) -> Self {
		CreateMessage { handle, data }
	}

	pub fn data(&self) -> &T {
		&self.data
	}

	pub fn into_data(self) -> T {
		self.data
	}

	pub fn handle(&self) -> &Handle {
		&self.handle
	}
}

impl<T: Clone> Message for CreateMessage<T> {}

impl<T: Clone> TargetedMessage for CreateMessage<T> {
	type Payload = T;

	fn from_handle_and_payload(handle: Handle, data: Self::Payload) -> Self {
		CreateMessage::new(handle, data)
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
/// The [`Handle`] struct identifies one creation stream entry across consuming
/// systems.
pub struct Handle(u32);

impl Handle {
	/// Allocates an identity shared by factory creations and message-backed components.
	pub(crate) fn new() -> Self {
		Self(COUNTER.fetch_add(1, Ordering::Relaxed))
	}
}

#[cfg(test)]
mod tests {
	use super::{Creator, Factory, Handle};
	use crate::core::listener::Listener;

	#[test]
	fn create_assigns_distinct_handles_and_broadcasts_in_creation_order() {
		let mut factory = Factory::new();
		let mut listener = factory.listener();

		let first = factory.create("first");
		let second = factory.create("second");
		let messages = listener.to_vec();

		assert_ne!(first, second);
		assert_eq!(messages.len(), 2);
		assert_eq!(messages[0].handle(), &first);
		assert_eq!(messages[0].data(), &"first");
		assert_eq!(messages[1].handle(), &second);
		assert_eq!(messages[1].data(), &"second");
	}

	#[test]
	fn creator_chains_different_values_under_one_handle() {
		struct Owner {
			labels: Factory<String>,
			indices: Factory<u32>,
		}

		impl Creator<String> for Owner {
			fn publish(&mut self, handle: Option<Handle>, value: String) -> Handle {
				if let Some(handle) = handle {
					self.labels.derive(handle, value);
					handle
				} else {
					self.labels.create(value)
				}
			}
		}

		impl Creator<u32> for Owner {
			fn publish(&mut self, handle: Option<Handle>, value: u32) -> Handle {
				if let Some(handle) = handle {
					self.indices.derive(handle, value);
					handle
				} else {
					self.indices.create(value)
				}
			}
		}

		let mut owner = Owner {
			labels: Factory::new(),
			indices: Factory::new(),
		};
		let mut labels = owner.labels.listener();
		let mut indices = owner.indices.listener();

		let handle: Handle = owner.create(String::from("entity")).with(7).into();

		assert_eq!(labels.read().expect("label creation").handle(), &handle);
		assert_eq!(indices.read().expect("index creation").handle(), &handle);
	}

	#[test]
	fn derive_reuses_the_supplied_identity() {
		let mut factory = Factory::new();
		let mut listener = factory.listener();
		let handle = factory.create(String::from("source"));
		factory.derive(handle, String::from("derived"));

		let created = listener.read().expect("source creation");
		let derived = listener.read().expect("derived creation");

		assert_eq!(created.handle(), derived.handle());
		assert_eq!(derived.into_data(), "derived");
	}

	#[test]
	fn cloned_factories_share_the_creation_stream() {
		let original = Factory::new();
		let mut clone = original.clone();
		let mut listener = original.listener();

		let handle = clone.create(7);
		let message = listener.read().expect("clone publishes to shared channel");

		assert_eq!(message.handle(), &handle);
		assert_eq!(message.data(), &7);
	}
}

use std::sync::atomic::{AtomicU32, Ordering};

use crate::core::{
	channel::{Channel as _, DefaultChannel},
	listener::{DefaultListener, Listener},
	message::Message,
	targeted_message::TargetedMessage,
};
