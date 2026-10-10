//! BELD command implementations grouped by operation and shared presentation support.

mod bake;
mod inspect;
mod maintenance;
mod query;
mod shared;

pub use bake::bake;
pub use inspect::inspect;
pub use maintenance::{clear, delete, list, wipe};
#[cfg(test)]
use query::parse_query_property;
pub use query::query;
#[cfg(test)]
use shared::{decode_query_cursor, encode_query_cursor};
#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use byte_engine::rendering::pipelines::visibility::VisibilityFeatures;
	#[cfg(debug_assertions)]
	use resource_management::{
		ProcessedAsset, ResourceTraceItem, ResourceTraceLevel,
		resource::{ReadStorageBackend, WriteStorageBackend},
		resources::audio::Audio,
		types::BitDepths,
	};
	use resource_management::{
		asset::{FileStorageBackend, ResourceId},
		resource::{ReDBStorageBackend, storage_backend::QueryCursor},
	};

	#[cfg(debug_assertions)]
	use super::list;
	#[cfg(debug_assertions)]
	use super::{bake, inspect, query};
	use super::{decode_query_cursor, encode_query_cursor, parse_query_property};
	#[cfg(debug_assertions)]
	use crate::OutputFormat;
	use crate::utils::get_asset_manager;

	#[test]
	fn query_property_parser_splits_once_and_rejects_missing_halves() {
		assert_eq!(parse_query_property("name=hero"), Ok(("name", "hero")));
		assert_eq!(parse_query_property("expression=a=b"), Ok(("expression", "a=b")));
		assert_eq!(parse_query_property("name"), Err(1));
		assert_eq!(parse_query_property("=value"), Err(1));
		assert_eq!(parse_query_property("name="), Err(1));
	}

	#[test]
	fn query_cursor_codec_is_lossless_and_rejects_non_cursor_json() {
		let cursor = QueryCursor::new(vec![0, 1, 2, 0xfe, 0xff]);
		let encoded = encode_query_cursor(&cursor);

		assert_eq!(decode_query_cursor(&encoded), Ok(cursor));
		assert_eq!(decode_query_cursor("not-hex"), Err(1));
		assert_eq!(decode_query_cursor(&utils::hex::encode(br#"{"wrong":true}"#)), Err(1));
	}

	#[cfg(debug_assertions)]
	#[test]
	fn list_refuses_a_stale_store_without_modifying_it() {
		let root = std::env::temp_dir().join(format!(
			"beld-stale-list-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		let signature_path = root.join(".resource-management-version");
		let sentinel_path = root.join("sentinel");
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(&signature_path, b"stale-signature").unwrap();
		std::fs::write(&sentinel_path, b"retain-me").unwrap();

		let executor = resource_management::r#async::Executor::new().unwrap();

		assert_eq!(executor.block_on(list(root.to_string_lossy().into_owned())), Err(1));
		assert_eq!(std::fs::read(&signature_path).unwrap(), b"stale-signature");
		assert_eq!(std::fs::read(&sentinel_path).unwrap(), b"retain-me");

		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn discovers_supported_assets_recursively_and_ignores_sidecars_and_unknown_files() {
		let root = std::env::temp_dir().join(format!(
			"beld-discovery-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		std::fs::create_dir_all(root.join("nested/deeper")).unwrap();
		std::fs::write(root.join("z-last.png"), []).unwrap();
		std::fs::write(root.join("nested/deeper/a-first.fbx"), []).unwrap();
		std::fs::write(root.join("nested/material.bema"), []).unwrap();
		std::fs::write(root.join("nested/material.bema.bead"), []).unwrap();
		std::fs::write(root.join("ignored.txt"), []).unwrap();

		let asset_manager = get_asset_manager(
			FileStorageBackend::new(root.clone()),
			ReDBStorageBackend::new(root.join("test-resources")),
			VisibilityFeatures::default(),
		);
		let executor = resource_management::r#async::Executor::new().unwrap();
		let ids = executor.block_on(asset_manager.discover()).unwrap();

		assert_eq!(ids, ["nested/deeper/a-first.fbx", "nested/material.bema", "z-last.png"]);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn standalone_besl_discovery_skips_orphans_and_includes_sources_with_sidecars() {
		let root = std::env::temp_dir().join(format!(
			"beld-besl-discovery-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		std::fs::create_dir_all(root.join("rendering")).unwrap();
		std::fs::write(root.join("rendering/configured.besl"), b"main: fn () -> void {}").unwrap();
		std::fs::write(
			root.join("rendering/configured.besl.bead"),
			br#"{ "stage": "Compute", "workgroup": [8, 8, 1] }"#,
		)
		.unwrap();
		std::fs::write(root.join("rendering/orphan.besl"), b"main: fn () -> void {}").unwrap();

		let asset_manager = get_asset_manager(
			FileStorageBackend::new(root.clone()),
			ReDBStorageBackend::new(root.join("test-resources")),
			VisibilityFeatures::default(),
		);
		let executor = resource_management::r#async::Executor::new().unwrap();
		let ids = executor.block_on(asset_manager.discover()).unwrap();

		assert_eq!(ids, ["rendering/configured.besl"]);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[cfg(unix)]
	#[test]
	fn discovers_assets_through_symlinks_without_following_directory_cycles() {
		use std::os::unix::fs::symlink;

		let nonce = format!(
			"{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		);
		let root = std::env::temp_dir().join(format!("beld-symlink-discovery-test-{nonce}"));
		let engine_assets = std::env::temp_dir().join(format!("beld-engine-assets-test-{nonce}"));
		std::fs::create_dir_all(engine_assets.join("shaders")).unwrap();
		std::fs::create_dir_all(&root).unwrap();
		std::fs::write(engine_assets.join("shaders/render-pass.bema"), []).unwrap();
		std::fs::write(engine_assets.join("engine-icon.png"), []).unwrap();
		symlink(&engine_assets, root.join("byte-engine")).unwrap();
		symlink(engine_assets.join("engine-icon.png"), root.join("linked-engine-icon.png")).unwrap();
		symlink(&root, engine_assets.join("cycle-to-application-assets")).unwrap();

		let asset_manager = get_asset_manager(
			FileStorageBackend::new(root.clone()),
			ReDBStorageBackend::new(root.join("test-resources")),
			VisibilityFeatures::default(),
		);
		let executor = resource_management::r#async::Executor::new().unwrap();
		let ids = executor.block_on(asset_manager.discover()).unwrap();

		assert_eq!(
			ids,
			[
				"byte-engine/engine-icon.png",
				"byte-engine/shaders/render-pass.bema",
				"linked-engine-icon.png",
			]
		);
		std::fs::remove_dir_all(root).unwrap();
		std::fs::remove_dir_all(engine_assets).unwrap();
	}

	#[cfg(debug_assertions)]
	#[test]
	fn failed_and_successful_resource_traces_are_inspectable_and_queryable() {
		let root = std::env::temp_dir().join(format!(
			"beld-trace-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		let assets_path = root.join("assets");
		let resources_path = root.join("resources");
		std::fs::create_dir_all(&assets_path).unwrap();
		std::fs::write(assets_path.join("broken.png"), b"not a PNG").unwrap();
		let executor = resource_management::r#async::Executor::new().unwrap();

		assert_eq!(
			executor.block_on(bake(
				assets_path.to_string_lossy().into_owned(),
				resources_path.to_string_lossy().into_owned(),
				Vec::new(),
				None,
				None,
				std::num::NonZeroUsize::new(1024 * 1024).unwrap(),
				false,
				VisibilityFeatures::default(),
			)),
			Err(1)
		);

		let resource_storage = ReDBStorageBackend::new(resources_path.clone());
		let failed_trace = executor
			.block_on(resource_storage.read_trace(ResourceId::new("broken.png")))
			.unwrap();

		assert_eq!(failed_trace.len(), 1);
		assert_eq!(failed_trace[0].level(), ResourceTraceLevel::Error);
		assert!(
			executor
				.block_on(resource_storage.read(ResourceId::new("broken.png")))
				.is_none()
		);

		let successful_id = ResourceId::new("successful.audio");
		executor
			.block_on(resource_storage.store(
				ProcessedAsset::new(
					successful_id,
					Audio {
						bit_depth: BitDepths::Sixteen,
						channel_count: 2,
						sample_rate: 48_000,
						sample_count: 1,
					},
				),
				&[],
			))
			.unwrap();
		resource_storage
			.replace_trace(
				successful_id,
				&[ResourceTraceItem::new(
					ResourceTraceLevel::Warn,
					"Test warning associated with a baked resource.".to_string(),
				)],
			)
			.unwrap();
		drop(resource_storage);

		assert_eq!(
			executor.block_on(inspect(
				resources_path.to_string_lossy().into_owned(),
				"broken.png".to_string(),
				OutputFormat::JSON,
			)),
			Ok(())
		);
		assert_eq!(
			executor.block_on(inspect(
				resources_path.to_string_lossy().into_owned(),
				"successful.audio".to_string(),
				OutputFormat::JSON,
			)),
			Ok(())
		);
		assert_eq!(
			executor.block_on(query(
				resources_path.to_string_lossy().into_owned(),
				"Audio".to_string(),
				Vec::new(),
				None,
				None,
				OutputFormat::JSON,
			)),
			Ok(())
		);
		std::fs::remove_dir_all(root).unwrap();
	}
}
