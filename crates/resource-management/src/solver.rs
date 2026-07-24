use crate::{r#async::BoxedFuture, resource::ReadStorageBackend};

/// The `Solver` trait provides the dependency-aware conversion from stored models to typed runtime resources.
///
/// [`ResourceManager`](crate::ResourceManager) starts resolution for the requested
/// resource. Nested models await [`Solver::solve`] to resolve their dependencies.
pub trait Solver<'de, T> {
	fn solve(self, storage_backend: &'de dyn ReadStorageBackend) -> BoxedFuture<'de, Result<T, SolveErrors>>
	where
		Self: 'de;
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
