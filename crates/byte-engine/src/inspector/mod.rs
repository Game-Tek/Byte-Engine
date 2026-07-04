//! Runtime inspection contracts and protocol-facing state access.
//!
//! Implement [`Inspectable`] on entities exposed to tooling, then register their
//! handles with an [`Inspector`]. Protocol adapters should query this object
//! rather than reaching into application subsystems directly.

use std::{fmt::Debug, sync::Arc};

use utils::sync::Mutex;

use crate::application::{Receiver, Sender};
use crate::{
	application::Events,
	core::{listener::Listener, Entity, EntityHandle},
};

#[cfg(feature = "headed")]
#[doc(hidden)]
pub mod http;

/// The [`Inspectable`] trait defines the read and mutation surface exposed to
/// external engine tooling.
pub trait Inspectable: Send + Sync {
	/// Returns a display string for inspection responses.
	fn as_string(&self) -> String;

	/// Returns the class name used by inspection filters.
	fn class_name(&self) -> &'static str {
		std::any::type_name::<Self>()
	}

	/// Applies an inspector-provided string value to a named property.
	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		Err(
			"Inspector mutation is not implemented. The most likely cause is that this inspectable type did not override set."
				.to_string(),
		)
	}
}

/// The [`Inspector`] struct owns the entity registry and application controls
/// shared by Byte Engine Inspection Protocol adapters.
pub struct Inspector {
	entities: Mutex<Vec<EntityHandle<dyn Inspectable>>>,
	events: Sender<Events>,
}

impl Inspector {
	/// Creates an inspector that can close the owning application through its event channel.
	pub fn new(tx: Sender<Events>) -> Self {
		let entities = Mutex::new(Vec::<EntityHandle<dyn Inspectable>>::with_capacity(32768));

		Self { entities, events: tx }
	}

	/// Returns inspectable entities, optionally filtered by class name.
	pub fn get_entities(&self, class: Option<&str>) -> Vec<EntityHandle<dyn Inspectable>> {
		let entities = self.entities.lock();
		let mut result = Vec::new();

		for entity in entities.iter() {
			if let Some(class) = class {
				if entity.class_name() == class {
					result.push(entity.clone());
				}
			} else {
				result.push(entity.clone());
			}
		}

		result
	}

	/// Applies a property update to the inspectable entity at the given index.
	pub fn call_set(&self, index: usize, key: &str, value: &str) -> Result<(), String> {
		let entities = self.entities.lock();
		let entity = entities.get(index).ok_or(
			"Inspector entity not found. The most likely cause is that the entity index came from an outdated inspection response."
				.to_string(),
		)?;
		Err("Inspector mutation dispatch is not implemented. The most likely cause is that Inspector::call_set is still a placeholder.".to_string())
	}

	/// Requests application shutdown through the inspector event channel.
	pub fn close_application(&self) {
		self.events.send(Events::Close).unwrap();
	}
}
