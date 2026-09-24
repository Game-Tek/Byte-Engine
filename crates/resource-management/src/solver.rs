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
	fn solve(self, storage_backend: &'de dyn DynReadStorageBackend) -> BoxedFuture<'de, Result<T, SolveErrors>>
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
	) -> BoxedFuture<'de, Result<Reference<Self::Resource>, SolveErrors>>;
}

impl<'de, M: StoredModel> Solver<'de, Reference<M::Resource>> for ReferenceModel<M> {
	fn solve(
		self,
		storage_backend: &'de dyn DynReadStorageBackend,
	) -> BoxedFuture<'de, Result<Reference<M::Resource>, SolveErrors>>
	where
		Self: 'de,
	{
		crate::r#async::future(async move {
			let (stored, reader) = storage_backend.read(self.id()).await.ok_or(SolveErrors::StorageError)?;

			M::solve_stored(stored, reader, storage_backend).await
		})
	}
}

#[derive(Debug)]
pub enum SolveErrors {
	DeserializationFailed(String),
	StorageError,
}

impl From<SolveErrors> for &'static str {
	fn from(err: SolveErrors) -> Self {
		match err {
			SolveErrors::DeserializationFailed(_) => "Solve deserialization failed",
			SolveErrors::StorageError => "Solve related storage error",
		}
	}
}
