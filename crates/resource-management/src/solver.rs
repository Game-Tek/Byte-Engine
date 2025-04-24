use serde::Deserialize;

use crate::resource::ReadStorageBackend;

/// The solver trait provides methods to resolve a resource.
/// This is used to load resources from the storage backend into typed resources.
/// `solve` will be called by nested resources to resolve their entire "tree" of dependencies.
/// The initial call to `solve` will be done by the resource manager to resolve the resource which was requested by the client.
pub trait Solver<'de, T> where Self: Deserialize<'de> {
    fn solve(self, storage_backend: &dyn ReadStorageBackend) -> Result<T, SolveErrors>;
}

#[derive(Debug)]
pub enum SolveErrors {
    DeserializationFailed(String),
    StorageError,
}