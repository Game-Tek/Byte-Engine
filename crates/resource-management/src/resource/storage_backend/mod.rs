//! Store, retrieve, and query baked resources through interchangeable backends.

mod memory;
pub mod redb;
mod transaction;

pub use memory::MemoryStorageBackend;
pub use transaction::ResourceTransaction;
pub(crate) use transaction::write_complete_owned_resource;
use transaction::{ResourceTransactionCommit, ResourceWriteOutput, ResourceWriter, StagedResourceFile};

/// The `StorageBackend` trait composes typed resource reads and writes for one storage implementation.
pub trait StorageBackend: ReadStorageBackend + WriteStorageBackend {}
/// The `DynStorageBackend` trait provides object-safe resource reads and writes for runtime-selected storage.
pub trait DynStorageBackend: DynReadStorageBackend + DynWriteStorageBackend {}

/// The `ReadStorageBackend` trait provides metadata and decoded payload readers for stored resources.
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
}

/// The `DynReadStorageBackend` trait provides object-safe resource lookup for runtime-selected storage.
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
}

/// The `WriteStorageBackend` trait provides complete and incremental resource-authoring paths.
///
/// Use [`Self::store`] for complete payloads that may use CPU compression. Use
/// [`Self::begin_resource`] when a processor must write incrementally without compression.
pub trait WriteStorageBackend: Sync + Send {
	/// Removes every stored resource while preserving the backend configuration.
	///
	/// Call [`ReadStorageBackend::list`] next to verify that the store is empty.
	fn clear(&self) -> impl Future<Output = Result<(), String>>;

	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;

	/// Reserves exact payload storage before a processor starts writing.
	///
	/// This incremental path always stores bytes uncompressed. Use [`Self::store`]
	/// when the complete payload is available and may use CPU compression.
	///
	/// Await [`ResourceTransaction::write_all`] until exactly `size` bytes have
	/// been accepted, then await [`ResourceTransaction::commit`].
	fn begin_resource<'a>(
		&'a self,
		id: ResourceId<'_>,
		size: usize,
	) -> impl Future<Output = Result<ResourceTransaction<'a>, ()>> + 'a;

	/// Returns the CPU compression policy this backend applies to one complete resource.
	///
	/// Native GPU encodings override this hook so their payload remains entirely outside the CPU compression path.
	fn cpu_compression_policy(&self, resource: &ProcessedAsset) -> crate::resource::ResourceCompressionPolicy {
		resource.compression_policy()
	}

	fn store<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		self.store_in(resource, data, &std::alloc::Global)
	}

	fn store_in<'a>(
		&'a self,
		resource: ProcessedAsset,
		data: &'a [u8],
		allocator: &'a dyn std::alloc::Allocator,
	) -> impl Future<Output = Result<SerializableResource, ()>> + 'a {
		async move {
			let prepared = crate::resource::compression::prepare(data, self.cpu_compression_policy(&resource));
			let transaction = if let Some(prepared) = prepared {
				let mut transaction = self
					.begin_resource(ResourceId::new(resource.id()), prepared.bytes.len())
					.await?
					.with_compressed_payload(prepared.decoded_size, prepared.decoded_hash);
				let compio::buf::BufResult(result, _) =
					compio::io::AsyncWriteExt::write_all(&mut transaction, prepared.bytes).await;
				result.map_err(|_| ())?;
				transaction
			} else {
				let mut transaction = self.begin_resource(ResourceId::new(resource.id()), data.len()).await?;
				transaction.write_all(data).await.map_err(|_| ())?;
				transaction
			};
			transaction.commit(resource, allocator).await
		}
	}

	/// Replaces development-time bake messages without creating a resource entry.
	#[cfg(debug_assertions)]
	fn replace_trace(&self, _: ResourceId<'_>, _: &[crate::ResourceTraceItem]) -> Result<(), String> {
		Ok(())
	}

	/// Makes every resource committed so far survive a crash or power loss.
	///
	/// Backends may publish individual resources without waiting for the disk so a bake with many outputs does not
	/// flush once per resource. Call this once a batch of writes is complete, such as when a root bake finishes.
	fn persist(&self) -> Result<(), String> {
		Ok(())
	}
}

/// The `DynWriteStorageBackend` trait provides object-safe resource authoring for runtime-selected storage.
pub trait DynWriteStorageBackend: Send + Sync {
	fn clear(&self) -> BoxedFuture<'_, Result<(), String>>;
	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String>;
	fn begin_resource<'a>(&'a self, id: ResourceId<'_>, size: usize) -> BoxedFuture<'a, Result<ResourceTransaction<'a>, ()>>;
	fn cpu_compression_policy(&self, resource: &ProcessedAsset) -> crate::resource::ResourceCompressionPolicy;
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
	fn persist(&self) -> Result<(), String> {
		Ok(())
	}
}

/// The `QueryCursor` struct provides an opaque continuation point for paginated resource queries.
#[derive(
	Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq, Eq, Hash, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum QueryPredicate {
	Eq { property: String, value: QueryableValue },
}

/// The `Query` struct provides a class-filtered, paginated request to a storage backend.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Query {
	pub class: String,
	pub predicates: Vec<QueryPredicate>,
	pub limit: usize,
	pub cursor: Option<QueryCursor>,
}

impl Query {
	pub fn new(class: impl Into<String>) -> Self {
		Self {
			class: class.into(),
			predicates: Vec::new(),
			limit: usize::MAX,
			cursor: None,
		}
	}

	pub fn eq(mut self, property: impl Into<String>, value: impl Into<QueryableValue>) -> Self {
		self.predicates.push(QueryPredicate::Eq {
			property: property.into(),
			value: value.into(),
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

/// The `QueryError` enum reports why a storage backend produced no metadata query page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueryError {
	InvalidCursor,
	StorageFailure,
}

impl std::fmt::Display for QueryError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			QueryError::InvalidCursor => {
				f.write_str("Failed to query resources. The most likely cause is that the provided cursor is invalid.")
			}
			QueryError::StorageFailure => f.write_str(
				"Failed to query resources. The most likely cause is that the resources database could not be read.",
			),
		}
	}
}

impl std::error::Error for QueryError {}

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
}

impl<T: WriteStorageBackend> DynWriteStorageBackend for T {
	fn clear(&self) -> BoxedFuture<'_, Result<(), String>> {
		Box::pin(self.clear())
	}

	fn delete<'a>(&'a self, id: ResourceId<'a>) -> Result<(), String> {
		self.delete(id)
	}

	fn begin_resource<'a>(&'a self, id: ResourceId<'_>, size: usize) -> BoxedFuture<'a, Result<ResourceTransaction<'a>, ()>> {
		Box::pin(WriteStorageBackend::begin_resource(self, id, size))
	}

	fn cpu_compression_policy(&self, resource: &ProcessedAsset) -> crate::resource::ResourceCompressionPolicy {
		WriteStorageBackend::cpu_compression_policy(self, resource)
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

	fn persist(&self) -> Result<(), String> {
		WriteStorageBackend::persist(self)
	}
}

impl<T: ReadStorageBackend + WriteStorageBackend> DynStorageBackend for T {}

use std::future::Future;

use super::resource_handler::MultiResourceReader;
use crate::{
	ArchivedSerializableResource, ProcessedAsset, SerializableResource, asset::ResourceId, model::ArchivedQueryableValue,
};
use crate::{QueryableValue, r#async::BoxedFuture};
