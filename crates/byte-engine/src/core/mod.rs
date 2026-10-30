//! Shared ownership, messaging, factories, and lane primitives.
//!
//! Use [`factory::Factory`] when systems need stable creation handles and a
//! stream of created values. Use [`channel::DefaultChannel`] for general
//! broadcast messages and consume them through [`listener::Listener`].
//! Long-lived shared objects should be exposed through [`EntityHandle`].

#[doc(hidden)]
pub(crate) mod async_runtime;
pub mod channel;
pub mod entity;
pub mod factory;
pub mod listener;
pub mod message;
pub mod message_bus;
pub mod message_observer;
pub mod publisher;
pub mod targeted_message;

pub mod alley;
pub mod machine;
pub mod threadpool;

pub use entity::Entity;
pub use entity::EntityHandle;
pub use factory::Creator;
