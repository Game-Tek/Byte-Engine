use crate::resource::ReadStorageBackend;

/// The `Solver` trait resolves serialized resource models into runtime resources.
/// This is used to load resources from the storage backend into typed resources.
/// `solve` will be called by nested resources to resolve their entire "tree" of dependencies.
/// The initial call to `solve` will be done by the resource manager to resolve the resource which was requested by the client.
pub trait Solver<'de, T> {
	fn solve(self, storage_backend: &dyn ReadStorageBackend) -> Result<T, SolveErrors>;
}

#[derive(Debug)]
pub enum SolveErrors {
	DeserializationFailed(String),
	StorageError,
}
