//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(closure_lifetime_binder)]
#![feature(stmt_expr_attributes)]
#![feature(path_file_prefix, path_add_extension)]
#![feature(map_try_insert)]
#![feature(future_join)]
#![feature(once_cell_try)]

use std::{any::Any, collections::{HashMap}, hash::Hasher, sync::{Arc, Mutex}};
use base64::Engine;
use redb::ReadableTable;
use serde::{ser::SerializeStruct, Deserialize, Serialize};

use resource::{resource_handler::{FileResourceReader, LoadTargets, ReadTargets, ResourceReader}, storage_backend::ReadStorageBackend};
use asset::{get_base, read_asset_from_source, BEADType, ResourceId};
use utils::{json, sync::File};

pub mod asset;
pub mod resource;

pub mod types;

pub mod audio;
pub mod image;
pub mod material;
pub mod mesh;

pub mod file_tracker;

pub mod shader_generator;

pub mod glsl_shader_generator;
pub mod spirv_shader_generator;

pub mod glsl;

pub use resource::resource_manager::ResourceManager;
pub use asset::asset_handler::AssetHandler;

pub(crate) type Data = pot::Value<'static>;
pub(crate) type DataStorage = Vec<u8>;
pub(crate) use pot::from_slice;

// https://www.yosoygames.com.ar/wp/2018/03/vertex-formats-part-1-compression/

/// This is the struct resource handlers should return when processing a resource.
#[derive(Debug, Clone)]
pub struct ProcessedAsset {
    /// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
    id: String,
    /// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
    class: String,
    /// List of resources that this resource depends on.
    // required_resources: Vec<ProcessedResources>,
    /// The resource data.
    // resource: Data,
	resource: DataStorage,
    streams: Option<Vec<StreamDescription>>,
}

impl ProcessedAsset {
    pub fn new<T: Model + serde::Serialize>(id: ResourceId<'_>, resource: T) -> Self {
        ProcessedAsset {
            id: id.to_string(),
            class: T::get_class().to_string(),
            resource: pot::to_vec(&resource).unwrap(),
            streams: None,
        }
    }

    pub fn new_with_serialized(id: &str, class: &str, resource: DataStorage) -> Self {
        ProcessedAsset {
            id: id.to_string(),
            class: class.to_string(),
            resource,
            streams: None,
        }
    }

    pub fn with_streams(mut self, streams: Vec<StreamDescription>) -> Self {
        self.streams = Some(streams);
        self
    }
}

impl<'a, T: Resource + Serialize + Clone> From<Reference<T>> for ProcessedAsset {
    fn from(value: Reference<T>) -> Self {
        ProcessedAsset {
            id: value.id,
            class: value.resource.get_class().to_string(),
            resource: pot::to_vec(&value.resource).unwrap(),
            streams: None,
        }
    }
}

