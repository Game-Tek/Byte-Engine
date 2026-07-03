//! Shared ownership, messaging, factories, and task primitives.
//!
//! Use [`factory::Factory`] when systems need stable creation handles and a
//! stream of created values. Use [`channel::DefaultChannel`] for general
//! broadcast messages and consume them through [`listener::Listener`].
//! Long-lived shared objects should be exposed through [`EntityHandle`].

pub mod channel;
pub mod entity;
pub mod factory;
pub mod listener;
pub mod message;

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
