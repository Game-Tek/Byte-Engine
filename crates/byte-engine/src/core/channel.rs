//! Broadcast channels used to connect engine systems without direct ownership.
//!
//! Create a [`DefaultChannel`], clone it for producers, and create one
//! [`crate::core::listener::DefaultListener`] per consumer. Use
//! [`crate::core::factory::Factory`] instead when messages represent entity
//! creation and require stable handles. Application-owned channels should come
//! from a shared [`crate::core::message_bus::MessageScope`].

use std::{panic::Location, sync::Arc};

use crate::core::{
	factory::Handle,
	listener::DefaultListener,
	message_bus::{MessageBus, MessageBusConfig, Topic},
};

/// Pool size of a channel that owns its own bus instead of sharing an application bus.
const STANDALONE_CAPACITY: usize = 4 * 1024 * 1024;

/// The `Channel` trait defines message publication independently of the underlying transport.
pub trait Channel<M> {
	fn send(&self, message: M);
}

/// The `DefaultChannel` struct provides a cached typed route into shared message-bus storage.
///
/// Create a [`Self::listener`] for each consumer before calling
/// [`Channel::send`]. Use [`crate::core::factory::Factory`] instead when the
/// message must include a stable creation handle.
pub struct DefaultChannel<M>
where
	M: Clone + Send + Sync + 'static,
{
	pub(crate) topic: Arc<Topic<M>>,
}

impl<M> Clone for DefaultChannel<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn clone(&self) -> Self {
		Self {
			topic: Arc::clone(&self.topic),
		}
	}
}

impl<M> Default for DefaultChannel<M>
where
	M: Clone + Send + Sync + 'static,
{
	fn default() -> Self {
		Self::new()
	}
}

impl<M> DefaultChannel<M>
where
	M: Clone + Send + Sync + 'static,
{
	/// Creates an independent channel with its own 4 MiB message pool.
	///
	/// The private bus has no ticks, so its listeners are future-only.
	/// Next, create each consumer with [`Self::listener`] before producers start
	/// calling [`Channel::send`]. Use a shared message scope for application-owned
	/// channels that should appear in unified diagnostics.
	pub fn new() -> Self {
		let bus = MessageBus::new(MessageBusConfig {
			capacity: STANDALONE_CAPACITY,
			ticks: false,
		})
		.unwrap_or_else(|error| panic!("{error}"));
		bus.root_scope("standalone").channel()
	}

	/// Creates a listener that starts at the current tick's first message.
	///
	/// On an application bus this includes messages sent earlier in the same
	/// tick. Next, keep the listener with the consuming system and call
	/// [`crate::core::listener::Listener::read`] during that system's update.
	/// Drop the listener when the system stops reading, so the storage behind
	/// it returns to the pool.
	#[track_caller]
	pub fn listener(&self) -> DefaultListener<M> {
		DefaultListener::from_token(self.topic.subscribe(Location::caller()))
	}

	pub(crate) fn from_topic(topic: Arc<Topic<M>>) -> Self {
		Self { topic }
	}

	/// Installs the route's one diagnostics watcher, which sees every later publication on this route.
	///
	/// Every channel and factory of the same scope and message type shares the route,
	/// so the watcher sees their sends too. Keep it short: it runs on the sender's thread.
	pub(crate) fn watch(&self, watcher: impl Fn(&M) + Send + Sync + 'static) {
		self.topic.watch(watcher);
	}

	/// Removes one terminally deleted handle from this bus's optional diagnostics catalog.
	#[inline(always)]
	pub(crate) fn forget_entity(&self, handle: Handle) {
		self.topic.forget_entity(handle);
	}
}

impl<M> Channel<M> for DefaultChannel<M>
where
	M: Clone + Send + Sync + 'static,
{
	/// Publishes one message. Waits only when the bus's pool has no free chunk.
	fn send(&self, message: M) {
		self.topic.send(message);
	}
}