impl From<SerializableResource> for ProcessedAsset {
    fn from(value: SerializableResource) -> Self {
        ProcessedAsset {
            id: value.id,
            class: value.class,
            resource: value.resource.clone(),
            streams: None,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StreamDescription {
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
    name: String,
    /// The subresource size.
    size: usize,
    /// The subresource offset.
    offset: usize,
}

impl StreamDescription {
    pub fn new(name: &str, size: usize, offset: usize) -> Self {
        StreamDescription {
            name: name.to_string(),
            size,
            offset,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SerializableResource {
    /// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
    id: String,
    hash: u64,
    /// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
    class: String,
    size: usize,
	resource: DataStorage,
	streams: Option<Vec<StreamDescription>>,
}

impl SerializableResource {
    pub fn new(id: String, hash: u64, class: String, size: usize, resource: DataStorage, streams: Option<Vec<StreamDescription>>) -> Self {
        SerializableResource {
            id,
            hash,
            class,
            size,
            resource,
			streams,
        }
    }
}

impl <M: Model> Into<ReferenceModel<M>> for SerializableResource {
	fn into(self) -> ReferenceModel<M> {
		ReferenceModel::new_serialized(&self.id, self.hash, self.size, self.resource, self.streams)
	}
}

pub trait Model: for<'de> serde::Deserialize<'de> {
    fn get_class() -> &'static str;
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ReferenceModel<T: Model> {
    pub id: String,
    hash: u64,
	size: usize,
    class: String,
    resource: DataStorage,
    #[serde(skip)]
    phantom: std::marker::PhantomData<T>,
	streams: Option<Vec<StreamDescription>>,
}

impl<T: Model> ReferenceModel<T> {
    pub fn new(id: &str, hash: u64, size: usize, resource: &T, streams: Option<Vec<StreamDescription>>) -> Self where T: serde::Serialize {
        ReferenceModel {
            id: id.to_string(),
            hash,
			size,
            class: T::get_class().to_string(),
            resource: pot::to_vec(resource).unwrap(),
            phantom: std::marker::PhantomData,
			streams,
        }
    }

    pub fn new_serialized(id: &str, hash: u64, size: usize, resource: DataStorage, streams: Option<Vec<StreamDescription>>) -> Self {
        ReferenceModel {
            id: id.to_string(),
            hash,
			size,
            class: T::get_class().to_string(),
            resource,
            phantom: std::marker::PhantomData,
			streams,
        }
    }

	pub fn id(&self) -> ResourceId {
		ResourceId::new(&self.id)
	}
}

#[derive(Debug)]
/// Represents a resource reference and can be use to embed resources in other resources.
pub struct Reference<T: Resource> {
    pub id: String,
    pub hash: u64,
    pub size: usize,
    pub resource: T,
    reader: Option<FileResourceReader>,
	streams: Option<Vec<StreamDescription>>,
}

impl<'a, T: Resource + 'a> Serialize for Reference<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("TypedDocument", 3)?;

        state.serialize_field("id", &self.id)?;
        state.serialize_field("hash", &self.hash)?;
        state.serialize_field("class", &self.resource.get_class())?;

        state.end()
    }
}

impl<'a, T: Resource + 'a> Reference<T> {
	pub fn from_model(model: ReferenceModel<T::Model>, resource: T, reader: FileResourceReader) -> Self {
		Reference {
			id: model.id,
			hash: model.hash,
			size: model.size,
			resource,
			reader: Some(reader),
			streams: model.streams,
		}
	}

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn hash(&self) -> u64 {
        self.hash
    }

	pub fn get_hash(&self) -> u64 { self.hash }

    pub fn resource(&self) -> &T {
        &self.resource
    }

    pub fn resource_mut(&mut self) -> &mut T {
        &mut self.resource
    }

    pub fn into_resource(self) -> T {
        self.resource
    }

    pub fn consume_reader(&mut self) -> FileResourceReader {
        self.reader.take().unwrap()
    }

    pub fn map(self, f: impl FnOnce(T) -> T) -> Self {
        Reference {
            resource: f(self.resource),
            ..self
        }
    }

	/// Loads the resource's binary data into memory from the storage backend.
	pub fn load<'s>(&'s mut self, read_target: ReadTargets<'a>) -> Result<LoadTargets<'a>, LoadResults> {
		let reader = self.reader.take().ok_or(LoadResults::NoReadTarget)?;
		reader.read_into(self.streams.as_ref().map(|s| s.as_slice()), read_target).map_err(|_| LoadResults::LoadFailed)
	}
}

#[derive(Debug)]
pub struct Stream<'a> {
    /// The slice of the buffer to load the resource binary data into.
    buffer: &'a [u8],
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
    name: &'a str,
}

impl<'a> Stream<'a> {
    pub fn new(name: &'a str, buffer: &'a [u8]) -> Self {
        Stream { buffer, name }
    }

    pub fn buffer(&'a self) -> &'a [u8] {
        self.buffer
    }
}

impl<'a> From<StreamMut<'a>> for Stream<'a> {
    fn from(value: StreamMut<'a>) -> Self {
        Stream::new(value.name, value.buffer)
    }
}

#[derive(Debug)]
pub struct StreamMut<'a> {
    /// The slice of the buffer to load the resource binary data into.
    buffer: &'a mut [u8],
    /// The subresource tag. This is used to identify the subresource. (EJ: "Vertex", "Index", etc.)
    name: &'a str,
}

impl<'a> StreamMut<'a> {
    pub fn new<T: Copy>(name: &'a str, buffer: &'a mut [T]) -> Self {
		let buffer = unsafe {
			std::slice::from_raw_parts_mut(buffer.as_mut_ptr() as *mut u8, std::mem::size_of::<T>() * buffer.len())
		};
        StreamMut { buffer, name }
    }

    pub fn buffer(&'a self) -> &'a [u8] {
        self.buffer
    }

    pub fn buffer_mut(&'a mut self) -> &'a mut [u8] {
        self.buffer
    }
}

/// Enumaration for all the possible results of a resource load fails.
#[derive(Debug)]
pub enum LoadResults {
    /// No resource could be resolved for the given path.
    ResourceNotFound,
    /// The resource could not be loaded.
    LoadFailed,
    /// The resource could not be found in cache.
    CacheFileNotFound,
    /// The resource type is not supported.
    UnsuportedResourceType,
    /// No read target was set for the resource.
    NoReadTarget,
}

/// Trait that defines a resource.
pub trait Resource: Send + Sync {
    /// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
    /// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
    /// Is needed by the deserialize function.
    fn get_class(&self) -> &'static str;

    type Model: Model;
}

impl <T: Resource> std::hash::Hash for Reference<T> {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state); self.hash.hash(state); self.size.hash(state); self.resource.get_class().hash(state);
	}
}

/// Options for loading a resource.
#[derive(Debug)]
pub struct OptionResource<'a> {
    /// The resource to apply this option to.
    pub url: String,
    /// The buffers to load the resource binary data into.
    pub streams: Vec<StreamMut<'a>>,
}

/// Represents the options for performing a bundled/batch resource load.
pub struct Options<'a> {
    pub resources: Vec<OptionResource<'a>>,
}

pub trait CreateResource: downcast_rs::Downcast + Send + Sync {}

downcast_rs::impl_downcast!(CreateResource);

pub struct CreateInfo<'a> {
    pub name: &'a str,
    pub info: Box<dyn CreateResource>,
    pub data: &'a [u8],
}

