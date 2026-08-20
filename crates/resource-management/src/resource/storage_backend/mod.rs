//! Store, retrieve, and query baked resources through interchangeable backends.

pub mod redb_storage_backend;
mod transaction;

pub use transaction::ResourceTransaction;
use transaction::{ResourceTransactionCommit, ResourceWriteOutput, ResourceWriter, StagedResourceFile};

pub trait StorageBackend: ReadStorageBackend + WriteStorageBackend {}
pub trait DynStorageBackend: DynReadStorageBackend + DynWriteStorageBackend {}

pub trait ReadStorageBackend: Sync + Send {
	fn list(&self) -> impl Future<Output = Result<Vec<String>, String>>;
	fn read<'a>(&'a self, id: ResourceId<'a>)
		-> impl Future<Output = Option<(SerializableResource, MultiResourceReader)>> + 'a;

	fn query(
		&self,
		query: Query,
	) -> impl Future<Output = Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>>;

	/// Returns development-time bake messages even when the requested resource was not stored.
	#[cfg(debug_assertions)]
	fn read_trace<'a>(&'a self, _: ResourceId<'a>) -> impl Future<Output = Result<Vec<crate::ResourceTraceItem>, String>> + 'a {
		async { Ok(Vec::new()) }
	}

	/// Returns the asset type from its URL when the backend can determine it.
	///
	/// Asset handlers use this value to skip unsupported sources before loading them.
	fn get_type<'a>(&'a self, url: ResourceId<'a>) -> Option<&'a str> {
		Some(url.get_extension())
	}

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> impl Future<Output = bool> + 'a {
		async move { self.read(id).await.is_some() }
	}
}

pub trait DynReadStorageBackend: Send + Sync {
	fn list(&self) -> BoxedFuture<'_, Result<Vec<String>, String>>;
	fn read<'a>(&'a self, id: ResourceId<'a>) -> BoxedFuture<'a, Option<(SerializableResource, MultiResourceReader)>>;
	fn query(
		&self,
		query: Query,
	) -> BoxedFuture<'_, Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>>;
	#[cfg(debug_assertions)]
	fn read_trace<'a>(&'a self, _id: ResourceId<'a>) -> BoxedFuture<'a, Result<Vec<crate::ResourceTraceItem>, String>> {
		Box::pin(async { Ok(Vec::new()) })
	}

	fn get_type<'a>(&'a self, id: ResourceId<'a>) -> Option<&'a str>;

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> BoxedFuture<'a, bool>;
}

pub trait WriteStorageBackend: Sync + Send {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;

	/// Reserves exact payload storage before a processor starts writing.
	///
	/// Await [`ResourceTransaction::write_all`] until exactly `size` bytes have
	/// been accepted, then await [`ResourceTransaction::commit`].
	fn begin_resource<'a>(
		&'a self,
		id: ResourceId<'_>,
		size: usize,
	) -> impl Future<Output = Result<ResourceTransaction<'a>, ()>> + 'a;

	fn store<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		self.store_in(resource, data, &std::alloc::Global)
	}

	/// Stores an owned payload without copying large buffers through the file staging buffer.
	fn store_owned<'a, T: compio::buf::IoBuf>(
		&'a self,
		resource: ProcessedAsset,
		data: T,
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		self.store_owned_in(resource, data, &std::alloc::Global)
	}

	/// Stores an owned payload while using `allocator` for serialized resource metadata.
	fn store_owned_in<'a, T: compio::buf::IoBuf>(
		&'a self,
		resource: ProcessedAsset,
		data: T,
		allocator: &'a dyn std::alloc::Allocator,
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		async move {
			let size = data.buf_len();
			let mut transaction = self.begin_resource(ResourceId::new(resource.id()), size).await?;
			let compio::buf::BufResult(result, _) = compio::io::AsyncWriteExt::write_all(&mut transaction, data).await;
			result.map_err(|_| ())?;
			transaction.commit(resource, allocator).await
		}
	}

	fn store_in<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
		allocator: &'a dyn std::alloc::Allocator,
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		async move {
			let mut transaction = self.begin_resource(ResourceId::new(resource.id()), data.len()).await?;
			transaction.write_all(data).await.map_err(|_| ())?;
			transaction.commit(resource, allocator).await
		}
	}

	fn sync<T: ReadStorageBackend>(&self, _: &T) {}

	/// Replaces development-time bake messages without creating a resource entry.
	#[cfg(debug_assertions)]
	fn replace_trace(&self, _: ResourceId<'_>, _: &[crate::ResourceTraceItem]) -> Result<(), String> {
		Ok(())
	}

	fn start(&self, _: ResourceId<'_>) {}
}

