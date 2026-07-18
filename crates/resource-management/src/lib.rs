//! Resource manager module.
//! Handles loading assets or resources from different origins (network, local, etc.).
//! It also handles caching of resources.

#![feature(stmt_expr_attributes)]
#![feature(future_join)]
#![feature(portable_simd)]
#![feature(allocator_api)]
// Existing resource code favors explicit asset pipeline APIs and allocator-aware buffers; these lint classes are tracked as style debt, not warning output.
#![allow(
	clippy::await_holding_lock,
	clippy::bool_assert_comparison,
	clippy::duplicate_mod,
	clippy::extra_unused_lifetimes,
	clippy::format_in_format_args,
	clippy::identity_op,
	clippy::if_same_then_else,
	clippy::items_after_test_module,
	clippy::needless_borrow,
	clippy::needless_lifetimes,
	clippy::needless_range_loop,
	clippy::new_without_default,
	clippy::mutable_key_type,
	clippy::question_mark,
	clippy::result_unit_err,
	clippy::tabs_in_doc_comments,
	clippy::to_string_trait_impl,
	clippy::too_many_arguments,
	clippy::type_complexity,
	clippy::unused_unit,
	clippy::wrong_self_convention,
	unused_imports
)]

use std::{alloc::Allocator, any::Any};

use asset::ResourceId;

pub mod asset;
pub mod resource;

pub mod model;
pub mod reference;
pub mod solver;
pub mod stream;

pub mod types;

pub mod resources;

pub mod shader;

pub mod pbr;

pub mod processors;

pub mod inspect;

pub mod r#async;

pub use asset::asset_handler::{AssetHandler, BakeContext};
pub use model::Model;
pub use model::{QueryableProperty, QueryableValue};
pub use reference::Reference;
pub use reference::ReferenceModel;
pub use resource::resource_manager::ResourceManager;
pub use resource::Resource;
pub use solver::Solver;
pub use stream::Stream;

pub(crate) type DataStorage = Vec<u8>;

pub type ResourceArchiveError = rkyv::rancor::Error;

/// The `ResourceArchive` trait marks values that can live in the engine resource archive format.
pub trait ResourceArchive: Sized + rkyv::Archive + for<'a> rkyv::Serialize<ResourceHighSerializer<'a>> {}

impl<T> ResourceArchive for T where T: rkyv::Archive + for<'a> rkyv::Serialize<ResourceHighSerializer<'a>> {}

type ResourceHighSerializer<'a> =
	rkyv::api::high::HighSerializer<rkyv::util::AlignedVec, rkyv::ser::allocator::ArenaHandle<'a>, ResourceArchiveError>;
pub(crate) type ResourceHighDeserializer = rkyv::api::high::HighDeserializer<ResourceArchiveError>;
pub(crate) type ResourceHighValidator<'a> = rkyv::api::high::HighValidator<'a, ResourceArchiveError>;

/// Serializes a resource archive value into bytes for storage.
pub(crate) fn to_vec<T: ResourceArchive>(value: &T) -> Result<Vec<u8>, ResourceArchiveError> {
	rkyv::to_bytes::<ResourceArchiveError>(value).map(Vec::from)
}

/// Serializes a resource archive value, then moves bytes into the provided allocator.
pub(crate) fn to_vec_in<'a, T: ResourceArchive>(
	value: &T,
	allocator: &'a dyn Allocator,
) -> Result<Vec<u8, &'a dyn Allocator>, ResourceArchiveError> {
	let bytes = rkyv::to_bytes::<ResourceArchiveError>(value)?;
	let mut output = Vec::with_capacity_in(bytes.len(), allocator);
	output.extend_from_slice(&bytes);
	Ok(output)
}

/// Deserializes a resource archive value into an owned Rust value.
pub(crate) fn from_slice<T>(bytes: &[u8]) -> Result<T, ResourceArchiveError>
where
	T: ResourceArchive,
	<T as rkyv::Archive>::Archived:
		for<'a> rkyv::bytecheck::CheckBytes<ResourceHighValidator<'a>> + rkyv::Deserialize<T, ResourceHighDeserializer>,
{
	rkyv::from_bytes::<T, ResourceArchiveError>(bytes)
}