#[derive(Debug)]
enum SolveErrors {
    DeserializationFailed(String),
    StorageError,
}

/// The solver trait provides methods to solve a resource.
/// This is used to load resources from the storage backend.
pub trait Solver<'de, T>
where
    Self: serde::Deserialize<'de>,
{
    fn solve(self, storage_backend: &dyn ReadStorageBackend) -> Result<T, SolveErrors>;
}

// #[cfg(test)]
// impl StorageBackend for DbStorageBackend {
// 	fn list<'a>(&'a self) -> utils::BoxedFuture<'a, Result<Vec<String>, String>> {
// 		let resources = self.resources.lock().unwrap();
// 		let mut names = Vec::with_capacity(resources.len());
// 		for resource in resources.iter() {
// 			names.push(resource.0.id.clone());
// 		}

// 		Box::pin(async move {
// 			Ok(names)
// 		})
// 	}

// 	fn delete<'a>(&'a self, id: &'a str) -> utils::BoxedFuture<'a, Result<(), String>> {
// 		let mut resources = self.resources.lock().unwrap();
// 		let mut index = None;
// 		for (i, resource) in resources.iter().enumerate() {
// 			if resource.0.id == id {
// 				index = Some(i);
// 				break;
// 			}
// 		}

// 		if let Some(i) = index {
// 			resources.remove(i);
// 			Box::pin(async move {
// 				Ok(())
// 			})
// 		} else {
// 			Box::pin(async move {
// 				Err("Resource not found".to_string())
// 			})
// 		}
// 	}

// 	fn store<'a, 'b: 'a>(&'a self, resource: &'b ProcessedAsset, data: &[u8]) -> utils::SendSyncBoxedFuture<'a, Result<GenericResourceResponse, ()>> {
// 		self.resources.lock().unwrap().push((resource.clone(), data.into()));

// 		let id = resource.id.clone();
// 		let class = resource.class.clone();
// 		let size = data.len();
// 		let streams = resource.streams.clone();
// 		let resource = resource.resource.clone();

// 		Box::pin(async move {
// 			Ok(GenericResourceResponse::new(id, 0, class, size, resource, streams))
// 		})
// 	}

// 	fn read<'s, 'a, 'b>(&'s self, id: ResourceId<'b>) -> utils::SendSyncBoxedFuture<'a, Option<(GenericResourceResponse, FileResourceReader)>> {
// 		let mut x = None;

// 		let resources = self.resources.lock().unwrap();
// 		for (resource, data) in resources.iter() {
// 			if resource.id == id.as_ref() {
// 				// TODO: use actual hash
// 				x = Some((GenericResourceResponse::new(id.to_string(), 0, resource.class.clone(), data.len(), resource.resource.clone(), resource.streams.clone()), FileResourceReader::new(data.clone())));
// 				break;
// 			}
// 		}

// 		Box::pin(async move {
// 			x
// 		})
// 	}

// 	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> utils::SendSyncBoxedFuture<'a, Result<(Box<[u8]>, Option<BEADType>, String), ()>> { Box::pin(async move {
// 		// All of this weirdness is to avoid Send + Sync errors because of the locks

// 		let r = {
// 			let files = self.files.lock().unwrap();
// 			if let Some(f) = files.get(url.as_ref()) {
// 				let bead = {
// 					let mut url = url.get_base().to_string();
// 					url.push_str(".bead");
// 					if let Some(spec) = files.get(url.as_str()) {
// 						Some(json::from_str(std::str::from_utf8(spec).unwrap()).unwrap())
// 					} else {
// 						None
// 					}
// 				};

// 				// Extract extension from url
// 				Ok((f.clone(), bead, url.get_extension().to_string()))
// 			} else {
// 				Err(())
// 			}
// 		};

