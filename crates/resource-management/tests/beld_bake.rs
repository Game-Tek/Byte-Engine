//! Black-box tests of BELD's bake, discovery, and inspection commands with the engine's default baking setup, as an
//! application's BELD runs them.

use std::time::{SystemTime, UNIX_EPOCH};

use byte_engine::application::graphics::configure_default_baking;
#[cfg(debug_assertions)]
use resource_management::{
	ProcessedAsset, ResourceTraceItem, ResourceTraceLevel, resource::WriteStorageBackend as _, resources::audio::Audio,
	types::BitDepths,
};
use resource_management::{
	ReferenceModel,
	asset::{FileStorageBackend, ResourceId, manager::AssetManager},
	r#async::Executor,
	beld,
	parameters::Parameter,
	resource::{ReDBStorageBackend, ReadStorageBackend},
	resources::mesh::MeshModel,
};

/// Configures `asset_manager` as [`configure_default_baking`] does, but with the CPU texture backends, so the tests
/// don't depend on a GPU.
fn configure_cpu_baking(asset_manager: &mut AssetManager, parameters: &[Parameter]) {
	configure_default_baking(asset_manager, parameters);
	asset_manager.set_image_processor(resource_management::processors::image::ImageProcessor::default());
}

/// Returns an asset manager over `assets` and `resources`, configured as an application's BELD bakes with.
fn baking_asset_manager(assets: FileStorageBackend, resources: ReDBStorageBackend) -> AssetManager {
	let mut asset_manager = AssetManager::new(assets, resources);
	configure_cpu_baking(&mut asset_manager, &[]);
	asset_manager
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

	let executor = Executor::new().unwrap();

	assert_eq!(executor.block_on(beld::list(root.to_string_lossy().into_owned())), Err(1));
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

	let asset_manager = baking_asset_manager(
		FileStorageBackend::new(root.clone()),
		ReDBStorageBackend::new(root.join("test-resources")).unwrap(),
	);
	let executor = Executor::new().unwrap();
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

	let asset_manager = baking_asset_manager(
		FileStorageBackend::new(root.clone()),
		ReDBStorageBackend::new(root.join("test-resources")).unwrap(),
	);
	let executor = Executor::new().unwrap();
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
	symlink(&engine_assets, root.join("byte_engine")).unwrap();
	symlink(engine_assets.join("engine-icon.png"), root.join("linked-engine-icon.png")).unwrap();
	symlink(&root, engine_assets.join("cycle-to-application-assets")).unwrap();

	let asset_manager = baking_asset_manager(
		FileStorageBackend::new(root.clone()),
		ReDBStorageBackend::new(root.join("test-resources")).unwrap(),
	);
	let executor = Executor::new().unwrap();
	let ids = executor.block_on(asset_manager.discover()).unwrap();

	assert_eq!(
		ids,
		[
			"byte_engine/engine-icon.png",
			"byte_engine/shaders/render-pass.bema",
			"linked-engine-icon.png",
		]
	);
	std::fs::remove_dir_all(root).unwrap();
	std::fs::remove_dir_all(engine_assets).unwrap();
}

/// Returns a one-sample, 16-bit mono WAV file.
fn wav() -> Vec<u8> {
	let mut bytes = Vec::with_capacity(46);
	bytes.extend_from_slice(b"RIFF");
	bytes.extend_from_slice(&38u32.to_le_bytes());
	bytes.extend_from_slice(b"WAVEfmt ");
	bytes.extend_from_slice(&16u32.to_le_bytes());
	bytes.extend_from_slice(&1u16.to_le_bytes());
	bytes.extend_from_slice(&1u16.to_le_bytes());
	bytes.extend_from_slice(&48_000u32.to_le_bytes());
	bytes.extend_from_slice(&96_000u32.to_le_bytes());
	bytes.extend_from_slice(&2u16.to_le_bytes());
	bytes.extend_from_slice(&16u16.to_le_bytes());
	bytes.extend_from_slice(b"data");
	bytes.extend_from_slice(&2u32.to_le_bytes());
	bytes.extend_from_slice(&0i16.to_le_bytes());
	bytes
}

/// Creates a fresh source directory with `music.wav`, `voice.wav`, and `unused.wav`, and returns the test root.
fn sound_project() -> std::path::PathBuf {
	let root = std::env::temp_dir().join(format!(
		"beld-roots-test-{}-{}",
		std::process::id(),
		SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
	));
	std::fs::create_dir_all(root.join("assets")).unwrap();
	for name in ["music.wav", "voice.wav", "unused.wav"] {
		std::fs::write(root.join("assets").join(name), wav()).unwrap();
	}
	// Every bake assembles the coverage shaders, which read the engine's layer modules from the project.
	copy_engine_layer_modules(&root.join("assets"));
	root
}

