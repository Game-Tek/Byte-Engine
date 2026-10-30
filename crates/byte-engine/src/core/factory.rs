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
pub struct Factory<T: Clone + Send + Sync + 'static> {
	channel: DefaultChannel<CreateMessage<T>>,
	/// The bus observer and this factory's catalog type, resolved once so creations never hash a type id.
	observation: Option<(MessageObserver, ObservedType)>,
}

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// The `Creator` trait provides fluent creation through an owning API boundary.
///
/// Use [`Self::create`] for the first representation of an entity, then chain
/// [`Creation::with`] for each additional representation that must share its handle.
pub trait Creator<T> {
	/// Creates a value in the owner's matching factory and starts a shared-handle creation chain.
	fn create(&self, value: T) -> Creation<'_, Self>
	where
		Self: Sized,
	{
		let handle = Handle::new();
		self.publish(handle, value);
		Creation { creator: self, handle }
	}

	/// Publishes a value under `handle` in the owner's matching factory.
	fn publish(&self, handle: Handle, value: T);
}

/// The `Creation` struct keeps one stable handle while an owner creates multiple entity representations.
///
/// Chain [`Self::with`] to publish another representation, then convert the
/// result into [`Handle`] when another API needs the entity identity.
pub struct Creation<'creator, C: ?Sized> {
	creator: &'creator C,
	handle: Handle,
}

impl<C: ?Sized> Creation<'_, C> {
	/// Publishes another representation through the same owner under this creation's handle.
	pub fn with<T>(self, value: T) -> Self
	where
		C: Creator<T>,
	{
		self.creator.publish(self.handle, value);
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

impl<T: Clone + Send + Sync + 'static> Default for Factory<T> {
	fn default() -> Self {
		Self::new()
	}
}

impl<T: Clone + Send + Sync + 'static> Factory<T> {
	/// Creates an empty creation stream.
	///
	/// Next, call [`Self::listener`] for each system that mirrors created values,
	/// then publish values through [`Self::create`].
	#[inline]
	pub fn new() -> Self {
		Self {
			channel: DefaultChannel::new(),
			observation: None,
		}
	}

	/// Publishes a value with a new stable handle.
	///
	/// Consumers read the resulting [`CreateMessage`] from listeners created by
	/// [`Self::listener`]. Pass the returned handle to [`Self::derive`] when a
	/// second factory publishes another representation of the same entity.
	#[inline(always)]
	pub fn create(&self, data: T) -> Handle {
		let handle = Handle::new();
		self.derive(handle, data);
		handle
	}

	/// Creates multiple entities in a single statically-sized batch.
	///
	/// Returns an array of [`Handle`]s corresponding to the created entities.
	pub fn create_array<const N: usize>(&self, data: [T; N]) -> [Handle; N] {
		data.map(|data| self.create(data))
	}

	/// Publishes a value with an existing stable handle.
	///
	/// Use this after [`Self::create`] when another system-specific representation
	/// must retain the original entity identity.
	#[inline(always)]
	pub fn derive(&self, handle: Handle, data: T) {
		if let Some((observer, observed)) = &self.observation {
			observer.observe_entity(handle, *observed);
		}
		let message = CreateMessage::new(handle, data);

		self.channel.send(message);
	}

	/// Publishes each value under its handle, in order, as one batch.
	///
	/// Use it instead of calling [`Self::derive`] in a loop when one owner publishes many values at once, such as a
	/// level spawning every component of a kind: the batch reserves message slots a chunk at a time instead of one
	/// at a time.
	pub fn derive_many(&self, values: impl ExactSizeIterator<Item = (Handle, T)>) {
		let observation = &self.observation;
		self.channel.send_many(values.map(|(handle, data)| {
			if let Some((observer, observed)) = observation {
				observer.observe_entity(handle, *observed);
			}
			CreateMessage::new(handle, data)
		}));
	}

	/// Creates a consumer for creation messages, starting at the current tick's first one.
	///
	/// Next, call [`Self::create`] or [`Self::derive`] and drain the messages
	/// through [`crate::core::listener::Listener::read`].
	#[track_caller]
	pub fn listener(&self) -> DefaultListener<CreateMessage<T>> {
		self.channel.listener()
	}

	#[inline]
	pub(crate) fn from_channel(channel: DefaultChannel<CreateMessage<T>>) -> Self {
		let observation = channel.topic.observation::<T>();
		Self { channel, observation }
	}
}

#[derive(Debug, Clone)]
/// The `CreateMessage` struct carries a created value and the stable handle
/// shared by systems that mirror it.
pub struct CreateMessage<T: Clone> {
	handle: Handle,
	data: T,
}