pub trait DynWriteStorageBackend: Send + Sync {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;
	fn begin_resource<'a>(&'a self, id: ResourceId<'_>, size: usize) -> BoxedFuture<'a, Result<ResourceTransaction<'a>, ()>>;
	fn store<'a>(&'a self, resource: ProcessedAsset, data: &'a [u8]) -> BoxedFuture<'a, Result<SerializableResource, ()>>;
	fn store_in<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
		allocator: &'a dyn std::alloc::Allocator,
	) -> BoxedFuture<'a, Result<SerializableResource, ()>>;
	#[cfg(debug_assertions)]
	fn replace_trace(&self, _: ResourceId<'_>, _: &[crate::ResourceTraceItem]) -> Result<(), String> {
		Ok(())
	}
	fn start(&self, _: ResourceId<'_>) {}
}

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

impl<T: ReadStorageBackend> DynReadStorageBackend for T {
	fn list(&self) -> BoxedFuture<'_, Result<Vec<String>, String>> {
		Box::pin(self.list())
	}

	fn read<'a>(&'a self, id: ResourceId<'a>) -> BoxedFuture<'a, Option<(SerializableResource, MultiResourceReader)>> {
		Box::pin(self.read(id))
	}

	fn query(
		&self,
		query: Query,
	) -> BoxedFuture<'_, Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>> {
		Box::pin(self.query(query))
	}

	#[cfg(debug_assertions)]
	fn read_trace<'a>(&'a self, id: ResourceId<'a>) -> BoxedFuture<'a, Result<Vec<crate::ResourceTraceItem>, String>> {
		Box::pin(self.read_trace(id))
	}

	fn get_type<'a>(&'a self, id: ResourceId<'a>) -> Option<&'a str> {
		self.get_type(id)
	}

	fn exists<'a>(&'a self, id: ResourceId<'a>) -> BoxedFuture<'a, bool> {
		Box::pin(self.exists(id))
	}
}

impl<T: WriteStorageBackend> DynWriteStorageBackend for T {
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
		self.delete(id)
	}

	fn begin_resource<'a>(&'a self, id: ResourceId<'_>, size: usize) -> BoxedFuture<'a, Result<ResourceTransaction<'a>, ()>> {
		Box::pin(WriteStorageBackend::begin_resource(self, id, size))
	}

	fn store<'a>(&'a self, resource: ProcessedAsset, data: &'a [u8]) -> BoxedFuture<'a, Result<SerializableResource, ()>> {
		Box::pin(WriteStorageBackend::store(self, resource, data))
	}

	fn store_in<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
		allocator: &'a dyn std::alloc::Allocator,
	) -> BoxedFuture<'a, Result<SerializableResource, ()>> {
		Box::pin(WriteStorageBackend::store_in(self, resource, data, allocator))
	}

	#[cfg(debug_assertions)]
	fn replace_trace(&self, id: ResourceId<'_>, items: &[crate::ResourceTraceItem]) -> Result<(), String> {
		self.replace_trace(id, items)
	}

	fn start(&self, id: ResourceId<'_>) {
		self.start(id)
	}
}

impl<T: ReadStorageBackend + WriteStorageBackend> DynStorageBackend for T {}

#[cfg(test)]
pub mod tests {
	use std::sync::Arc;

	use gxhash::HashMapExt;
	use utils::{hash::HashMap, sync::Mutex};

	use super::*;
	use crate::resource::resource_handler::tests::MemoryResourceReader;

	/// The `TestStorageBackend` struct keeps baked resources and development traces in memory for focused tests.
	#[derive(Clone)]
	pub struct TestStorageBackend {
		resources: Arc<Mutex<HashMap<String, (Box<[u8]>, Box<[u8]>)>>>,
		#[cfg(debug_assertions)]
		traces: Arc<Mutex<HashMap<String, Vec<crate::ResourceTraceItem>>>>,
	}

	impl TestStorageBackend {
		pub fn new() -> Self {
			Self {
				resources: Arc::new(Mutex::new(HashMap::new())),
				#[cfg(debug_assertions)]
				traces: Arc::new(Mutex::new(HashMap::new())),
			}
		}