/// Bakes every discovered asset of the project at `root` with `roots` and `excluded`, and returns the exit code
/// and the stored IDs, leaving out the coverage shaders every bake assembles.
fn bake_project(root: &std::path::Path, roots: &[&str], excluded: &[&str], prune: bool) -> (Result<(), i32>, Vec<String>) {
	let resources_path = root.join("resources");
	// The same parameters a `config.json` with `resources.roots` and `assets.exclude` arrays sets.
	let elements = |name: &str, patterns: &[&str]| {
		patterns
			.iter()
			.enumerate()
			.map(|(index, pattern)| Parameter::new(&format!("{name}.{index}"), pattern))
			.collect::<Vec<_>>()
	};
	let parameters = [elements("resources.roots", roots), elements("assets.exclude", excluded)].concat();
	let executor = Executor::new().unwrap();

	let result = executor.block_on(beld::bake(
		root.join("assets").to_string_lossy().into_owned(),
		resources_path.to_string_lossy().into_owned(),
		Vec::new(),
		None,
		None,
		std::num::NonZeroUsize::new(1024 * 1024).unwrap(),
		false,
		&parameters,
		configure_cpu_baking,
		prune,
	));
	let mut ids = executor
		.block_on(ReadStorageBackend::list(&ReDBStorageBackend::new(resources_path).unwrap()))
		.unwrap()
		.into_iter()
		.map(|resource| resource.id().to_string())
		.filter(|id| !id.contains("-coverage.uber"))
		.collect::<Vec<_>>();
	ids.sort_unstable();
	(result, ids)
}

/// Bakes a fresh project once with `roots` and `excluded`, and returns the exit code and the stored IDs.
fn bake_with(roots: &[&str], excluded: &[&str]) -> (Result<(), i32>, Vec<String>) {
	let root = sound_project();
	let baked = bake_project(&root, roots, excluded, false);
	std::fs::remove_dir_all(root).unwrap();
	baked
}

#[test]
fn a_bake_with_roots_bakes_only_the_matching_assets() {
	assert_eq!(
		bake_with(&[], &[]),
		(Ok(()), vec!["music.wav".into(), "unused.wav".into(), "voice.wav".into()])
	);
	assert_eq!(
		bake_with(&["music.*", "VOICE.wav"], &[]),
		(Ok(()), vec!["music.wav".into(), "voice.wav".into()])
	);
}

