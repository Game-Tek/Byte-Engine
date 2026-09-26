use crate::{
	Model, Reference, ReferenceModel, Resource, SerializableResource,
	r#async::BoxedFuture,
	resource::{resource_handler::MultiResourceReader, storage_backend::DynReadStorageBackend},
};

/// The `Solver` trait provides the dependency-aware conversion from stored models to typed runtime resources.
///
/// [`ResourceManager`](crate::ResourceManager) starts resolution for the requested
/// resource. Nested models await [`Solver::solve`] to resolve their dependencies.
pub trait Solver<'de, T> {
	fn solve(self, storage_backend: &'de dyn DynReadStorageBackend) -> BoxedFuture<'de, Result<T, SolveError>>
	where
		Self: 'de;
}

/// The `StoredModel` trait builds a runtime reference from a resource record that storage already returned.
///
/// Implement it for each stored model instead of [`Solver`]: [`ReferenceModel`] solves through it after reading its
/// record, and [`ResourceManager::request`](crate::ResourceManager::request) passes in the record it already read, so
/// a requested resource is read from storage once.
pub trait StoredModel: Model + Sized {
	/// The runtime resource this model resolves to.
	type Resource: Resource<Model = Self>;

	/// Deserializes `stored`, solves its dependencies, and keeps `reader` for the resource's binary data.
	fn solve_stored<'de>(
		stored: SerializableResource,
		reader: MultiResourceReader,
		storage_backend: &'de dyn DynReadStorageBackend,
	) -> BoxedFuture<'de, Result<Reference<Self::Resource>, SolveError>>;
}

impl<'de, M: StoredModel> Solver<'de, Reference<M::Resource>> for ReferenceModel<M> {
	fn solve(
		self,
		storage_backend: &'de dyn DynReadStorageBackend,
	) -> BoxedFuture<'de, Result<Reference<M::Resource>, SolveError>>
	where
		Self: 'de,
	{
		crate::r#async::future(async move {
			let id = self.id();
			let (stored, reader) = storage_backend
				.read(id)
				.await
				.ok_or_else(|| SolveError::UnreadableDependency { id: id.to_string() })?;

			M::solve_stored(stored, reader, storage_backend).await
		})
	}
}

/// The `SolveError` enum reports why a stored model could not become a typed runtime resource.
///
/// [`ResourceManager::request`](crate::ResourceManager::request) wraps it in
/// [`RequestError::Solve`](crate::RequestError::Solve) together with the requested ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SolveError {
	/// The stored record or one of its dependencies could not be deserialized.
	DeserializationFailed(String),
	/// A dependency named by the stored record could not be read from storage.
	///
	/// Storage backends report absence and read failures the same way, so this covers a dependency that was never
	/// baked as well as one whose record or payload is damaged.
	UnreadableDependency { id: String },
}

impl std::fmt::Display for SolveError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		match self {
			SolveError::DeserializationFailed(error) => write!(
				f,
				"Could not deserialize the stored resource: {error}. The most likely cause is that the resource was baked by an incompatible resource-management version."
			),
			SolveError::UnreadableDependency { id } => write!(
				f,
				"Dependency '{id}' could not be read from storage. The most likely cause is that the dependency was not baked, or its stored payload is missing or damaged."
			),
		}
	}
}

impl std::error::Error for SolveError {}
