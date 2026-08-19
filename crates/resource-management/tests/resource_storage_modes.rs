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

#[test]
fn application_opener_discards_a_store_with_a_mismatched_resource_management_signature() {
	let path = temporary_store();
	let stale_resource = path.join("stale-resource");
	std::fs::create_dir_all(&path).unwrap();
	std::fs::write(path.join(".resource-management-version"), "stale-signature").unwrap();
	std::fs::write(path.join("resources.db"), "stale database").unwrap();
	std::fs::write(&stale_resource, "stale data").unwrap();

	// This is the public opener used by applications. It must synchronize the marker before any stale value can be read.
	{
		let _storage = RedbStorageBackend::new(path.clone());
	}

	assert!(
		!stale_resource.exists(),
		"a stale resource must be removed before an application can read it"
	);
	assert_ne!(
		std::fs::read_to_string(path.join(".resource-management-version"))
			.unwrap()
			.trim(),
		"stale-signature"
	);
	std::fs::remove_dir_all(&path).unwrap();
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
