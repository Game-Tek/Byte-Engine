//! Black-box tests of BELD's dry-run bake, payload, and intercept output, as specified in the BELD command-line
//! reference.

use std::path::{Path, PathBuf};

use byte_engine::application::graphics::configure_default_baking;
use resource_management::{
	ProcessedAsset,
	asset::ResourceId,
	beld::{self, OutputFormat, Payload},
	resource::{ReDBStorageBackend, WriteStorageBackend as _},
	resources::audio::Audio,
	types::BitDepths,
};
use serde_json::Value;

const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../src/asset/test_data/triangle_move_ascii.fbx");

/// Creates an empty, uniquely named directory for one test.
fn temporary_root(name: &str) -> PathBuf {
	let root = std::env::temp_dir().join(format!(
		"beld-payload-{name}-{}-{}",
		std::process::id(),
		std::time::SystemTime::now()
			.duration_since(std::time::UNIX_EPOCH)
			.unwrap()
			.as_nanos()
	));
	std::fs::create_dir_all(&root).unwrap();
	root
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

/// Bakes `ids` from `root/assets` without a store, as `beld bake --dry-run` does, and returns the exit code and what
/// the dry run wrote.
fn dry_run(
	root: &Path,
	ids: &[&str],
	intercept: &[&str],
	intercept_out: Option<&Path>,
	format: OutputFormat,
	payload: Payload,
) -> (Result<(), i32>, Vec<u8>) {
	let mut output = Vec::new();
	let result = beld::Executor::new().unwrap().block_on(beld::bake_dry_run(
		root.join("assets").to_string_lossy().into_owned(),
		ids.iter().map(|id| id.to_string()).collect(),
		std::num::NonZeroUsize::new(1024 * 1024 * 1024).unwrap(),
		&[],
		configure_default_baking,
		intercept.iter().map(|pattern| pattern.to_string()).collect(),
		intercept_out,
		format,
		payload,
		&mut output,
	));
	(result, output)
}

/// Bakes `ids` as [`dry_run`] does in the JSON format, and returns the exit code and the parsed document.
fn dry_run_json(root: &Path, ids: &[&str], intercept: &[&str], payload: Payload) -> (Result<(), i32>, Option<Value>) {
	let (result, output) = dry_run(root, ids, intercept, None, OutputFormat::JSON, payload);
	(result, serde_json::from_slice(&output).ok())
}

/// Inspects `id` in the store at `root/resources`, as `beld inspect` does, and returns the exit code and the parsed
/// document.
fn inspect(root: &Path, id: &str, format: OutputFormat, payload: Payload) -> (Result<(), i32>, Option<Value>) {
	let mut output = Vec::new();
	let result = beld::Executor::new().unwrap().block_on(beld::inspect(
		root.join("resources").to_string_lossy().into_owned(),
		id.to_string(),
		format,
		payload,
		&mut output,
	));
	(result, serde_json::from_slice(&output).ok())
}

fn stream<'a>(resource: &'a Value, name: &str) -> &'a Value {
	resource["streams"]
		.as_array()
		.unwrap()
		.iter()
		.find(|stream| stream["name"] == name)
		.unwrap_or_else(|| panic!("stream '{name}' is missing from {resource}"))
}

