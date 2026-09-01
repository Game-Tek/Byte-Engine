#![feature(allocator_api)]

use resource_management::{
	Model, ProcessedAsset,
	asset::ResourceId,
	resource::{
		ReDBStorageBackend, ReadStorageBackend as _, ResourceCompressionPolicy, ResourcePayloadEncoding, ResourceStorageMode,
		WriteStorageBackend as _,
	},
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

fn incompressible_payload(size: usize) -> Vec<u8> {
	let mut state = 0x9e37_79b9_u32;
	(0..size)
		.map(|_| {
			state ^= state << 13;
			state ^= state >> 17;
			state ^= state << 5;
			state as u8
		})
		.collect()
}

fn is_payload_file(path: &std::path::Path) -> bool {
	let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
		return false;
	};
	let Some((resource_id, remainder)) = name.split_once('-') else {
		return false;
	};
	let Some((hash, encoding)) = remainder.split_once('-') else {
		return false;
	};
	resource_id.len() == 32
		&& resource_id.bytes().all(|byte| byte.is_ascii_hexdigit())
		&& hash.len() == 16
		&& hash.bytes().all(|byte| byte.is_ascii_hexdigit())
		&& matches!(encoding, "raw" | "cpu-lz4" | "metal-io-lz4")
}

#[resource_management::r#async::test]
async fn cpu_compression_metadata_survives_reopen_and_readers_only_return_decoded_bytes() {
	for storage_mode in [ResourceStorageMode::Files, ResourceStorageMode::Packed] {
		let path = temporary_store();
		let id = ResourceId::new("compressed.fixture");
		let payload = vec![0x5a; 32 * 1024];

		{
			let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), storage_mode).unwrap();
			let stored = storage.store(ProcessedAsset::new(id, StoredFixture), &payload).await.unwrap();
			let raw = storage
				.store(
					ProcessedAsset::new(ResourceId::new("raw.fixture"), StoredFixture)
						.with_compression(ResourceCompressionPolicy::Disabled),
					&payload,
				)
				.await
				.unwrap();

			assert_eq!(stored.encoding(), ResourcePayloadEncoding::CpuLz4);
			assert_eq!(stored.size(), payload.len());
			assert!(stored.stored_size() < payload.len() - payload.len() / 8);
			assert_eq!(stored.hash(), raw.hash());
		}

		let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
		let (stored, mut reader) = storage.read(id).await.unwrap();
		assert_eq!(stored.encoding(), ResourcePayloadEncoding::CpuLz4);
		assert_eq!(reader.encoding(), ResourcePayloadEncoding::CpuLz4);

		let mut partial = vec![0; payload.len() - 1];
		assert!(reader.read_into(None, partial.as_mut_slice().into()).await.is_err());
		let mut stream_destination = [0_u8; 16];
		let stream_target = vec![resource_management::stream::StreamMut::new("chunk", &mut stream_destination)];
		assert!(reader.read_into(None, stream_target.into()).await.is_err());

		let mut decoded = vec![0; payload.len()];
		let loaded = reader.read_into(None, decoded.as_mut_slice().into()).await.unwrap();
		assert_eq!(loaded.buffer(), Some(payload.as_slice()));
		drop(loaded);

		let (_, reader) = storage.read(id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), payload);
		drop(backing);
		drop(storage);
		std::fs::remove_dir_all(path).unwrap();
	}
}

