use resource_management::{
	Model, ProcessedAsset,
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend as _, ResourceStorageMode, WriteStorageBackend as _},
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
		let _storage = ReDBStorageBackend::new(path.clone());
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
async fn packed_store_rebuilds_after_a_resource_management_signature_change() {
	let path = temporary_store();
	std::fs::create_dir_all(&path).unwrap();
	std::fs::write(path.join(".resource-management-version"), "stale-signature").unwrap();
	std::fs::write(path.join("resources.db"), "stale database").unwrap();
	std::fs::write(path.join("resources.pack"), b"stale packed payload").unwrap();

	let id = ResourceId::new("rebuilt.fixture");
	let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Packed).unwrap();
	storage
		.store(ProcessedAsset::new(id, StoredFixture), b"rebuilt")
		.await
		.unwrap();

	assert_eq!(std::fs::metadata(path.join("resources.pack")).unwrap().len(), 7);
	let (_, reader) = storage.read(id).await.unwrap();
	let backing = reader.into_backing_storage().await.unwrap();
	assert_eq!(backing.as_slice(), b"rebuilt");
	drop(backing);
	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[cfg(all(target_os = "macos", feature = "gpu-processing"))]
#[resource_management::r#async::test]
async fn metal_texture_compression_persists_across_read_only_reopen() {
	use resource_management::{
		StreamDescription,
		resource::{ResourceCompression, ResourceReaderBacking, ResourceStorageSettings},
		resources::image::Image,
		types::{Formats, Gamma},
	};

	let path = temporary_store();
	let id = ResourceId::new("texture.image");
	let decoded = [3_u8; 4 * 4 * 4];
	{
		let storage = ReDBStorageBackend::new_writable_with_settings(
			path.clone(),
			ResourceStorageSettings::new(ResourceStorageMode::Files).image_compression(ResourceCompression::MetalIoLz4),
		)
		.unwrap();
		let image = ProcessedAsset::new(
			id,
			Image {
				format: Formats::RGBA8,
				gamma: Gamma::Linear,
				extent: [4, 4, 1],
				mip_count: 1,
				ibl: None,
				photometry: None,
			},
		)
		.with_streams(vec![StreamDescription::new("mip[0]", decoded.len(), 0)]);
		storage.store(image, &decoded).await.unwrap();
	}

	let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
	let (_, reader) = storage.read(id).await.unwrap();
	let backing = reader.into_backing_storage().await.unwrap();
	let ResourceReaderBacking::Gpu(backing) = backing else {
		panic!(
			"Compressed texture reopened as CPU data. The most likely cause is that its per-resource encoding was not persisted."
		);
	};

	assert_eq!(backing.compression(), ResourceCompression::MetalIoLz4);
	assert!(backing.path().exists());
	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[resource_management::r#async::test]
async fn packed_mode_persists_across_writable_and_read_only_reopens() {
	let path = temporary_store();
	let first_id = ResourceId::new("first.fixture");
	{
		let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Packed).unwrap();
		storage
			.store(ProcessedAsset::new(first_id, StoredFixture), b"first payload")
			.await
			.unwrap();
	}

	assert!(ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).is_err());

	{
		let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
		let (_, reader) = storage.read(first_id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();

		assert_eq!(backing.as_slice(), b"first payload");
	}

	// The default writable opener discovers the packed mode instead of reverting the existing store to separate files.
	let second_id = ResourceId::new("second.fixture");
	{
		let storage = ReDBStorageBackend::new_writable(path.clone());
		storage
			.store(ProcessedAsset::new(second_id, StoredFixture), b"second payload")
			.await
			.unwrap();
	}
	{
		let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
		let (_, reader) = storage.read(second_id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();

		assert_eq!(backing.as_slice(), b"second payload");
	}

	std::fs::remove_dir_all(path).unwrap();
}

#[resource_management::r#async::test]
async fn packed_mode_recovers_reusable_ranges_after_reopen() {
	let path = temporary_store();
	let deleted_id = ResourceId::new("deleted.fixture");
	let retained_id = ResourceId::new("retained.fixture");
	{
		let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Packed).unwrap();
		storage
			.store(ProcessedAsset::new(deleted_id, StoredFixture), b"gone")
			.await
			.unwrap();
		storage
			.store(ProcessedAsset::new(retained_id, StoredFixture), b"kept")
			.await
			.unwrap();
		storage.delete(deleted_id).unwrap();
	}

	assert_eq!(std::fs::metadata(path.join("resources.pack")).unwrap().len(), 8);

	let reused_id = ResourceId::new("reused.fixture");
	{
		let storage = ReDBStorageBackend::new_writable(path.clone());
		storage
			.store(ProcessedAsset::new(reused_id, StoredFixture), b"free")
			.await
			.unwrap();

		assert_eq!(std::fs::metadata(path.join("resources.pack")).unwrap().len(), 8);
		for (id, expected) in [(retained_id, b"kept".as_slice()), (reused_id, b"free".as_slice())] {
			let (_, reader) = storage.read(id).await.unwrap();
			let backing = reader.into_backing_storage().await.unwrap();
			assert_eq!(backing.as_slice(), expected);
		}
	}

	std::fs::remove_dir_all(path).unwrap();
}