impl<T: Clone> CreateMessage<T> {
	/// Creates a creation or targeted replacement message for an existing handle.
	pub fn new(handle: Handle, data: T) -> Self {
		CreateMessage { handle, data }
	}

	pub fn data(&self) -> &T {
		&self.data
	}

	pub fn into_data(self) -> T {
		self.data
	}

	pub fn handle(&self) -> Handle {
		self.handle
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
/// The `Handle` struct identifies one creation stream entry across consuming
/// systems.
///
/// Get one from [`Factory::create`], then use [`Factory::derive`] to share its identity across representations.
/// Allocation terminates the process if the nonzero 32-bit identity space is exhausted.
#[repr(transparent)]
pub struct Handle(std::num::NonZeroU32);

const _: () = assert!(size_of::<Option<Handle>>() == size_of::<u32>());

impl Handle {
	/// Allocates an identity shared by factory creations and message-backed components.
	pub(crate) fn new() -> Self {
		// Identity exhaustion is terminal: issuing another identity could reuse a live entity's ID.
		let id = std::num::NonZeroU32::new(COUNTER.fetch_add(1, Ordering::Relaxed).wrapping_add(1))
			.unwrap_or_else(|| Self::exhausted());
		Self(id)
	}

	/// Allocates `count` consecutive identities and returns the first, so each of the others is the first plus its
	/// offset. `count` must be at least one.
	pub(crate) fn run(count: u32) -> Self {
		debug_assert!(count > 0, "A handle run must hold at least one handle.");
		let previous = COUNTER.fetch_add(count, Ordering::Relaxed);
		// The run's last identity, `previous + count`, must fit, or it would wrap onto identities already handed out.
		if previous.checked_add(count).is_none() {
			Self::exhausted();
		}
		Self::from_id(previous + 1)
	}

	/// Stops the process when no unique identity remains.
	#[cold]
	fn exhausted() -> ! {
		eprintln!("Factory identity allocation failed. The process exhausted its 32-bit identities. Restart the application.");
		std::process::abort();
	}

	/// Returns the process-local numeric identity used by inspection and diagnostics.
	pub fn id(self) -> u32 {
		self.0.get()
	}

	/// Restores an identity supplied by a trusted engine protocol adapter.
	pub(crate) fn from_id(id: u32) -> Self {
		Self(
			std::num::NonZeroU32::new(id).expect(
				"Entity identity is invalid. Zero is reserved for absent entities. Use an identity returned by a factory.",
			),
		)
	}
}

#[cfg(test)]
mod tests {
	use super::Factory;
	use crate::core::{listener::Listener, message_bus::MessageBus};

	#[test]
	fn array_creation_preserves_message_order() {
		let factory = Factory::new();
		let mut listener = factory.listener();
		assert_eq!(factory.create_array([]), []);
		let handles = factory.create_array([10, 20, 30]);
		let messages = listener.to_vec();
		assert_eq!(messages.iter().map(|message| message.handle()).collect::<Vec<_>>(), handles);
		assert_eq!(
			messages.iter().map(|message| *message.data()).collect::<Vec<_>>(),
			[10, 20, 30]
		);
	}

	#[test]
	fn create_assigns_distinct_handles_and_broadcasts_in_creation_order() {
		let factory = Factory::new();
		let mut listener = factory.listener();

		let first = factory.create("first");
		let second = factory.create("second");
		let messages = listener.to_vec();

		assert_ne!(first, second);
		assert_eq!(messages.len(), 2);
		assert_eq!(messages[0].handle(), first);
		assert_eq!(messages[0].data(), &"first");
		assert_eq!(messages[1].handle(), second);
		assert_eq!(messages[1].data(), &"second");
	}

	#[test]
	fn observed_factories_catalog_every_type_once_under_the_shared_handle() {
		let bus = MessageBus::default();
		let observer = bus.observe().expect("attach observer");
		let messages = bus.new_scope("factory-observation");
		let labels = messages.factory::<String>();
		let indices = messages.factory::<u32>();

		let handle = labels.create("entity".to_string());
		labels.derive(handle, "renamed".to_string());
		indices.derive(handle, 7);

		let entities = observer.entities();
		assert_eq!(entities.len(), 1);
		assert_eq!(entities[0].handle(), handle);
		assert_eq!(
			entities[0].types(),
			[std::any::type_name::<String>(), std::any::type_name::<u32>()]
		);
	}
}

use std::sync::atomic::{AtomicU32, Ordering};

use crate::core::{
	channel::{Channel as _, DefaultChannel},
	listener::DefaultListener,
	message::Message,
	message_observer::{MessageObserver, ObservedType},
	targeted_message::TargetedMessage,
};
