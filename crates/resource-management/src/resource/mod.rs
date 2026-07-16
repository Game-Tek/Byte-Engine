//! The resource module contains the resource management system.

pub mod resource_manager;

pub mod resource_handler;

pub mod storage_backend;

pub mod resource_id;

pub mod read_target;
pub mod reader;

pub use read_target::ReadTargets;
pub use read_target::ReadTargetsMut;
pub use resource_id::ResourceId;
pub use storage_backend::redb_storage_backend::RedbStorageBackend;
pub use storage_backend::ReadStorageBackend;
pub use storage_backend::StorageBackend;
pub use storage_backend::WriteStorageBackend;

use crate::Model;

/// Trait that defines a resource.
pub trait Resource: Send + Sync {
	/// Returns the resource class (EJ: "Texture", "Mesh", "Material", etc.)
	/// This is used to identify the resource type. Needs to be meaningful and will be a public constant.
	/// Is needed by the deserialize function.
	fn get_class(&self) -> &'static str {
		<Self::Model as Model>::get_class()
	}

	fn queryable_properties(&self, id: &str) -> Vec<crate::QueryableProperty> {
		vec![crate::QueryableProperty {
			name: "name".to_string(),
			value: crate::QueryableValue::String(id.to_string()),
		}]
	}

	type Model: Model;
}
