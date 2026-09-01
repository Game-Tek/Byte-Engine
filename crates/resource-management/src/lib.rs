//! Load, cache, and bake engine resources from local or remote storage.
//!
//! Start with the [resource management guide](/docs/develop/resource-management)
//! to choose between development-time asset baking and runtime resource loading.

#![feature(stmt_expr_attributes)]
#![feature(future_join)]
#![feature(portable_simd)]
#![feature(allocator_api)]
// Existing resource code favors explicit asset pipeline APIs and allocator-aware buffers; these lint classes are tracked as style debt, not warning output.
#![allow(
	clippy::await_holding_lock,
	clippy::bool_assert_comparison,
	clippy::cognitive_complexity,
	clippy::duplicate_mod,
	clippy::empty_line_after_outer_attr,
	clippy::excessive_nesting,
	clippy::extra_unused_lifetimes,
	clippy::format_in_format_args,
	clippy::identity_op,
	clippy::if_same_then_else,
	clippy::items_after_test_module,
	clippy::module_inception,
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
	clippy::too_many_lines,
	clippy::type_complexity,
	clippy::unused_unit,
	clippy::wrong_self_convention,
	unused_imports
)]

use std::{alloc::Allocator, any::Any};

use asset::ResourceId;

pub(crate) const ONLINE_DOCS_BASE_URL: &str = match option_env!("BYTE_ENGINE_DOCS_BASE_URL") {
	Some(url) => url,
	None => "https://byte-engine.0x44491229.dev/docs",
};

/// Builds a link to one online documentation page.
pub(crate) fn online_docs_url(path: &str) -> String {
	format!(
		"{}/{}",
		ONLINE_DOCS_BASE_URL.trim_end_matches('/'),
		path.trim_start_matches('/')
	)
}

pub mod asset;
pub mod resource;

pub mod model;
pub mod reference;
pub mod solver;
pub mod stream;

pub mod types;

pub mod resources;

pub mod shader;

pub mod ibl;
pub mod pbr;

pub mod processors;

pub mod inspect;

pub mod r#async;

pub use asset::handler::{AssetHandler, BakeContext};
#[cfg(debug_assertions)]
pub use asset::{ResourceTrace, ResourceTraceItem, ResourceTraceLevel};
pub use model::Model;
pub use model::{QueryableProperty, QueryableValue};
pub use reference::Reference;
pub use reference::ReferenceModel;
pub use resource::Resource;
pub use resource::resource_manager::ResourceManager;
pub use solver::Solver;
pub use stream::Stream;

pub(crate) type DataStorage = Vec<u8>;

pub type ResourceArchiveError = rkyv::rancor::Error;

/// The `ResourceArchive` trait identifies values that the engine can store in its resource archive format.
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

/// The `ProcessedAsset` struct carries a handler's baked metadata and binary data to resource storage.
#[derive(Debug, Clone)]
pub struct ProcessedAsset {
	/// The stable, public resource ID.
	id: String,
	/// The resource class, such as `Texture`, `Mesh`, or `Material`.
	class: String,
	/// Source versions that must still match before this baked resource can be reused.
	asset_dependencies: Vec<asset::storage_backend::AssetDependency>,
	/// The resources that this resource depends on.
	// required_resources: Vec<ProcessedResources>,
	/// The serialized resource metadata.
	// resource: Data,
	resource: DataStorage,
	streams: Option<Vec<StreamDescription>>,
	queryable_properties: Vec<QueryableProperty>,
	compression: resource::ResourceCompressionPolicy,
}

impl ProcessedAsset {
	/// Creates processed metadata for one runtime resource with CPU compression enabled.
	///
	/// Next, attach any named ranges with [`Self::with_streams`] or change the
	/// compression policy with [`Self::with_compression`] before passing the
	/// resource to [`WriteStorageBackend::store`](resource::WriteStorageBackend::store).
	pub fn new<T: Model>(id: ResourceId<'_>, resource: T) -> Self {
		ProcessedAsset {
			id: id.to_string(),
			class: T::get_class().to_string(),
			asset_dependencies: Vec::new(),
			resource: to_vec(&resource).unwrap(),
			streams: None,
			queryable_properties: resource.queryable_properties(id.as_ref()),
			compression: resource::ResourceCompressionPolicy::Enabled,
		}
	}

	/// Returns the stable ID that storage must publish for this resource.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Returns the resource class used by storage policy decisions.
	pub fn class(&self) -> &str {
		&self.class
	}

	pub(crate) fn serialized_metadata(&self) -> &[u8] {
		&self.resource
	}

	/// Returns whether whole-resource storage may apply CPU compression.
	pub fn compression_policy(&self) -> resource::ResourceCompressionPolicy {
		self.compression
	}

	/// Attaches the source versions observed while the asset handler produced this resource.
	pub(crate) fn with_asset_dependencies(mut self, asset_dependencies: Vec<asset::storage_backend::AssetDependency>) -> Self {
		self.asset_dependencies = asset_dependencies;

		self
	}

