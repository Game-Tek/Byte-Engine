//! Record what a bake observed, so the next request can tell whether its resources are still current.
//!
//! A resource is current when every input its bake observed still has the fingerprint the bake recorded. The
//! [freshness reference](/docs/reference/resource-management#freshness) defines the rule; this module holds the record.

use std::hash::Hasher as _;

use super::storage_backend::AssetVersion;
use crate::{SerializableResource, resource::storage_backend::Query};

/// The `BakeInput` enum names one thing a bake observed and the fingerprint it had, so a later request can tell
/// whether it changed.
///
/// A [`BakeContext`](crate::asset::handler::BakeContext) records inputs as a handler reads sources, bakes
/// dependencies, runs queries, and reads settings. Handlers don't build these.
#[derive(Clone, Debug, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub enum BakeInput {
	/// A source file, a sidecar, or a directory default, read or only checked for existence.
	Source { id: String, version: AssetVersion },
	/// A stored resource of another bake. The fingerprint is [`resource_fingerprint`] of what the bake read, or
	/// [`MISSING_RESOURCE`] when the dependency was absent or failed to bake.
	Resource { id: String, fingerprint: u64 },
	/// The stored resources a query matched: its class and its property-equals-value predicates.
	Query {
		class: String,
		predicates: Vec<(String, String)>,
		fingerprint: u64,
	},
	/// The source files whose ID matched a glob pattern.
	Listing { pattern: String, fingerprint: u64 },
	/// A value the baking tool or application declared with
	/// [`AssetManager::declare_setting`](crate::asset::manager::AssetManager::declare_setting).
	Setting { name: String, fingerprint: u64 },
}

impl BakeInput {
	/// Returns the input's key: its kind, a `:`, and its name, such as `source:rocks/slate.fbx`.
	///
	/// Tools print it to say which input made a resource stale.
	pub fn key(&self) -> String {
		match self {
			BakeInput::Source { id, .. } => format!("source:{id}"),
			BakeInput::Resource { id, .. } => format!("resource:{id}"),
			BakeInput::Query { class, predicates, .. } => {
				let mut key = format!("query:{class}");
				for (property, value) in predicates {
					key.push_str(&format!(",{property}={value}"));
				}
				key
			}
			BakeInput::Listing { pattern, .. } => format!("listing:{pattern}"),
			BakeInput::Setting { name, .. } => format!("setting:{name}"),
		}
	}

	/// Returns the source ID when the input is a source file.
	pub fn source_id(&self) -> Option<&str> {
		match self {
			BakeInput::Source { id, .. } => Some(id),
			_ => None,
		}
	}

	/// Returns the recorded version when the input is a source file.
	pub(crate) fn source_version(&self) -> Option<&AssetVersion> {
		match self {
			BakeInput::Source { version, .. } => Some(version),
			_ => None,
		}
	}

	/// Builds the query input for `query`, whose matches hash to `fingerprint`.
	pub(crate) fn query(query: &Query, fingerprint: u64) -> Self {
		BakeInput::Query {
			class: query.class.clone(),
			predicates: query
				.predicates
				.iter()
				.map(|predicate| match predicate {
					crate::resource::storage_backend::QueryPredicate::Eq {
						property,
						value: crate::QueryableValue::String(value),
					} => (property.clone(), value.clone()),
				})
				.collect(),
			fingerprint,
		}
	}

	/// Rebuilds the query a [`BakeInput::Query`] recorded.
	pub(crate) fn recorded_query(class: &str, predicates: &[(String, String)]) -> Query {
		predicates.iter().fold(Query::new(class), |query, (property, value)| {
			query.eq(property, value.as_str())
		})
	}
}

/// Replaces the input with the same key in `inputs`, or inserts it so the list stays sorted by key.
///
/// A sorted list persists the same bytes whatever order a handler's concurrent reads finished in.
pub(crate) fn upsert_input(inputs: &mut Vec<BakeInput>, input: BakeInput) {
	let key = input.key();
	match inputs.binary_search_by(|existing| existing.key().cmp(&key)) {
		Ok(index) => inputs[index] = input,
		Err(index) => inputs.insert(index, input),
	}
}

