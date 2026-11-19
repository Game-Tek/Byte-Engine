//! The properties the inspector tracks for each entity: an open set, each kept under the name it was registered with.
//!
//! [`DefaultInspector::watch_property`](super::DefaultInspector::watch_property) adds one. Each property keeps the
//! latest value of every entity in a table of its own type, so a publication only replaces a value in place; a value
//! is serialized only when a query reads it, through [`Properties::read`].

use std::{collections::HashMap, sync::Arc};

use serde::Serialize;
use utils::sync::{Mutex, RwLock};

use super::InspectedEntity;
use crate::core::factory::Handle;

/// The `Table` trait is one property's latest values, by entity, read without knowing their type.
trait Table: Send + Sync {
	/// Appends this property's values to a handle-sorted entity snapshot.
	fn read(&self, name: &'static str, entities: &mut [InspectedEntity]);
	/// Forgets the entity, once it was deleted.
	fn forget(&self, handle: Handle);
}

/// The `Typed` struct is one property's latest values, in their own type.
struct Typed<T>(Mutex<HashMap<Handle, T>>);

impl<T: Serialize + Send + Sync> Table for Typed<T> {
	fn read(&self, name: &'static str, entities: &mut [InspectedEntity]) {
		let values = self.0.lock();
		let append = |entity: &mut InspectedEntity, value: &T| match serde_json::to_value(value) {
			Ok(value) => entity.properties.push((name, value)),
			Err(error) => log::warn!(
				"An inspector property couldn't be read. The most likely cause is that its type doesn't serialize to JSON: {error}. Make its `Serialize` implementation produce JSON-compatible data."
			),
		};
		// Sparse tables join their stored handles by binary search; dense tables and narrow queries use direct lookups.
		let search_steps = entities.len().checked_ilog2().unwrap_or(0) as usize + 1;
		if values.len().saturating_mul(search_steps) < entities.len() {
			for (handle, value) in values.iter() {
				if let Ok(index) = entities.binary_search_by_key(&handle.id(), |entity| entity.handle().id()) {
					append(&mut entities[index], value);
				}
			}
		} else {
			for entity in entities {
				if let Some(value) = values.get(&entity.handle()) {
					append(entity, value);
				}
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

	/// Appends properties in registration order, locking each table once for the whole handle-sorted snapshot.
	pub(super) fn read(&self, entities: &mut [InspectedEntity]) {
		debug_assert!(entities.windows(2).all(|pair| pair[0].handle().id() < pair[1].handle().id()));
		for (name, table) in self.tables.read().iter() {
			table.read(name, entities);
		}
	}

	/// Forgets every property of the entity, once it was deleted.
	pub(super) fn forget(&self, handle: Handle) {
		for (_, table) in self.tables.read().iter() {
			table.forget(handle);
		}
	}
}
