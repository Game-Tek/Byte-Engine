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
}
