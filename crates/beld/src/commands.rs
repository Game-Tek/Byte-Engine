//! BELD command implementations grouped by operation and shared presentation support.

mod bake;
mod inspect;
mod maintenance;
mod query;
mod shared;

pub use bake::bake;
pub use inspect::inspect;
pub use maintenance::{delete, list, wipe};
pub use query::query;
#[cfg(test)]
use query::{parse_query_property, query_error_message};
#[cfg(test)]
use shared::{decode_hex, decode_query_cursor, encode_hex, encode_query_cursor, queryable_properties_json};
#[cfg(all(test, debug_assertions))]
use {bake::discover_asset_ids, shared::resource_trace_json};
#[cfg(test)]
mod tests {
	use std::time::{SystemTime, UNIX_EPOCH};

	use resource_management::{
		asset::{FileStorageBackend, ResourceId},
		resource::storage_backend::{QueryCursor, QueryError},
		QueryableProperty, QueryableValue,
	};
	#[cfg(debug_assertions)]
	use resource_management::{
		resource::{ReadStorageBackend, RedbStorageBackend, WriteStorageBackend},
		resources::audio::Audio,
		types::BitDepths,
		ProcessedAsset, ResourceTraceItem, ResourceTraceLevel,
	};
	use serde_json::json;

	#[cfg(debug_assertions)]
	use super::list;
	#[cfg(debug_assertions)]
	use super::{bake, inspect, query, resource_trace_json};
	use super::{
		decode_hex, decode_query_cursor, discover_asset_ids, encode_hex, encode_query_cursor, parse_query_property,
		query_error_message, queryable_properties_json, wipe,
	};
	use crate::utils::get_asset_manager;
	#[cfg(debug_assertions)]
	use crate::{InspectFormat, QueryFormat};

	#[test]
	fn query_property_parser_splits_once_and_rejects_missing_halves() {
		assert_eq!(parse_query_property("name=hero"), Ok(("name", "hero")));
		assert_eq!(parse_query_property("expression=a=b"), Ok(("expression", "a=b")));
		assert_eq!(parse_query_property("name"), Err(1));
		assert_eq!(parse_query_property("=value"), Err(1));
		assert_eq!(parse_query_property("name="), Err(1));
	}

	#[test]
	fn hex_codec_round_trips_all_byte_values_and_accepts_uppercase() {
		let bytes: Vec<u8> = (u8::MIN..=u8::MAX).collect();
		let encoded = encode_hex(&bytes);

		assert_eq!(encoded.len(), bytes.len() * 2);
		assert_eq!(decode_hex(&encoded), Some(bytes.clone()));
		assert_eq!(decode_hex(&encoded.to_uppercase()), Some(bytes));
		assert_eq!(decode_hex("0"), None);
		assert_eq!(decode_hex("gg"), None);
	}

	#[test]
	fn query_cursor_codec_is_lossless_and_rejects_non_cursor_json() {
		let cursor = QueryCursor::new(vec![0, 1, 2, 0xfe, 0xff]);
		let encoded = encode_query_cursor(&cursor);

		assert_eq!(decode_query_cursor(&encoded), Ok(cursor));
		assert_eq!(decode_query_cursor("not-hex"), Err(1));
		assert_eq!(decode_query_cursor(&encode_hex(br#"{"wrong":true}"#)), Err(1));
	}

	#[test]
	fn query_properties_convert_to_json_without_losing_names_or_values() {
		let properties = [
			QueryableProperty {
				name: "name".into(),
				value: QueryableValue::String("hero".into()),
			},
			QueryableProperty {
				name: "group".into(),
				value: QueryableValue::String("opaque".into()),
			},
		];

		assert_eq!(
			queryable_properties_json(&properties),
			json!({"name": "hero", "group": "opaque"})
		);
	}

	#[test]
	fn query_errors_keep_distinct_actionable_causes() {
		assert!(query_error_message(QueryError::InvalidCursor).contains("cursor is invalid"));
		assert!(query_error_message(QueryError::StorageFailure).contains("database could not be read"));
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

	#[cfg(debug_assertions)]
	#[test]
	fn trace_json_preserves_item_order_levels_and_messages() {
		let items = [
			ResourceTraceItem::new(ResourceTraceLevel::Info, "Imported metadata.".to_string()),
			ResourceTraceItem::new(ResourceTraceLevel::Warn, "Discarded optional data.".to_string()),
			ResourceTraceItem::new(ResourceTraceLevel::Error, "Source is malformed.".to_string()),
		];

		assert_eq!(
			resource_trace_json(&items),
			json!([
				{"level": "info", "message": "Imported metadata."},
				{"level": "warn", "message": "Discarded optional data."},
				{"level": "error", "message": "Source is malformed."},
			])
		);
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
			RedbStorageBackend::new(root.join("test-resources")),
		);
		let ids = discover_asset_ids(&root, &asset_manager).unwrap();

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
			RedbStorageBackend::new(root.join("test-resources")),
		);
		let ids = discover_asset_ids(&root, &asset_manager).unwrap();

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
			RedbStorageBackend::new(root.join("test-resources")),
		);
		let ids = discover_asset_ids(&root, &asset_manager).unwrap();

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

		assert_eq!(
			bake(
				assets_path.to_string_lossy().into_owned(),
				resources_path.to_string_lossy().into_owned(),
				Vec::new(),
				None,
				std::num::NonZeroUsize::new(1024 * 1024).unwrap(),
			),
			Err(1)
		);

		let executor = resource_management::r#async::Executor::new().unwrap();
		let resource_storage = RedbStorageBackend::new(resources_path.clone());
		let failed_trace = executor
			.block_on(resource_storage.read_trace(ResourceId::new("broken.png")))
			.unwrap();

		assert_eq!(failed_trace.len(), 1);
		assert_eq!(failed_trace[0].level(), ResourceTraceLevel::Error);
		assert!(executor
			.block_on(resource_storage.read(ResourceId::new("broken.png")))
			.is_none());

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
				InspectFormat::Json,
			)),
			Ok(())
		);
		assert_eq!(
			executor.block_on(inspect(
				resources_path.to_string_lossy().into_owned(),
				"successful.audio".to_string(),
				InspectFormat::Json,
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
				QueryFormat::Json,
			)),
			Ok(())
		);
		std::fs::remove_dir_all(root).unwrap();
	}

	#[test]
	fn wipe_removes_old_contents_and_recreates_empty_destination() {
		let path = std::env::temp_dir().join(format!(
			"beld-wipe-test-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		));
		std::fs::create_dir_all(path.join("nested")).unwrap();
		std::fs::write(path.join("nested/old.resource"), b"old").unwrap();

		wipe(path.to_string_lossy().into_owned()).unwrap();

		assert!(path.is_dir());
		assert_eq!(std::fs::read_dir(&path).unwrap().count(), 0);
		std::fs::remove_dir(path).unwrap();
	}
}