/// Borrows a validated archived resource value directly from storage bytes.
pub(crate) fn archived_from_slice<T>(bytes: &[u8]) -> Result<&<T as rkyv::Archive>::Archived, ResourceArchiveError>
where
	T: ResourceArchive,
	<T as rkyv::Archive>::Archived: for<'a> rkyv::bytecheck::CheckBytes<ResourceHighValidator<'a>>,
{
	rkyv::access::<<T as rkyv::Archive>::Archived, ResourceArchiveError>(bytes)
}

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
	queryable_properties: Vec<QueryableProperty>,
}

impl ProcessedAsset {
	pub fn new<T: Model>(id: ResourceId<'_>, resource: T) -> Self {
		ProcessedAsset {
			id: id.to_string(),
			class: T::get_class().to_string(),
			resource: to_vec(&resource).unwrap(),
			streams: None,
			queryable_properties: resource.queryable_properties(id.as_ref()),
		}
	}

	/// Moves processed metadata into a serializable resource container.
	pub fn into_serializable(self, hash: u64, size: usize) -> SerializableResource {
		SerializableResource {
			id: self.id,
			hash,
			class: self.class,
			size,
			resource: self.resource,
			streams: self.streams,
			queryable_properties: self.queryable_properties,
		}
	}

	pub fn new_with_serialized(id: &str, class: &str, resource: DataStorage) -> Self {
		ProcessedAsset {
			id: id.to_string(),
			class: class.to_string(),
			resource,
			streams: None,
			queryable_properties: vec![QueryableProperty {
				name: "name".to_string(),
				value: QueryableValue::String(id.to_string()),
			}],
		}
	}

	pub fn with_streams(mut self, streams: Vec<StreamDescription>) -> Self {
		self.streams = Some(streams);
		self
	}
}

impl<'a, T: Resource + ResourceArchive + Clone> From<Reference<T>> for ProcessedAsset {
	fn from(value: Reference<T>) -> Self {
		let id = value.id.clone();
		let queryable_properties = value.resource.queryable_properties(&id);

		ProcessedAsset {
			id,
			class: value.resource.get_class().to_string(),
			resource: to_vec(&value.resource).unwrap(),
			streams: None,
			queryable_properties,
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
			queryable_properties: value.queryable_properties,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
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

	pub fn name(&self) -> &str {
		&self.name
	}

	pub fn size(&self) -> usize {
		self.size
	}

	pub fn offset(&self) -> usize {
		self.offset
	}
}

#[derive(Debug, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct SerializableResource {
	/// The resource id. This is used to identify the resource. Needs to be meaningful and will be a public constant.
	id: String,
	hash: u64,
	/// The resource class (EJ: "Texture", "Mesh", "Material", etc.)
	class: String,
	size: usize,
	resource: DataStorage,
	streams: Option<Vec<StreamDescription>>,
	queryable_properties: Vec<QueryableProperty>,
}

impl SerializableResource {
	pub fn new(
		id: String,
		hash: u64,
		class: String,
		size: usize,
		resource: DataStorage,
		streams: Option<Vec<StreamDescription>>,
		queryable_properties: Vec<QueryableProperty>,
	) -> Self {
		SerializableResource {
			id,
			hash,
			class,
			size,
			resource,
			streams,
			queryable_properties,
		}
	}

	pub fn id(&self) -> &str {
		&self.id
	}

	pub fn uid(&self) -> String {
		resource::ResourceId::from(self.id.as_str()).to_hex()
	}

	pub fn hash(&self) -> u64 {
		self.hash
	}

	pub fn class(&self) -> &str {
		&self.class
	}

	pub fn size(&self) -> usize {
		self.size
	}

	pub fn resource(&self) -> &[u8] {
		&self.resource
	}

	pub fn streams(&self) -> Option<&[StreamDescription]> {
		self.streams.as_deref()
	}

	pub fn queryable_properties(&self) -> &[QueryableProperty] {
		&self.queryable_properties
	}
}

impl<M: Model> From<SerializableResource> for ReferenceModel<M> {
	fn from(val: SerializableResource) -> Self {
		ReferenceModel::new_serialized(&val.id, val.hash, val.size, val.resource, val.streams)
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

pub trait Description: Any + Send + Sync {
	// type Resource: Resource;
	fn get_resource_class() -> &'static str
	where
		Self: Sized;
}

#[cfg(test)]
mod tests {
	/// Path to the assets folder for the tests.
	pub const ASSETS_PATH: &str = "../../assets";
}
