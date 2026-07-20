//! Store, retrieve, and query baked resources through interchangeable backends.

pub mod redb_storage_backend;

use super::resource_handler::MultiResourceReader;
use crate::QueryableValue;
use crate::{
	asset::ResourceId, model::ArchivedQueryableValue, ArchivedSerializableResource, ProcessedAsset, SerializableResource,
};

/// The `QueryCursor` struct provides an opaque continuation point for paginated resource queries.
#[derive(
	Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct QueryCursor {
	pub(crate) token: Vec<u8>,
}

impl QueryCursor {
	pub fn new(token: Vec<u8>) -> Self {
		Self { token }
	}
}

/// The `QueryPredicate` enum defines one indexed property constraint for a resource query.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueryPredicate {
	Eq { property: String, value: QueryableValue },
}

/// The `Query` struct provides a class-filtered, paginated request to a storage backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
	pub class: String,
	pub predicates: Vec<QueryPredicate>,
	pub limit: usize,
	pub cursor: Option<QueryCursor>,
}

impl Query {
	pub fn new(class: &str) -> Self {
		Self {
			class: class.to_string(),
			predicates: Vec::new(),
			limit: usize::MAX,
			cursor: None,
		}
	}

	pub fn eq(mut self, property: &str, value: &str) -> Self {
		self.predicates.push(QueryPredicate::Eq {
			property: property.to_string(),
			value: QueryableValue::String(value.to_string()),
		});
		self
	}

	pub fn limit(mut self, limit: usize) -> Self {
		self.limit = limit;
		self
	}

	pub fn cursor(mut self, cursor: QueryCursor) -> Self {
		self.cursor = Some(cursor);
		self
	}

	pub fn matches(&self, resource: &SerializableResource, properties: &[crate::QueryableProperty]) -> bool {
		if resource.class != self.class {
			return false;
		}

		self.predicates.iter().all(|predicate| match predicate {
			QueryPredicate::Eq { property, value } => properties
				.iter()
				.any(|candidate| candidate.name == *property && &candidate.value == value),
		})
	}

	/// Returns whether archived metadata matches this query without deserializing it.
	pub fn matches_archived(&self, resource: &ArchivedSerializableResource) -> bool {
		if resource.class.as_str() != self.class {
			return false;
		}

		self.predicates.iter().all(|predicate| match predicate {
			QueryPredicate::Eq { property, value } => resource.queryable_properties.iter().any(|candidate| {
				candidate.name.as_str() == property
					&& match (&candidate.value, value) {
						(ArchivedQueryableValue::String(candidate), QueryableValue::String(value)) => {
							candidate.as_str() == value
						}
					}
			}),
		})
	}

	pub fn first_indexed_predicate(&self) -> Option<(&str, &QueryableValue)> {
		self.predicates.first().map(|predicate| match predicate {
			QueryPredicate::Eq { property, value } => (property.as_str(), value),
		})
	}
}

/// The `QueryPage` struct carries one result page and its optional continuation cursor.
#[derive(Debug)]
pub struct QueryPage<T> {
	pub items: Vec<T>,
	pub cursor: Option<QueryCursor>,
}

/// The `QueryError` enum identifies failures while a storage backend executes a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
	InvalidCursor,
	StorageFailure,
}

pub trait ReadStorageBackend: Sync + Send + downcast_rs::Downcast {
	fn list(&self) -> Result<Vec<String>, String>;
	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>) -> Option<(SerializableResource, MultiResourceReader)>;

	fn query(&self, query: Query) -> Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>;

	/// Returns the asset type from its URL when the backend can determine it.
	///
	/// Asset handlers use this value to skip unsupported sources before loading them.
	fn get_type<'a>(&'a self, url: ResourceId<'a>) -> Option<&'a str> {
		Some(url.get_extension())
	}

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> bool {
		self.read(id).is_some()
	}
}