	/// Moves processed metadata into a serializable resource container.
	pub fn into_serializable(
		self,
		hash: u64,
		size: usize,
		stored_size: usize,
		encoding: resource::ResourcePayloadEncoding,
	) -> SerializableResource {
		SerializableResource {
			id: self.id,
			hash,
			class: self.class,
			asset_dependencies: self.asset_dependencies,
			size,
			stored_size,
			encoding,
			resource: self.resource,
			streams: self.streams,
			queryable_properties: self.queryable_properties,
		}
	}

	/// Creates processed metadata from an already serialized model with CPU compression enabled.
	///
	/// Next, pass the result and its complete binary payload to
	/// [`WriteStorageBackend::store`](resource::WriteStorageBackend::store).
	pub fn new_with_serialized(id: &str, class: &str, resource: DataStorage) -> Self {
		ProcessedAsset {
			id: id.to_string(),
			class: class.to_string(),
			asset_dependencies: Vec::new(),
			resource,
			streams: None,
			queryable_properties: vec![QueryableProperty {
				name: "name".to_string(),
				value: QueryableValue::String(id.to_string()),
			}],
			compression: resource::ResourceCompressionPolicy::Enabled,
		}
	}

	/// Attaches named decoded ranges that consumers can select from an uncompressed payload.
	///
	/// If whole-resource compression is retained, load decoded backing storage with
	/// [`Reference::load`] before selecting these ranges.
	pub fn with_streams(mut self, streams: Vec<StreamDescription>) -> Self {
		self.streams = Some(streams);

		self
	}

	/// Selects whether whole-resource store calls may apply CPU compression.
	///
	/// Compression is enabled by default. Partial [`ResourceTransaction`](resource::ResourceTransaction)
	/// writes remain uncompressed regardless of this setting.
	pub fn with_compression(mut self, compression: resource::ResourceCompressionPolicy) -> Self {
		self.compression = compression;
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
			asset_dependencies: Vec::new(),
			resource: to_vec(&value.resource).unwrap(),
			streams: None,
			queryable_properties,
			compression: resource::ResourceCompressionPolicy::Enabled,
		}
	}
}

impl From<SerializableResource> for ProcessedAsset {
	fn from(value: SerializableResource) -> Self {
		ProcessedAsset {
			id: value.id,
			class: value.class,
			asset_dependencies: value.asset_dependencies,
			resource: value.resource.clone(),
			streams: None,
			queryable_properties: value.queryable_properties,
			compression: resource::ResourceCompressionPolicy::Enabled,
		}
	}
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
/// The `StreamDescription` struct identifies one named range in decoded resource bytes.
pub struct StreamDescription {
	/// The subresource name, such as `Vertex` or `Index`.
	name: String,
	/// The subresource size.
	size: usize,
	/// The subresource offset.
	offset: usize,
}

impl StreamDescription {
	/// Creates a stream description while preserving ownership of a generated name.
	pub fn new(name: impl Into<String>, size: usize, offset: usize) -> Self {
		StreamDescription {
			name: name.into(),
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
/// The `SerializableResource` struct persists runtime metadata and the explicit encoding of its binary payload.
pub struct SerializableResource {
	/// The stable, public resource ID.
	id: String,
	hash: u64,
	/// The resource class, such as `Texture`, `Mesh`, or `Material`.
	class: String,
	asset_dependencies: Vec<asset::storage_backend::AssetDependency>,
	/// Number of bytes clients receive after CPU decompression.
	size: usize,
	/// Number of bytes occupied by the encoded payload in storage.
	stored_size: usize,
	encoding: resource::ResourcePayloadEncoding,
	resource: DataStorage,
	streams: Option<Vec<StreamDescription>>,
	queryable_properties: Vec<QueryableProperty>,
}

impl SerializableResource {
	/// Creates persisted resource metadata for an explicitly encoded payload.
	///
	/// `size` is the decoded size clients receive and `stored_size` is the physical
	/// payload extent. Next, construct a reader that applies [`Self::encoding`]
	/// before exposing the payload.
	pub fn new(
		id: String,
		hash: u64,
		class: String,
		size: usize,
		stored_size: usize,
		encoding: resource::ResourcePayloadEncoding,
		resource: DataStorage,
		streams: Option<Vec<StreamDescription>>,
		queryable_properties: Vec<QueryableProperty>,
	) -> Self {
		SerializableResource {
			id,
			hash,
			class,
			asset_dependencies: Vec::new(),
			size,
			stored_size,
			encoding,
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

	/// Returns the source versions that determine whether this resource needs to be baked again.
	pub(crate) fn asset_dependencies(&self) -> &[asset::storage_backend::AssetDependency] {
		&self.asset_dependencies
	}

	pub fn class(&self) -> &str {
		&self.class
	}

	/// Returns the decoded payload size clients must allocate before loading.
	pub fn size(&self) -> usize {
		self.size
	}

	/// Returns the physical encoded payload size used for files and packed allocation.
	pub fn stored_size(&self) -> usize {
		self.stored_size
	}

	/// Returns the explicit storage and delivery encoding for this resource payload.
	pub fn encoding(&self) -> resource::ResourcePayloadEncoding {
		self.encoding
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

/// The `LoadResults` enum identifies failures that can occur while loading a resource.
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

	/// The path to the asset fixtures used by tests.
	pub const ASSETS_PATH: &str = "../../assets";
}