#[cfg(debug_assertions)]
#[test]
fn a_pruning_bake_deletes_what_no_root_reaches_anymore() {
	let root = sound_project();

	assert_eq!(bake_project(&root, &["*.wav"], &[], false).1.len(), 3);
	assert_eq!(
		bake_project(&root, &["music.wav"], &[], false),
		(Ok(()), vec!["music.wav".into(), "unused.wav".into(), "voice.wav".into()]),
		"a bake without --prune deletes nothing"
	);
	assert_eq!(
		bake_project(&root, &["music.wav"], &[], true),
		(Ok(()), vec!["music.wav".into()])
	);

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_pruning_bake_without_roots_is_rejected_before_baking() {
	let root = sound_project();

	assert_eq!(bake_project(&root, &[], &[], true), (Err(2), Vec::new()));

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn a_bake_stops_before_baking_when_a_root_is_excluded() {
	assert_eq!(bake_with(&["*.wav"], &["voice.wav"]), (Err(1), Vec::new()));
	assert_eq!(bake_with(&["music.wav"], &["voice.wav"]), (Ok(()), vec!["music.wav".into()]));
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
	let executor = Executor::new().unwrap();

	assert_eq!(
		executor.block_on(beld::bake(
			assets_path.to_string_lossy().into_owned(),
			resources_path.to_string_lossy().into_owned(),
			Vec::new(),
			None,
			None,
			std::num::NonZeroUsize::new(1024 * 1024).unwrap(),
			false,
			&[],
			configure_cpu_baking,
			false,
		)),
		Err(1)
	);

	let resource_storage = ReDBStorageBackend::new(resources_path.clone()).unwrap();
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
		executor.block_on(beld::inspect(
			resources_path.to_string_lossy().into_owned(),
			"broken.png".to_string(),
			beld::OutputFormat::JSON,
			beld::Payload::None,
			&mut std::io::sink(),
		)),
		Ok(())
	);
	assert_eq!(
		executor.block_on(beld::inspect(
			resources_path.to_string_lossy().into_owned(),
			"successful.audio".to_string(),
			beld::OutputFormat::JSON,
			beld::Payload::None,
			&mut std::io::sink(),
		)),
		Ok(())
	);
	let query = |terms: &[&str]| {
		executor.block_on(beld::query(
			assets_path.to_string_lossy().into_owned(),
			resources_path.to_string_lossy().into_owned(),
			Vec::new(),
			terms.iter().map(|term| term.to_string()).collect(),
			None,
			None,
			false,
			None,
			beld::OutputFormat::JSON,
		))
	};
	assert_eq!(query(&["class=Audio"]), Ok(()));
	assert_eq!(query(&["is:failed", "has:errors"]), Ok(()));
	// A field no resource declares is an invalid argument, not a failed read.
	assert_eq!(query(&["bogus=1"]), Err(2));
	std::fs::remove_dir_all(root).unwrap();
}

/// Copies the engine's layer modules into the project at `assets`, where a project that links the engine's assets
/// has them, so material bakes and the shaders assembled from them can read them. Only the `layouts` directories are
/// copied, which hold no asset a bake would discover.
fn copy_engine_layer_modules(assets: &std::path::Path) {
	fn copy(from: &std::path::Path, to: &std::path::Path, inside_layouts: bool) {
		for entry in std::fs::read_dir(from).unwrap().map(Result::unwrap) {
			let (from, to) = (entry.path(), to.join(entry.file_name()));
			if entry.file_type().unwrap().is_dir() {
				copy(&from, &to, inside_layouts || entry.file_name() == "layouts");
			} else if inside_layouts && from.extension().is_some_and(|extension| extension == "besl") {
				std::fs::create_dir_all(to.parent().unwrap()).unwrap();
				std::fs::copy(&from, &to).unwrap();
			}
		}
	}

	let engine = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../byte-engine/assets/rendering"));
	copy(engine, &assets.join("byte_engine/rendering"), false);
}

const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../src/asset/test_data/triangle_move_ascii.fbx");

/// Confirms that the production handlers bake an FBX mesh and its visibility material dependencies.
#[test]
fn default_asset_manager_bakes_fbx_mesh_and_generated_materials() {
	let executor =
		Executor::new().expect("Async runtime could not start. The most likely cause is unavailable platform I/O support.");

	executor.block_on(async {
		let root = std::env::temp_dir().join(format!(
			"beld-fbx-test-{}-{}",
			std::process::id(),
			SystemTime::now()
				.duration_since(UNIX_EPOCH)
				.expect("System clock is invalid. The most likely cause is a clock value before the Unix epoch.")
				.as_nanos()
		));

		let assets_path = root.join("assets");

		let resources_path = root.join("resources");

		std::fs::create_dir_all(&assets_path)
			.expect("FBX test assets could not be created. The most likely cause is an unwritable temporary directory.");

		std::fs::write(assets_path.join("triangle_move.fbx"), TRIANGLE_MOVE_FBX)
			.expect("FBX test asset could not be written. The most likely cause is an unwritable temporary directory.");
		copy_engine_layer_modules(&assets_path);

		let asset_manager = baking_asset_manager(
			FileStorageBackend::new(assets_path),
			ReDBStorageBackend::new(resources_path.clone()).unwrap(),
		);

		let mesh: ReferenceModel<MeshModel> = asset_manager
			.bake_if_not_exists("triangle_move.fbx")
			.await
			.expect("FBX mesh baking failed. The most likely cause is broken BELD handler or shader-generator registration.");

		drop(asset_manager);

		let resource_storage = ReDBStorageBackend::new(resources_path).unwrap();

		let (serialized, _) = resource_storage
			.read(ResourceId::new("triangle_move.fbx"))
			.await
			.expect("Baked FBX mesh is missing. The most likely cause is a resource storage failure.");

		let streams = serialized
			.streams()
			.expect("Baked FBX mesh streams are missing. The most likely cause is a mesh serialization regression.");

		assert_eq!(mesh.class(), "Mesh");

		for expected in ["level[0].Vertex.Position", "level[0].Vertex.Normal", "level[0].Vertex.UV"] {
			assert!(streams.iter().any(|stream| stream.name() == expected));
		}

		let resources = resource_storage
			.list()
			.await
			.expect("Resource list is unreadable. The most likely cause is a test storage failure.");

		assert_eq!(resources.len(), 5);
		assert!(resources.iter().any(|resource| resource.id() == "triangle_move.fbx#skeleton"));

		drop(resource_storage);

		std::fs::remove_dir_all(root)
			.expect("FBX test directory could not be removed. The most likely cause is an open resource file.");
	});
}
