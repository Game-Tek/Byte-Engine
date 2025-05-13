use super::{property::Subscriber, EntityHandle};

/// Trait for an event-like object.
/// Allows an event object to be subscribed to and to be triggered.
pub trait Event {
}

#[cfg(test)]
#[allow(dead_code)]
mod tests {
	use crate::core::{entity::EntityBuilder, spawn, Entity};

	use super::*;
}
