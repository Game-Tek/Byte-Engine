//! Black-box tests of the `beld` binary's dry-run bake, payload, and intercept output, as specified in the BELD
//! command-line reference.

use std::{path::PathBuf, process::Command};

use resource_management::{
	ProcessedAsset,
	asset::ResourceId,
	resource::{ReDBStorageBackend, WriteStorageBackend as _},
	resources::audio::Audio,
	types::BitDepths,
};
use serde_json::Value;

const TRIANGLE_MOVE_FBX: &[u8] = include_bytes!("../../resource-management/src/asset/test_data/triangle_move_ascii.fbx");

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

/// Runs BELD in `root`, so it finds no project configuration, with `root/assets` and `root/resources` as its
/// directories, and returns its exit code and parsed standard output.
fn run_beld(root: &PathBuf, arguments: &[&str]) -> (Option<i32>, Option<Value>) {
	let output = Command::new(env!("CARGO_BIN_EXE_beld"))
		.current_dir(root)
		.args(["--source", "assets", "--destination", "resources"])
		.args(arguments)
		.output()
		.expect("BELD could not be started. The most likely cause is that the test binary was not built.");
	let stdout = serde_json::from_slice(&output.stdout).ok();
	(output.status.code(), stdout)
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
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let (code, output) = run_beld(
		&root,
		&[
			"bake",
			"--dry-run",
			"--format",
			"json",
			"--payload",
			"typed",
			"triangle_move.fbx",
			"triangle_move.fbx#animations/MoveX",
		],
	);

	assert_eq!(code, Some(0));
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
	let entries = [[0.0f32, 0.25, 0.5], [1.0, 0.75, 0.1]];
	std::fs::write(root.join("assets/ramp.cube"), "LUT_1D_SIZE 2\n0 0.25 0.5\n1 0.75 0.1\n").unwrap();
	let dry_run = |payload| {
		let (code, output) = run_beld(
			&root,
			&["bake", "--dry-run", "--format", "json", "--payload", payload, "ramp.cube"],
		);
		assert_eq!(code, Some(0));
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

	let typed = dry_run("typed");
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

	let exact = dry_run("base64");
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

	let (code, output) = run_beld(&root, &["inspect", "tone.wav", "--format", "json", "--payload", "typed"]);

	assert_eq!(code, Some(0));
	let payload = stream(output.as_ref().unwrap(), "payload");
	assert_eq!(payload["element"], "frame");
	assert_eq!(payload["data"], serde_json::json!([[1, -2], [300, -32768]]));

	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn payload_output_needs_the_json_format() {
	let root = temporary_root("payload-format");

	let (code, _) = run_beld(&root, &["inspect", "tone.wav", "--payload", "typed"]);

	assert_eq!(code, Some(2));
	std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn an_intercepted_dry_run_prints_the_besl_of_every_shader_it_compiles() {
	let root = temporary_root("intercept-besl");
	std::fs::create_dir_all(root.join("assets")).unwrap();
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let (code, output) = run_beld(
		&root,
		&[
			"bake",
			"--dry-run",
			"--intercept",
			"*.besl",
			"--format",
			"json",
			"triangle_move.fbx",
		],
	);

	assert_eq!(code, Some(0));
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
	std::fs::write(root.join("assets/triangle_move.fbx"), TRIANGLE_MOVE_FBX).unwrap();

	let output = Command::new(env!("CARGO_BIN_EXE_beld"))
		.current_dir(&root)
		.args([
			"--source",
			"assets",
			"bake",
			"--dry-run",
			"--intercept",
			"*",
			"--intercept-out",
			"products",
			"triangle_move.fbx",
		])
		.output()
		.expect("BELD could not be started. The most likely cause is that the test binary was not built.");

	assert_eq!(output.status.code(), Some(0));
	let paths = String::from_utf8(output.stdout)
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

#[test]
fn intercept_needs_a_dry_run() {
	let root = temporary_root("intercept-dry-run");

	let (code, _) = run_beld(&root, &["bake", "--intercept", "*.besl", "triangle_move.fbx"]);

	assert_eq!(code, Some(2));
	std::fs::remove_dir_all(root).unwrap();
}
