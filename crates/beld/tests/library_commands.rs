use resource_management::{
	ProcessedAsset,
	asset::ResourceId,
	resource::{ReDBStorageBackend, ReadStorageBackend as _, ResourceStorageMode, WriteStorageBackend as _},
	resources::audio::Audio,
	types::BitDepths,
};

fn temporary_store() -> std::path::PathBuf {
	std::env::temp_dir().join(format!(
		"beld-library-command-test-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	))
}

#[test]
fn wipe_clears_the_selected_backend_without_changing_its_storage_mode() {
	let executor = beld::Executor::new().unwrap();
	executor.block_on(async {
		let path = temporary_store();
		let id = ResourceId::new("old.audio");
		{
			let storage = ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Packed).unwrap();
			storage
				.store(
					ProcessedAsset::new(
						id,
						Audio {
							bit_depth: BitDepths::Sixteen,
							channel_count: 1,
							sample_rate: 48_000,
							sample_count: 1,
						},
					),
					b"old",
				)
				.await
				.unwrap();
		}

		beld::wipe(path.to_string_lossy().into_owned()).await.unwrap();

		{
			let storage = ReDBStorageBackend::open_read_only(path.clone()).unwrap();
			assert!(storage.list().await.unwrap().is_empty());
			assert!(storage.read(id).await.is_none());
		}
		assert!(ReDBStorageBackend::new_writable_with_mode(path.clone(), ResourceStorageMode::Files).is_err());

		std::fs::remove_dir_all(path).unwrap();
	});
}
