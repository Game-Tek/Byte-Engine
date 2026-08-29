//! Broadcast channels used to connect engine systems without direct ownership.
//!
//! Create a [`DefaultChannel`], clone it for producers, and create one
//! [`crate::core::listener::DefaultListener`] per consumer. Use
//! [`crate::core::factory::Factory`] instead when messages represent entity
//! creation and require stable handles. Application-owned channels should come
//! from a shared [`crate::core::message_bus::MessageScope`].

use std::{any::type_name, fmt, sync::Arc};

use crate::core::{
	factory::Handle,
	listener::DefaultListener,
	message_bus::{MessageBus, MessageRouteError, Topic},
};

/// The `Channel` trait defines message publication independently of the underlying transport.
pub trait Channel<M> {
	fn send(&self, message: M);
}

/// The `TrySendError` enum returns a message when a typed route cannot accept it immediately.
pub enum TrySendError<M> {
	/// No listener was registered when publication was attempted.
	Disconnected(M),
	/// The slowest listener still retains the route's complete fixed capacity.
	Full(M),
	/// The monotonic route ticket reached its representable limit.
	SequenceExhausted(M),
}

impl<M> TrySendError<M> {
	/// Returns the message that was not published.
	pub fn into_inner(self) -> M {
		match self {
			Self::Disconnected(message) | Self::Full(message) | Self::SequenceExhausted(message) => message,
		}
	}
}

impl<M> fmt::Debug for TrySendError<M> {
	fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		formatter.write_str(match self {
			Self::Disconnected(_) => "TrySendError::Disconnected(..)",
			Self::Full(_) => "TrySendError::Full(..)",
			Self::SequenceExhausted(_) => "TrySendError::SequenceExhausted(..)",
		})
	}
}

/// The `DefaultChannel` struct provides a cached typed route into fixed message-bus storage.
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
	/// Creates an independent channel with the engine's standard 128-message capacity.
	///
	/// Next, create each consumer with [`Self::listener`] before producers start
	/// calling [`Channel::send`]. Use a shared message scope for application-owned
	/// channels that should appear in unified diagnostics.
	pub fn new() -> Self {
		Self::with_capacity(128)
	}

	/// Creates an independent channel with a fixed message capacity.
	pub fn with_capacity(capacity: usize) -> Self {
		let bus = MessageBus::new(crate::core::message_bus::MessageBusConfig::standalone::<M>(capacity))
			.expect("A Rust message layout must produce a valid standalone bus");
		bus.root_scope("standalone").channel()
	}

	/// Creates a listener for messages sent after registration.
	///
	/// Next, keep the listener with the consuming system and call
	/// [`crate::core::listener::Listener::read`] during that system's update.
	pub fn listener(&self) -> DefaultListener<M> {
		self.try_listener().unwrap_or_else(|error| panic!("{error}"))
	}

	/// Tries to create a future-only listener without exceeding startup metadata.
	pub fn try_listener(&self) -> Result<DefaultListener<M>, MessageRouteError> {
		self.topic.subscribe().map(DefaultListener::from_token)
	}

	/// Attempts to publish once without waiting for a slow listener.
	pub fn try_send(&self, message: M) -> Result<(), TrySendError<M>> {
		match self.topic.try_send(message) {
			Err(TrySendError::Full(message)) => {
				self.topic.record_full();
				Err(TrySendError::Full(message))
			}
			result => result,
		}
	}

	pub(crate) fn from_topic(topic: Arc<Topic<M>>) -> Self {
		Self { topic }
	}

	/// Returns the optional diagnostics owner attached to this route's bus.
	pub(crate) fn observer(&self) -> Option<crate::core::message_observer::MessageObserver> {
		self.topic.observer().cloned()
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
	fn send(&self, mut message: M) {
		let mut record_full = true;
		loop {
			let result = if record_full {
				self.try_send(message)
			} else {
				self.topic.try_send(message)
			};
			match result {
				Ok(()) => return,
				Err(TrySendError::Disconnected(_)) => {
					log::debug!("No listeners for message type '{}'.", type_name::<M>());
					return;
				}
				Err(TrySendError::Full(returned)) => {
					message = returned;
					record_full = false;
					std::hint::spin_loop();
				}
				Err(TrySendError::SequenceExhausted(_)) => {
					panic!(
						"Message sequence exhausted for '{}'. The most likely cause is that one route published u64::MAX messages.",
						type_name::<M>()
					);
				}
			}
		}
	}
}
