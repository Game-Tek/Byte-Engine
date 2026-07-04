//! Shared ownership, messaging, factories, and task primitives.
//!
//! Use [`factory::Factory`] when systems need stable creation handles and a
//! stream of created values. Use [`channel::DefaultChannel`] for general
//! broadcast messages and consume them through [`listener::Listener`].
//! Long-lived shared objects should be exposed through [`EntityHandle`].

#[doc(hidden)]
pub mod channel;
#[doc(hidden)]
pub mod entity;
#[doc(hidden)]
pub mod factory;
#[doc(hidden)]
pub mod listener;
#[doc(hidden)]
pub mod message;

#[doc(hidden)]
pub mod task;

use std::ops::Deref;

pub use entity::Entity;
pub use entity::EntityHandle;
pub use task::Task;
use utils::sync::{Arc, RwLock};

#[cfg(test)]
mod tests {
	use super::*;
}
