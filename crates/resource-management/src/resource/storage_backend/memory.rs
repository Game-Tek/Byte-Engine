//! Keep baked resources in memory so a bake can run without touching the disk.

use std::sync::Arc;

use utils::{hash::HashMap, sync::Mutex};

use super::*;
use crate::resource::reader::{ResourceReaderBacking, StoredResourceReader};

/// The `MemoryStorageBackend` struct holds baked resources and their development traces in memory, for dry-run bakes
/// and tests that must not write a resource store to disk.
///
/// Clones share the same resources, so keep one clone to read what an
/// [`AssetManager`](crate::asset::manager::AssetManager) baked into another. Read them back with
/// [`ReadStorageBackend::list`] and [`ReadStorageBackend::read`], then turn them into JSON with
/// [`crate::inspect::inspect_resource`].
#[derive(Clone)]
pub struct MemoryStorageBackend {
	/// Serialized resource metadata and stored payload bytes by resource ID.
	resources: Arc<Mutex<HashMap<String, (Box<[u8]>, Box<[u8]>)>>>,
	#[cfg(debug_assertions)]
	traces: Arc<Mutex<HashMap<String, Vec<crate::ResourceTraceItem>>>>,
}

impl MemoryStorageBackend {
	/// Creates an empty store. Pass it, or a clone of it, to
	/// [`AssetManager::new`](crate::asset::manager::AssetManager::new) as the resource storage backend.
	pub fn new() -> Self {
		Self {
			resources: Arc::new(Mutex::new(HashMap::default())),
			#[cfg(debug_assertions)]
			traces: Arc::new(Mutex::new(HashMap::default())),
		}
	}
}

impl Default for MemoryStorageBackend {
	fn default() -> Self {
		Self::new()
	}
}

/// Test helpers that read stored resources without going through the asynchronous reader.
#[cfg(test)]
impl MemoryStorageBackend {
	pub fn get_resources(&self) -> Vec<ProcessedAsset> {
		self.resources
			.lock()
			.iter()
			.map(|x| {
				let resource: SerializableResource = crate::from_slice(&x.1.0).unwrap();
				ProcessedAsset {
					id: resource.id,
					class: resource.class,
					asset_dependencies: resource.asset_dependencies,
					resource: resource.resource,
					streams: resource.streams,
					queryable_properties: resource.queryable_properties,
					compression: crate::resource::ResourceCompressionPolicy::Enabled,
				}
			})
			.collect()
	}

	pub fn get_resource(&self, name: ResourceId<'_>) -> Option<ProcessedAsset> {
		self.resources
			.lock()
			.iter()
			.find(|x| {
				let resource: SerializableResource = crate::from_slice(&x.1.0).unwrap();
				resource.id == name.as_ref()
			})
			.map(|x| {
				let resource: SerializableResource = crate::from_slice(&x.1.0).unwrap();
				ProcessedAsset {
					id: resource.id,
					class: resource.class,
					asset_dependencies: resource.asset_dependencies,
					resource: resource.resource,
					streams: resource.streams,
					queryable_properties: resource.queryable_properties,
					compression: crate::resource::ResourceCompressionPolicy::Enabled,
				}
			})
	}

	pub fn get_resource_data_by_name(&self, name: ResourceId<'_>) -> Option<Box<[u8]>> {
		let resources = self.resources.lock();
		let (serialized, stored) = resources.iter().find_map(|(_, value)| {
			let resource: SerializableResource = crate::from_slice(&value.0).ok()?;
			(resource.id() == name.as_ref()).then_some((resource, &value.1))
		})?;

		match serialized.encoding() {
			crate::resource::ResourcePayloadEncoding::Raw => Some(stored.clone()),
			crate::resource::ResourcePayloadEncoding::CpuLz4 => {
				let mut decoded = vec![0; serialized.size()].into_boxed_slice();
				crate::resource::compression::decompress_into(stored, &mut decoded).ok()?;
				Some(decoded)
			}
			crate::resource::ResourcePayloadEncoding::MetalIoLz4 => None,
		}
	}
}

impl ReadStorageBackend for MemoryStorageBackend {
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

			let resource_reader: MultiResourceReader = Box::new(StoredResourceReader::new(
				ResourceReaderBacking::Buffer(data),
				resource.encoding(),
				resource.size(),
			));

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

impl WriteStorageBackend for MemoryStorageBackend {
	async fn clear(&self) -> Result<(), String> {
		self.resources.lock().clear();
		#[cfg(debug_assertions)]
		self.traces.lock().clear();
		Ok(())
	}

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
}

impl ResourceTransactionCommit for MemoryStorageBackend {
	fn commit_resource(
		&self,
		_resource_id: crate::resource::ResourceId,
		_backend_reservation: Option<super::transaction::ResourceReservation>,
		resource: ProcessedAsset,
		output: ResourceWriteOutput,
		allocator: &dyn std::alloc::Allocator,
	) -> Result<SerializableResource, ()> {
		let id = resource.id().to_string();
		let hash = output.hash();
		let size = output.size();
		let stored_size = output.stored_size();
		let encoding = output.encoding();
		let data = output.into_memory()?;
		let container = resource.into_serializable(hash, size, stored_size, encoding);
		let serialized_container = crate::to_vec_in(&container, allocator).map_err(|_| ())?;

		self.resources
			.lock()
			.insert(id, (serialized_container.to_vec().into(), data.into_boxed_slice()));

		Ok(container)
	}
}

impl StorageBackend for MemoryStorageBackend {}