#[test]
fn dry_run_prints_every_baked_resource_with_typed_streams_and_writes_no_store() {
	let root = temporary_root("dry-run-fbx");
	std::fs::create_dir_all(root.join("assets")).unwrap();
	copy_engine_layer_modules(&root.join("assets"));
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let (code, output) = dry_run_json(
		&root,
		&["triangle_move.fbx", "triangle_move.fbx#animations/MoveX"],
		&[],
		Payload::Typed,
	);

	assert_eq!(code, Ok(()));
	assert!(!root.join("resources").exists(), "a dry run must not create the destination");
	let output = output.expect("standard output should hold exactly one JSON document");
	assert_eq!(output["failed"], Value::Array(Vec::new()));

	let resources = output["resources"].as_array().unwrap();
	let ids = resources
		.iter()
		.map(|resource| resource["id"].as_str().unwrap())
		.collect::<Vec<_>>();
	assert!(ids.is_sorted(), "resources should be ordered by ID: {ids:?}");
	assert!(ids.contains(&"triangle_move.fbx#skeleton"));

	let mesh = resources
		.iter()
		.find(|resource| resource["id"] == "triangle_move.fbx")
		.unwrap();
	assert_eq!(mesh["class"], "Mesh");
	let positions = stream(mesh, "level[0].Vertex.Position");
	assert_eq!(
		(&positions["encoding"], &positions["element"]),
		(&"json".into(), &"vec3f".into())
	);
	let vertices = positions["data"].as_array().unwrap();
	assert_eq!(vertices.len() * 12, positions["size"].as_u64().unwrap() as usize);
	assert!(vertices.iter().all(|vertex| vertex.as_array().unwrap().len() == 3));

	let triangles = stream(mesh, "level[0].TriangleIndices");
	assert_eq!(triangles["element"], "u16");
	assert!(triangles["data"].as_array().unwrap().iter().all(Value::is_u64));
	let meshlets = stream(mesh, "level[0].Meshlets");
	assert_eq!(meshlets["element"], "meshlet");
	let first_meshlet = &meshlets["data"][0];
	assert_eq!(first_meshlet["triangle_count"], 1);
	assert_eq!(first_meshlet["center"].as_array().unwrap().len(), 3);

	// Animation curves live in the resource model, so they print as JSON without any payload option.
	let animation = resources
		.iter()
		.find(|resource| resource["id"] == "triangle_move.fbx#animations/MoveX")
		.unwrap();
	assert_eq!(animation["class"], "Animation");
	assert!(!animation["resource"]["tracks"].as_array().unwrap().is_empty());

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn dry_run_prints_a_lut_payload_as_values_or_as_its_exact_bytes() {
	let root = temporary_root("dry-run-lut");
	std::fs::create_dir_all(root.join("assets")).unwrap();
	copy_engine_layer_modules(&root.join("assets"));
	let entries = [[0.0f32, 0.25, 0.5], [1.0, 0.75, 0.1]];
	std::fs::write(root.join("assets/ramp.cube"), "LUT_1D_SIZE 2\n0 0.25 0.5\n1 0.75 0.1\n").unwrap();
	let dry_run = |payload| {
		let (code, output) = dry_run_json(&root, &["ramp.cube"], &[], payload);
		assert_eq!(code, Ok(()));
		let output = output.unwrap();
		let lut = output["resources"]
			.as_array()
			.unwrap()
			.iter()
			.find(|resource| resource["id"] == "ramp.cube")
			.unwrap()
			.clone();
		stream(&lut, "payload").clone()
	};

	let typed = dry_run(Payload::Typed);
	assert_eq!((&typed["encoding"], &typed["element"]), (&"json".into(), &"rgb".into()));
	let values = typed["data"]
		.as_array()
		.unwrap()
		.iter()
		.map(|entry| {
			let entry = entry.as_array().unwrap();
			std::array::from_fn(|channel| entry[channel].as_f64().unwrap() as f32)
		})
		.collect::<Vec<[f32; 3]>>();
	assert_eq!(values, entries);

	let exact = dry_run(Payload::Base64);
	assert_eq!(exact["encoding"], "base64");
	let bytes = base64::decode(exact["data"].as_str().unwrap()).unwrap();
	assert_eq!(
		bytes,
		entries
			.as_flattened()
			.iter()
			.flat_map(|value| value.to_le_bytes())
			.collect::<Vec<_>>()
	);

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn inspect_prints_the_frames_of_a_stored_audio_payload() {
	let root = temporary_root("inspect-audio");
	let samples: [i16; 4] = [1, -2, 300, i16::MIN];
	let executor = beld::Executor::new().unwrap();
	executor.block_on(async {
		let storage = ReDBStorageBackend::new_writable(root.join("resources")).unwrap();
		let bytes = samples.iter().flat_map(|sample| sample.to_le_bytes()).collect::<Vec<_>>();
		let audio = Audio {
			bit_depth: BitDepths::Sixteen,
			channel_count: 2,
			sample_rate: 48_000,
			sample_count: 2,
		};
		storage
			.store(ProcessedAsset::new(ResourceId::new("tone.wav"), audio), &bytes)
			.await
			.unwrap();
	});

	let (code, output) = inspect(&root, "tone.wav", OutputFormat::JSON, Payload::Typed);

	assert_eq!(code, Ok(()));
	let payload = stream(output.as_ref().unwrap(), "payload");
	assert_eq!(payload["element"], "frame");
	assert_eq!(payload["data"], serde_json::json!([[1, -2], [300, -32768]]));

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn payload_output_needs_the_json_format() {
	let root = temporary_root("payload-format");

	let (code, _) = inspect(&root, "tone.wav", OutputFormat::Human, Payload::Typed);

	assert_eq!(code, Err(2));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_intercepted_dry_run_prints_the_besl_of_every_shader_it_compiles() {
	let root = temporary_root("intercept-besl");
	std::fs::create_dir_all(root.join("assets")).unwrap();
	copy_engine_layer_modules(&root.join("assets"));
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let (code, output) = dry_run_json(&root, &["triangle_move.fbx"], &["*.besl"], Payload::None);

	assert_eq!(code, Ok(()));
	assert!(!root.join("resources").exists(), "a dry run must not create the destination");
	let output = output.expect("standard output should hold exactly one JSON document");
	assert_eq!(output["failed"], Value::Array(Vec::new()));
	assert!(
		!output["resources"].as_array().unwrap().is_empty(),
		"an intercepted dry run still lists its resources"
	);
	let intercepted = output["intercepted"].as_array().unwrap();
	assert!(
		!intercepted.is_empty(),
		"the FBX's generated material should compile a shader"
	);
	assert!(
		intercepted
			.iter()
			.any(|product| product["name"].as_str().unwrap().starts_with("triangle_move.fbx#shaders/")),
		"the FBX's generated material should compile its own shader"
	);
	for product in intercepted {
		let name = product["name"].as_str().unwrap();
		// Every bake also assembles the masked passes' coverage shaders from the baked materials.
		assert!(
			(name.starts_with("triangle_move.fbx#shaders/") || name.contains("-coverage.uber#shaders/"))
				&& name.ends_with(".besl"),
			"unexpected product {name}"
		);
		assert_eq!(product["encoding"], "utf-8");
		let source = product["content"].as_str().unwrap();
		besl::parse(source).unwrap_or_else(|error| panic!("{name} should hold BESL source ({error:?}):\n{source}"));
	}

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_intercept_out_directory_receives_every_product_as_a_file() {
	let root = temporary_root("intercept-out");
	std::fs::create_dir_all(root.join("assets")).unwrap();
	copy_engine_layer_modules(&root.join("assets"));
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let (code, output) = dry_run(
		&root,
		&["triangle_move.fbx"],
		&["*"],
		Some(&root.join("products")),
		OutputFormat::Human,
		Payload::None,
	);

	assert_eq!(code, Ok(()));
	let paths = String::from_utf8(output)
		.unwrap()
		.lines()
		.map(|line| root.join(line))
		.collect::<Vec<_>>();
	assert!(
		paths
			.iter()
			.all(|path| path.starts_with(root.join("products")) && path.is_file()),
		"every printed path should be a written file under the output directory: {paths:?}"
	);
	let source = paths
		.iter()
		.find(|path| path.extension().is_some_and(|extension| extension == "besl"))
		.expect("the FBX's generated material should compile a shader");
	besl::parse(&std::fs::read_to_string(source).unwrap()).unwrap();
	// Apple platforms compile to a metallib, which the platform compiler stores with its `MTLB` magic.
	if cfg!(target_vendor = "apple") {
		let binary = paths
			.iter()
			.find(|path| path.extension().is_some_and(|extension| extension == "metallib"))
			.expect("the compiled shader should be written next to its source");
		assert!(std::fs::read(binary).unwrap().starts_with(b"MTLB"));
	}

	std::fs::remove_dir_all(root).unwrap();
}
