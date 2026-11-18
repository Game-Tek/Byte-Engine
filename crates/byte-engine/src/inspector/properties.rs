//! The properties the inspector tracks for each entity: an open set, each kept under the name it was registered with.
//!
//! [`DefaultInspector::watch_property`](super::DefaultInspector::watch_property) adds one. Each property keeps the
//! latest value of every entity in a table of its own type, so a publication only replaces a value in place; a value
//! is serialized only when a query reads it, through [`Properties::of`].

use std::{collections::HashMap, sync::Arc};

use serde::Serialize;
use utils::sync::{Mutex, RwLock};

use crate::core::factory::Handle;

/// The `Table` trait is one property's latest values, by entity, read without knowing their type.
trait Table: Send + Sync {
	/// The entity's latest value, serialized, if it published one.
	fn value(&self, handle: Handle) -> Option<serde_json::Value>;
	/// Forgets the entity, once it was deleted.
	fn forget(&self, handle: Handle);
}

/// The `Typed` struct is one property's latest values, in their own type.
struct Typed<T>(Mutex<HashMap<Handle, T>>);

impl<T: Serialize + Send + Sync> Table for Typed<T> {
	fn value(&self, handle: Handle) -> Option<serde_json::Value> {
		let values = self.0.lock();
		let value = values.get(&handle)?;
		match serde_json::to_value(value) {
			Ok(value) => Some(value),
			Err(error) => {
				log::warn!(
					"An inspector property couldn't be read. The most likely cause is that its type doesn't serialize to JSON: {error}. Make its `Serialize` implementation produce JSON-compatible data."
				);
				None
			}
		}
	}

	fn forget(&self, handle: Handle) {
		self.0.lock().remove(&handle);
	}
}

/// The `Properties` struct holds every tracked property's table, in registration order, which is the order a query
/// lists them in.
#[derive(Default)]
pub(super) struct Properties {
	tables: RwLock<Vec<(&'static str, Arc<dyn Table>)>>,
}

impl Properties {
	/// Adds the property `name`, and returns the function that records an entity's new value of it.
	pub(super) fn add<T: Serialize + Send + Sync + 'static>(
		&self,
		name: &'static str,
	) -> Result<impl Fn(Handle, T) + Send + Sync + 'static, String> {
		let mut tables = self.tables.write();
		if name.is_empty() {
			return Err(
				"Inspector property could not be registered. The most likely cause is that its name is empty. Give it the name clients read it by."
					.to_string(),
			);
		}
		if tables.iter().any(|(registered, _)| *registered == name) {
			return Err(format!(
				"Inspector property could not be registered. The most likely cause is that '{name}' is already registered. Give it another name."
			));
		}
		let table = Arc::new(Typed::<T>(Mutex::new(HashMap::new())));
		tables.push((name, Arc::clone(&table) as Arc<dyn Table>));
		Ok(move |handle, value| {
			table.0.lock().insert(handle, value);
		})
	}

	/// Returns every property the entity has, by name, serialized.
	pub(super) fn of(&self, handle: Handle) -> Vec<(&'static str, serde_json::Value)> {
		self.tables
			.read()
			.iter()
			.filter_map(|(name, table)| Some((*name, table.value(handle)?)))
			.collect()
	}

	/// Forgets every property of the entity, once it was deleted.
	pub(super) fn forget(&self, handle: Handle) {
		for (_, table) in self.tables.read().iter() {
			table.forget(handle);
		}
	}
}
