//! Byte-Engine inspector module.
//! Provides interfaces to interact with the engine's internal state.

use std::{fmt::Debug, sync::Arc};

use crate::application::{Receiver, Sender};
use utils::sync::Mutex;

use crate::{
	application::Events,
	core::{listener::Listener, Entity, EntityHandle},
};

pub mod http;

pub trait Inspectable: Send + Sync {
	fn as_string(&self) -> String;

	fn class_name(&self) -> &'static str {
		std::any::type_name::<Self>()
	}

	fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
		Err("Not implemented".to_string())
	}
}

/// The inspector allows different implementations of the Byte Engine Inspection Protocol to interact an query the engine's internal state.
pub struct Inspector {
	entities: Mutex<Vec<EntityHandle<dyn Inspectable>>>,
	events: Sender<Events>,
}

impl Inspector {
	pub fn new(tx: Sender<Events>) -> Self {
		let entities = Mutex::new(Vec::<EntityHandle<dyn Inspectable>>::with_capacity(32768));

		Self { entities, events: tx }
	}

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

	pub fn call_set(&self, index: usize, key: &str, value: &str) -> Result<(), String> {
		let entities = self.entities.lock();
		let entity = entities.get(index).ok_or("Entity not found".to_string())?;
		Err("Not implemented".to_string())
	}

	pub fn close_application(&self) {
		self.events.send(Events::Close).unwrap();
	}
}