// 		if let Err(_) = r {
// 			let bead = {
// 				let mut url = url.get_base().to_string();
// 				url.push_str(".bead");
// 				if let Some(spec) = self.files.lock().unwrap().get(url.as_str()) {
// 					Some(json::from_str(std::str::from_utf8(spec).unwrap()).unwrap())
// 				} else {
// 					None
// 				}
// 			};

// 			if let Ok(x) = read_asset_from_source(url, Some(&std::path::Path::new("../assets"))).await {
// 				let bead = bead.or(x.1);
// 				Ok((x.0, bead, x.2))
// 			} else {
// 				Err(())
// 			}
// 		} else {
// 			r
// 		}
// 	}) }
// }

pub trait Description: Any + Send + Sync {
    // type Resource: Resource;
    fn get_resource_class() -> &'static str
    where
        Self: Sized;
}

#[cfg(test)]
mod tests {
    use futures::future::try_join_all;
    use serde::Deserialize;

    use crate::{ProcessedAsset, Model, Reference,ReferenceModel, Resource, SolveErrors, Solver,};

    // #[test]
    // fn solve_resources() {
    // 	#[derive(serde::Serialize)]
    // 	struct Base<'a> {
    // 		items: Vec<Reference<'a, Item>>,
    // 	}

    // 	#[derive(serde::Deserialize)]
    // 	struct BaseModel {
    // 		items: Vec<ReferenceModel<Item>>,
    // 	}

    // 	#[derive(serde::Serialize, serde::Deserialize)]
    // 	struct Item {
    // 		property: String,
    // 	}

    // 	#[derive(serde::Serialize,)]
    // 	struct Variant<'a> {
    // 		parent: Reference<'a, Base<'a>>
    // 	}

    // 	#[derive(serde::Deserialize)]
    // 	struct VariantModel {
    // 		parent: ReferenceModel<BaseModel>
    // 	}

    // 	impl <'a> Resource for Base<'a> {
    // 		fn get_class(&self) -> &'static str { "Base" }

    // 		type Model = BaseModel;
    // 	}

    // 	impl Model for BaseModel {
    // 		fn get_class() -> &'static str { "Base" }
    // 	}

    // 	impl Resource for Item {
    // 		fn get_class(&self) -> &'static str { "Item" }

    // 		type Model = Item;
    // 	}

    // 	impl Model for Item {
    // 		fn get_class() -> &'static str { "Item" }
    // 	}

    // 	impl <'a> Resource for Variant<'a> {
    // 		fn get_class(&self) -> &'static str { "Variant" }

    // 		type Model = VariantModel;
    // 	}

    // 	impl Model for VariantModel {
    // 		fn get_class() -> &'static str { "Variant" }
    // 	}

    // 	let storage_backend = TestStorageBackend::new();

    // 	smol::block_on(storage_backend.store(&GenericResourceSerialization::new("item", Item{ property: "hello".to_string() }), &[])).unwrap();
    // 	smol::block_on(storage_backend.store(&GenericResourceSerialization::new("base", Base{ items: vec![Reference::new("item", 0, 0, Item{ property: "hello".to_string() })] }), &[])).unwrap();
    // 	smol::block_on(storage_backend.store(&GenericResourceSerialization::new("variant", Variant{ parent: Reference::new("base", 0, 0, Base{ items: vec![Reference::new("item", 0, 0, Item{ property: "hello".to_string() })] }) }), &[])).unwrap();

    // 	impl <'a, 'de> Solver<'de, Reference<'a, Base<'a>>> for ReferenceModel<BaseModel> {
    // 		async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Base<'a>>, SolveErrors> {
    // 			let (gr, reader) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
    // 			let resource = BaseModel::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;
    // 			let base = Base{
    // 				items: try_join_all(resource.items.into_iter().map(|item| {
    // 					item.solve(storage_backend)
    // 				})).await.map_err(|_| SolveErrors::StorageError)?
    // 			};
    // 			Ok(Reference::new(&gr.id, 0, 0, base, reader))
    // 		}
    // 	}

    // 	impl <'a, 'de> Solver<'de, Reference<'a, Item>> for ReferenceModel<Item> {
    // 		async fn solve(self, storage_backend: &dyn StorageBackend) -> Result<Reference<'a, Item>, SolveErrors> {
    // 			let (gr, reader) = smol::block_on(storage_backend.read(&self.id)).ok_or_else(|| SolveErrors::StorageError)?;
    // 			let item = Item::deserialize(bson::Deserializer::new(gr.resource.clone().into())).map_err(|e| SolveErrors::DeserializationFailed(e.to_string()))?;
    // 			Ok(Reference::new(&gr.id, 0, 0, item, reader))
    // 		}
    // 	}

    // 	let base: ReferenceModel<BaseModel> = smol::block_on(storage_backend.read("base")).unwrap().0.into();
    // 	let base = base.solve(&storage_backend);
    // }
}