/// The `Provenance` struct is what the resource store remembers about how one resource ID came to be stored, so
/// the asset manager can decide whether to bake it again and which resources to remove.
///
/// A resource a bake was asked for records the bake's inputs and the other resources it stored. A resource stored by
/// another ID's bake records that producer instead. Read one with
/// [`ReadStorageBackend::read_provenance`](crate::resource::ReadStorageBackend::read_provenance), and find the inputs
/// that changed with [`stale_sources`](crate::asset::manager::stale_sources).
#[derive(Clone, Debug, Default, PartialEq, Eq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
pub struct Provenance {
	/// The inputs the ID's latest bake observed, sorted by key. `None` when the ID was never baked by request, or
	/// when its latest bake didn't finish.
	pub(crate) inputs: Option<Vec<BakeInput>>,
	/// The IDs of the other resources the ID's latest bake stored, sorted.
	pub(crate) outputs: Vec<String>,
	/// The IDs whose bakes store this resource as one of their outputs, sorted.
	pub(crate) producers: Vec<String>,
}

impl Provenance {
	/// Returns the inputs the ID's latest bake observed, or `None` when no bake of the ID finished.
	pub fn inputs(&self) -> Option<&[BakeInput]> {
		self.inputs.as_deref()
	}

	/// Returns the IDs of the other resources the ID's latest bake stored.
	pub fn outputs(&self) -> &[String] {
		&self.outputs
	}

	/// Returns the IDs whose bakes store this resource.
	pub fn producers(&self) -> &[String] {
		&self.producers
	}

	/// Returns whether the record says nothing, so it need not be stored.
	pub(crate) fn is_empty(&self) -> bool {
		self.inputs.is_none() && self.outputs.is_empty() && self.producers.is_empty()
	}
}

/// The fingerprint a [`BakeInput::Resource`] records for a dependency that was absent or failed to bake.
pub(crate) const MISSING_RESOURCE: u64 = 0;

/// Returns the fingerprint of a stored resource: a hash of everything a bake that reads it can observe.
///
/// A resource baked again to the same result keeps its fingerprint, so the resources baked from it stay current.
pub(crate) fn resource_fingerprint(resource: &SerializableResource) -> u64 {
	let mut hasher = fingerprint_hasher();
	hasher.write_u64(resource.hash());
	hasher.write_usize(resource.size());
	write_str(&mut hasher, resource.class());
	hasher.write_usize(resource.resource().len());
	hasher.write(resource.resource());
	for stream in resource.streams().unwrap_or_default() {
		write_str(&mut hasher, stream.name());
		hasher.write_usize(stream.size());
		hasher.write_usize(stream.offset());
	}
	// Zero is reserved for a missing resource.
	hasher.finish().max(1)
}

/// Returns the fingerprint of a list of `(id, fingerprint)` pairs, whatever order they arrive in.
pub(crate) fn set_fingerprint<'a>(entries: impl IntoIterator<Item = (&'a str, u64)>) -> u64 {
	let mut entries = entries.into_iter().collect::<Vec<_>>();
	entries.sort_unstable();
	let mut hasher = fingerprint_hasher();
	for (id, fingerprint) in entries {
		write_str(&mut hasher, id);
		hasher.write_u64(fingerprint);
	}
	hasher.finish()
}

/// Returns the fingerprint of source bytes, which a [`BakeInput::Source`] keeps beside the file's size and
/// modification time.
pub(crate) fn content_fingerprint(bytes: &[u8]) -> u64 {
	crate::resource::compression::payload_hash(bytes)
}

/// Returns the hasher every fingerprint that isn't a content hash is built with.
///
/// Changing it makes every stored fingerprint differ, so every resource bakes again.
pub(crate) fn fingerprint_hasher() -> utils::hash::FxHasher {
	utils::hash::FxHasher::with_seed(961961961961961)
}

/// Writes a string with its length, so adjacent strings can't run together into the same bytes.
fn write_str(hasher: &mut impl std::hash::Hasher, value: &str) {
	hasher.write_usize(value.len());
	hasher.write(value.as_bytes());
}