#[resource_management::r#async::test]
async fn corrupt_cpu_compression_fails_without_returning_stored_bytes() {
	let path = temporary_store();
	let id = ResourceId::new("corrupt.fixture");
	{
		let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).unwrap();
		storage
			.store(ProcessedAsset::new(id, StoredFixture), &vec![0x44; 16 * 1024])
			.await
			.unwrap();
	}
	let payload_path = std::fs::read_dir(&path)
		.unwrap()
		.map(|entry| entry.unwrap().path())
		.find(|entry| is_payload_file(entry))
		.expect("compressed payload file");
	assert!(payload_path.extension().is_none());
	let stored_size = std::fs::metadata(&payload_path).unwrap().len() as usize;
	std::fs::write(payload_path, vec![0xff; stored_size]).unwrap();

	let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
	let (_, reader) = storage.read(id).await.unwrap();
	assert!(reader.into_backing_storage().await.is_err());
	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[resource_management::r#async::test]
async fn whole_resource_controls_and_heuristics_keep_unsuitable_payloads_uncompressed() {
	let path = temporary_store();
	let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).unwrap();
	let cases = [
		(
			ResourceId::new("small.fixture"),
			vec![7; 1023],
			ResourceCompressionPolicy::Enabled,
		),
		(
			ResourceId::new("incompressible.fixture"),
			incompressible_payload(16 * 1024),
			ResourceCompressionPolicy::Enabled,
		),
		(
			ResourceId::new("disabled.fixture"),
			vec![7; 16 * 1024],
			ResourceCompressionPolicy::Disabled,
		),
	];

	for (id, payload, policy) in cases {
		let stored = storage
			.store(ProcessedAsset::new(id, StoredFixture).with_compression(policy), &payload)
			.await
			.unwrap();

		assert_eq!(stored.encoding(), ResourcePayloadEncoding::Raw);
		assert_eq!(stored.size(), payload.len());
		assert_eq!(stored.stored_size(), payload.len());
		let (_, reader) = storage.read(id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), payload);
	}

	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[resource_management::r#async::test]
async fn extensionless_files_keep_distinct_encodings_for_the_same_decoded_hash() {
	let path = temporary_store();
	let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).unwrap();
	let id = ResourceId::new("encoding-change.fixture");
	let payload = vec![0x6c; 16 * 1024];
	let compressed = storage.store(ProcessedAsset::new(id, StoredFixture), &payload).await.unwrap();
	let raw = storage
		.store(
			ProcessedAsset::new(id, StoredFixture).with_compression(ResourceCompressionPolicy::Disabled),
			&payload,
		)
		.await
		.unwrap();

	assert_eq!(compressed.hash(), raw.hash());
	assert_eq!(compressed.encoding(), ResourcePayloadEncoding::CpuLz4);
	assert_eq!(raw.encoding(), ResourcePayloadEncoding::Raw);
	let payload_files = std::fs::read_dir(&path)
		.unwrap()
		.map(|entry| entry.unwrap().path())
		.filter(|path| is_payload_file(path))
		.collect::<Vec<_>>();
	assert_eq!(payload_files.len(), 2);
	assert!(payload_files.iter().all(|path| path.extension().is_none()));

	let (_, reader) = storage.read(id).await.unwrap();
	assert_eq!(reader.encoding(), ResourcePayloadEncoding::Raw);
	assert_eq!(reader.into_backing_storage().await.unwrap().as_slice(), payload);
	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[resource_management::r#async::test]
async fn partial_resource_transactions_remain_uncompressed() {
	for storage_mode in [ResourceStorageMode::Files, ResourceStorageMode::Packed] {
		let path = temporary_store();
		let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), storage_mode).unwrap();
		let id = ResourceId::new("partial.fixture");
		let payload = vec![0x33; 16 * 1024];
		let mut transaction = storage.begin_resource(id, payload.len()).await.unwrap();
		let midpoint = payload.len() / 2;
		transaction.write_all(&payload[..midpoint]).await.unwrap();
		transaction.write_all(&payload[midpoint..]).await.unwrap();
		let stored = transaction
			.commit(ProcessedAsset::new(id, StoredFixture), &std::alloc::Global)
			.await
			.unwrap();

		assert_eq!(stored.encoding(), ResourcePayloadEncoding::Raw);
		assert_eq!(stored.size(), payload.len());
		assert_eq!(stored.stored_size(), payload.len());
		let (_, reader) = storage.read(id).await.unwrap();
		let backing = reader.into_backing_storage().await.unwrap();
		assert_eq!(backing.as_slice(), payload);
		drop(backing);
		drop(storage);
		std::fs::remove_dir_all(path).unwrap();
	}
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
	let (stored, reader) = storage.read(id).await.unwrap();
	assert_eq!(stored.encoding(), ResourcePayloadEncoding::Raw);
	let backing = reader.into_backing_storage().await.unwrap();
	assert_eq!(backing.as_slice(), b"rebuilt");
	drop(backing);
	drop(storage);
	std::fs::remove_dir_all(path).unwrap();
}