pub trait WriteStorageBackend: Sync + Send + downcast_rs::Downcast {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;
	fn store(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, ()> {
		self.store_in(resource, data, &std::alloc::Global)
	}
	fn store_in(
		&self,
		resource: ProcessedAsset,
		data: &[u8],
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()>;
	fn sync(&self, _: &dyn ReadStorageBackend) {}

	fn start(&self, _: ResourceId<'_>) {}
}

downcast_rs::impl_downcast!(ReadStorageBackend);
downcast_rs::impl_downcast!(WriteStorageBackend);

pub trait StorageBackend: ReadStorageBackend + WriteStorageBackend {}

downcast_rs::impl_downcast!(StorageBackend);

#[cfg(test)]
pub mod tests {
	use std::{hash::Hasher, sync::Arc};

	use gxhash::HashMapExt;
	use utils::{hash::HashMap, sync::Mutex};

	use super::*;
	use crate::resource::resource_handler::tests::MemoryResourceReader;

	#[derive(Clone)]
	pub struct TestStorageBackend(pub Arc<Mutex<HashMap<String, (Box<[u8]>, Box<[u8]>)>>>);

	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::new())))
		}

		pub fn get_resources(&self) -> Vec<ProcessedAsset> {
			self.0
				.lock()
				.iter()
				.map(|x| {
					let resource: SerializableResource = crate::from_slice(&x.1 .0).unwrap();
					ProcessedAsset {
						id: resource.id,
						class: resource.class,
						resource: resource.resource,
						streams: resource.streams,
						queryable_properties: resource.queryable_properties,
					}
				})
				.collect()
		}

		pub fn get_resource(&self, name: ResourceId<'_>) -> Option<ProcessedAsset> {
			self.0
				.lock()
				.iter()
				.find(|x| {
					let resource: SerializableResource = crate::from_slice(&x.1 .0).unwrap();
					resource.id == name.as_ref()
				})
				.map(|x| {
					let resource: SerializableResource = crate::from_slice(&x.1 .0).unwrap();
					ProcessedAsset {
						id: resource.id,
						class: resource.class,
						resource: resource.resource,
						streams: resource.streams,
						queryable_properties: resource.queryable_properties,
					}
				})
		}

		pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
			Some(
				self.0
					.lock()
					.iter()
					.find(|x| {
						let resource: SerializableResource = crate::from_slice(&x.1 .0).unwrap();
						resource.id == name.as_ref()
					})?
					.1
					 .1
					.clone(),
			)
		}
	}

	impl ReadStorageBackend for TestStorageBackend {
		fn list<'a>(&'a self) -> Result<Vec<String>, String> {
			Ok(self.0.lock().keys().map(|x| x.to_string()).collect())
		}

		fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>) -> Option<(SerializableResource, MultiResourceReader)> {
			let (resource, data) = if let Some(e) = self.0.lock().get(id.as_ref()) {
				(e.0.clone(), e.1.clone())
			} else {
				return None;
			};

			let _ = id.get_base().to_string();

			let resource: SerializableResource = crate::from_slice(&resource).unwrap();

			let resource_reader = Box::new(MemoryResourceReader::new(data));

			Some((resource, resource_reader))
		}

		fn query(&self, _: Query) -> Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError> {
			Err(QueryError::StorageFailure)
		}
	}

	impl WriteStorageBackend for TestStorageBackend {
		fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
			self.0.lock().remove(id.as_ref());
			Ok(())
		}

		fn store_in(
			&self,
			resource: ProcessedAsset,
			data: &[u8],
			allocator: &dyn std::alloc::Allocator,
		) -> Result<SerializableResource, ()> {
			let id = resource.id.clone();
			let size = data.len();

			let hash = {
				let mut hasher = gxhash::GxHasher::with_seed(961961961961961);

				std::hash::Hasher::write(&mut hasher, data); // Hash binary data (For identifying the resource)

				hasher.finish()
			};

			let container = resource.into_serializable(hash, size);
			let serialized_container = crate::to_vec_in(&container, allocator).unwrap();

			self.0.lock().insert(id, (serialized_container.to_vec().into(), data.into()));

			Ok(container)
		}

		fn sync<'s, 'a>(&'s self, _: &'a dyn ReadStorageBackend) -> () {
			{}
		}
	}

	impl StorageBackend for TestStorageBackend {}
}