		pub fn get_resources(&self) -> Vec<ProcessedAsset> {
			self.resources
				.lock()
				.iter()
				.map(|x| {
					let resource: SerializableResource = crate::from_slice(&x.1 .0).unwrap();
					ProcessedAsset {
						id: resource.id,
						class: resource.class,
						asset_dependencies: resource.asset_dependencies,
						resource: resource.resource,
						streams: resource.streams,
						queryable_properties: resource.queryable_properties,
					}
				})
				.collect()
		}

		pub fn get_resource(&self, name: ResourceId<'_>) -> Option<ProcessedAsset> {
			self.resources
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
						asset_dependencies: resource.asset_dependencies,
						resource: resource.resource,
						streams: resource.streams,
						queryable_properties: resource.queryable_properties,
					}
				})
		}

		pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
			Some(
				self.resources
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
		fn list(&self) -> impl std::future::Future<Output = Result<Vec<String>, String>> {
			crate::r#async::future(async { Ok(self.resources.lock().keys().map(|x| x.to_string()).collect()) })
		}

		fn read<'a>(
			&'a self,
			id: ResourceId<'a>,
		) -> impl std::future::Future<Output = Option<(SerializableResource, MultiResourceReader)>> + 'a {
			crate::r#async::future(async move {
				let (resource, data) = if let Some(e) = self.resources.lock().get(id.as_ref()) {
					(e.0.clone(), e.1.clone())
				} else {
					return None;
				};

				let _ = id.get_base().to_string();

				let resource: SerializableResource = crate::from_slice(&resource).unwrap();

				let resource_reader: MultiResourceReader = Box::new(MemoryResourceReader::new(data));

				Some((resource, resource_reader))
			})
		}

		fn query(
			&self,
			_: Query,
		) -> impl std::future::Future<Output = Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>> {
			crate::r#async::future(async { Err(QueryError::StorageFailure) })
		}

		#[cfg(debug_assertions)]
		fn read_trace<'a>(
			&'a self,
			id: ResourceId<'a>,
		) -> impl std::future::Future<Output = Result<Vec<crate::ResourceTraceItem>, String>> + 'a {
			crate::r#async::future(async move { Ok(self.traces.lock().get(id.as_ref()).cloned().unwrap_or_default()) })
		}
	}

	impl WriteStorageBackend for TestStorageBackend {
		fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
			self.resources.lock().remove(id.as_ref());
			#[cfg(debug_assertions)]
			self.traces.lock().remove(id.as_ref());
			Ok(())
		}

		fn begin_resource<'a>(
			&'a self,
			id: ResourceId<'_>,
			size: usize,
		) -> impl Future<Output = Result<ResourceTransaction<'a>, ()>> + 'a {
			let resource_id = crate::resource::ResourceId::from(id.as_ref());
			async move {
				let writer = ResourceWriter::memory(size)?;
				Ok(ResourceTransaction::new(self, resource_id, None, writer))
			}
		}

		#[cfg(debug_assertions)]
		fn replace_trace(&self, id: ResourceId<'_>, items: &[crate::ResourceTraceItem]) -> Result<(), String> {
			let mut traces = self.traces.lock();
			if items.is_empty() {
				traces.remove(id.as_ref());
			} else {
				traces.insert(id.to_string(), items.to_vec());
			}
			Ok(())
		}

		fn sync<'s, 'a, T: ReadStorageBackend>(&'s self, _: &'a T) -> () {
			{}
		}
	}

	impl ResourceTransactionCommit for TestStorageBackend {
		fn commit_resource(
			&self,
			_resource_id: crate::resource::ResourceId,
			_backend_offset: Option<u64>,
			resource: ProcessedAsset,
			output: ResourceWriteOutput,
			allocator: &dyn std::alloc::Allocator,
		) -> Result<SerializableResource, ()> {
			let id = resource.id().to_string();
			let hash = output.hash();
			let size = output.size();
			let data = output.into_memory()?;
			let container = resource.into_serializable(hash, size);
			let serialized_container = crate::to_vec_in(&container, allocator).map_err(|_| ())?;

			self.resources
				.lock()
				.insert(id, (serialized_container.to_vec().into(), data.into_boxed_slice()));

			Ok(container)
		}
	}

	impl StorageBackend for TestStorageBackend {}
}

use std::future::Future;

use super::resource_handler::MultiResourceReader;
use crate::{
	asset::ResourceId, model::ArchivedQueryableValue, ArchivedSerializableResource, ProcessedAsset, SerializableResource,
};
use crate::{r#async::BoxedFuture, QueryableValue};
