use std::{
	env, fs,
	path::{Path, PathBuf},
};

fn collect_files(path: &Path, files: &mut Vec<PathBuf>) {
	if path.is_dir() {
		for entry in fs::read_dir(path).unwrap() {
			let entry = entry.unwrap();
			collect_files(&entry.path(), files);
		}
	} else if path.is_file() {
		files.push(path.to_path_buf());
	}
}

fn main() {
	let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
	let mut files = Vec::new();

	collect_files(&manifest_dir.join("src"), &mut files);
	files.push(manifest_dir.join("Cargo.toml"));
	files.sort();

	let mut context = md5::Context::new();

	for file in files {
		let relative_path = file.strip_prefix(&manifest_dir).unwrap();
		println!("cargo:rerun-if-changed={}", relative_path.display());

		context.consume(relative_path.to_string_lossy().as_bytes());
		context.consume([0]);
		context.consume(fs::read(&file).unwrap());
		context.consume([0]);
	}

	println!("cargo:rustc-env=RESOURCE_MANAGEMENT_CODE_HASH={:x}", context.finalize());

	// The BESL crate shapes every compiled shader but not the layout of the store, so its changes make shaders stale
	// instead of resetting the store. Outside the workspace the crate's sources aren't beside this one, and its version
	// stands in for them.
	let besl = manifest_dir.join("../besl/src");
	let mut files = Vec::new();
	if besl.is_dir() {
		collect_files(&besl, &mut files);
		files.sort();
	}

	let mut context = md5::Context::new();
	context.consume(env::var("CARGO_PKG_VERSION").unwrap().as_bytes());
	for file in files {
		println!("cargo:rerun-if-changed={}", file.display());

		context.consume(file.strip_prefix(&besl).unwrap().to_string_lossy().as_bytes());
		context.consume([0]);
		context.consume(fs::read(&file).unwrap());
		context.consume([0]);
	}

	println!("cargo:rustc-env=BESL_CODE_HASH={:x}", context.finalize());
}
