//! Check baked resources against project rules before they reach resource storage.
//!
//! Implement [`ResourceValidator`] for each rule, such as a payload size limit, and register it with
//! [`AssetManager::add_resource_validator`](crate::asset::manager::AssetManager::add_resource_validator).

use std::fmt;

use crate::{ProcessedAsset, QueryableProperty, StreamDescription};

/// The `ResourceCandidate` struct lets a [`ResourceValidator`] judge a resource that a handler is about to store.
///
/// It exposes metadata and the payload size, not the payload bytes, so every store path can validate a resource
/// without buffering or reading back its payload.
#[derive(Clone, Copy)]
pub struct ResourceCandidate<'a> {
	resource: &'a ProcessedAsset,
	payload_size: usize,
}

impl<'a> ResourceCandidate<'a> {
	/// Describes `resource` with the decoded size of the payload stored with it.
	pub fn new(resource: &'a ProcessedAsset, payload_size: usize) -> Self {
		Self { resource, payload_size }
	}

	/// Returns the ID the resource is stored under.
	pub fn id(&self) -> &'a str {
		self.resource.id()
	}

	/// Returns the resource class, such as `Image`, `Mesh`, or `Material`.
	pub fn class(&self) -> &'a str {
		self.resource.class()
	}

	/// Returns the payload size in bytes, before any storage compression.
	pub fn payload_size(&self) -> usize {
		self.payload_size
	}

	/// Returns the named payload ranges, when the resource declares them.
	pub fn streams(&self) -> Option<&'a [StreamDescription]> {
		self.resource.streams()
	}

	/// Returns the indexed properties that resource queries match against.
	pub fn queryable_properties(&self) -> &'a [QueryableProperty] {
		self.resource.queryable_properties()
	}
}

/// The `ResourceValidator` trait lets a project reject baked resources that break its own rules.
///
/// The asset manager calls [`Self::validate`] for every resource a bake stores, including generated dependencies,
/// before the resource is published to resource storage. A rejection fails the bake of that resource and of every
/// resource that depends on it, and nothing is stored for it.
///
/// Register a validator with
/// [`AssetManager::add_resource_validator`](crate::asset::manager::AssetManager::add_resource_validator).
pub trait ResourceValidator: Send + Sync {
	/// Returns an error when `candidate` must not be stored.
	fn validate(&self, candidate: &ResourceCandidate<'_>) -> Result<(), ValidationError>;
}

/// The `ValidationError` struct explains why a [`ResourceValidator`] rejected a resource and how to fix it.
///
/// Its three parts follow the engine's error message rule: what happened, the most likely cause, and the fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationError {
	what: String,
	cause: String,
	fix: String,
}

impl ValidationError {
	/// Creates a rejection from what happened, its most likely cause, and the action that most likely fixes it.
	pub fn new(what: impl Into<String>, cause: impl Into<String>, fix: impl Into<String>) -> Self {
		Self {
			what: what.into(),
			cause: cause.into(),
			fix: fix.into(),
		}
	}
}

impl fmt::Display for ValidationError {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{} {} {}", self.what, self.cause, self.fix)
	}
}

impl std::error::Error for ValidationError {}

/// Runs `validators` in registration order and returns the first rejection.
pub(crate) fn validate_resource(
	validators: &[Box<dyn ResourceValidator>],
	candidate: &ResourceCandidate<'_>,
) -> Result<(), ValidationError> {
	validators.iter().try_for_each(|validator| validator.validate(candidate))
}
