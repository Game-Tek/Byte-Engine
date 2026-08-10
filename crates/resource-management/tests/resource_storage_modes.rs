use resource_management::{
	asset::ResourceId,
	resource::{ReadStorageBackend as _, RedbStorageBackend, ResourceStorageMode, WriteStorageBackend as _},
	Model, ProcessedAsset,
};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
struct StoredFixture;

impl Model for StoredFixture {
	fn get_class() -> &'static str {
		"StoredFixture"
	}
}

fn temporary_store() -> std::path::PathBuf {
	std::env::temp_dir().join(format!(
		"byte-engine-packed-resource-store-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	))
}

#[resource_management::r#async::test]
async fn packed_mode_persists_across_writable_and_read_only_reopens() {
	let path = temporary_store();
	let first_id = ResourceId::new("first.fixture");
	{
		let storage = RedbStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Packed).unwrap();
		storage
			.store(ProcessedAsset::new(first_id, StoredFixture), b"first payload")
			.unwrap();
	}

	assert!(RedbStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).is_err());

	{
		let storage = RedbStorageBackend::open_read_only(path.clone()).unwrap();
		let (_, reader) = storage.read(first_id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), b"first payload");
	}

	// The default writable opener discovers the packed mode instead of reverting the existing store to separate files.
	let second_id = ResourceId::new("second.fixture");
	{
		let storage = RedbStorageBackend::new_writable(path.clone());
		storage
			.store(ProcessedAsset::new(second_id, StoredFixture), b"second payload")
			.unwrap();
	}
	{
		let storage = RedbStorageBackend::open_read_only(path.clone()).unwrap();
		let (_, reader) = storage.read(second_id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), b"second payload");
	}

	std::fs::remove_dir_all(path).unwrap();
}