#[cfg(all(target_os = "macos", feature = "gpu-processing"))]
#[resource_management::r#async::test]
async fn metal_texture_compression_persists_across_read_only_reopen() {
	use std::io::Write as _;

	use resource_management::{
		StreamDescription,
		resource::{ResourceGpuCompressionPolicy, ResourcePayloadEncoding, ResourceReaderBacking, ResourceStorageSettings},
		resources::image::Image,
		types::{Formats, Gamma},
	};

	let path = temporary_store();
	let id = ResourceId::new("texture.image");
	let decoded = [3_u8; 32 * 32 * 4];
	{
		let storage = ReDBStorageBackend::new_writable_with_settings(
			path.clone(),
			ResourceStorageSettings::new(ResourceStorageMode::Files)
				.image_compression(ResourceGpuCompressionPolicy::MetalIoLz4),
		)
		.unwrap();
		let image = ProcessedAsset::new(
			id,
			Image {
				format: Formats::RGBA8,
				gamma: Gamma::Linear,
				extent: [32, 32, 0],
				mip_count: 1,
				ibl: None,
				photometry: None,
			},
		)
		.with_streams(vec![StreamDescription::new("mip[0]", decoded.len(), 0)]);
		storage.store(image, &decoded).await.unwrap();
	}

	let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
	let (stored, reader) = storage.read(id).await.unwrap();
	assert_eq!(stored.encoding(), ResourcePayloadEncoding::MetalIoLz4);
	let backing = reader.into_backing_storage().await.unwrap();
	let ResourceReaderBacking::Gpu(backing) = backing else {
		panic!(
			"Compressed texture reopened as CPU data. The most likely cause is that its per-resource encoding was not persisted."
		);
	};

	assert_eq!(backing.encoding(), ResourcePayloadEncoding::MetalIoLz4);
	assert!(backing.path().exists());
	assert!(backing.path().extension().is_none());
	assert_eq!(
		u64::try_from(stored.stored_size()).unwrap(),
		std::fs::metadata(backing.path()).unwrap().len()
	);
	let payload_path = backing.path().to_owned();
	drop(backing);
	drop(storage);

	std::fs::OpenOptions::new()
		.append(true)
		.open(payload_path)
		.unwrap()
		.write_all(&[0])
		.unwrap();
	let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
	assert!(
		storage.read(id).await.is_none(),
		"A GPU container whose physical extent disagrees with its resource metadata must not be opened."
	);
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

#[resource_management::r#async::test]
async fn clear_persists_an_empty_reusable_store_without_changing_its_payload_mode() {
	for storage_mode in [ResourceStorageMode::Files, ResourceStorageMode::Packed] {
		let path = temporary_store();
		let removed_id = ResourceId::new("removed.fixture");
		{
			let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), storage_mode).unwrap();
			storage
				.store(ProcessedAsset::new(removed_id, StoredFixture), b"removed")
				.await
				.unwrap();
			storage.clear().await.unwrap();

			assert!(storage.list().await.unwrap().is_empty());
			assert!(storage.read(removed_id).await.is_none());
		}

		{
			let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
			assert!(storage.list().await.unwrap().is_empty());
		}

		let incompatible_mode = match storage_mode {
			ResourceStorageMode::Files => ResourceStorageMode::Packed,
			ResourceStorageMode::Packed => ResourceStorageMode::Files,
		};
		assert!(ReDBStorageBackend::new_writable_with_mode(path.clone(), incompatible_mode).is_err());

		let rebuilt_id = ResourceId::new("rebuilt.fixture");
		{
			let storage = ReDBStorageBackend::new_writable(path.clone());
			storage
				.store(ProcessedAsset::new(rebuilt_id, StoredFixture), b"rebuilt")
				.await
				.unwrap();
			let (_, reader) = storage.read(rebuilt_id).await.unwrap();
			let backing = reader.into_backing_storage().await.unwrap();
			assert_eq!(backing.as_slice(), b"rebuilt");
		}

		std::fs::remove_dir_all(path).unwrap();
	}
}
