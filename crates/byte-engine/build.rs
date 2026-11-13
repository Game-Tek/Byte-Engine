use std::{
	env, fs,
	path::{Path, PathBuf},
};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
	for byte in bytes {
		*hash ^= u64::from(*byte);
		*hash = hash.wrapping_mul(FNV_PRIME);
	}
}

/// Hashes every file below `directory`, in name order, with its path from the crate root.
fn hash_directory(hash: &mut u64, manifest_dir: &Path, directory: &Path) {
	let mut entries = fs::read_dir(manifest_dir.join(directory))
		.unwrap()
		.map(|entry| entry.unwrap())
		.collect::<Vec<_>>();
	entries.sort_by_key(|entry| entry.file_name());

	for entry in entries {
		let relative_path = directory.join(entry.file_name());
		if entry.file_type().unwrap().is_dir() {
			hash_directory(hash, manifest_dir, &relative_path);
			continue;
		}

		hash_bytes(hash, relative_path.to_string_lossy().as_bytes());
		hash_bytes(hash, &[0]);
		hash_bytes(hash, &fs::read(entry.path()).unwrap());
		hash_bytes(hash, &[0]);
	}
}

fn main() {
	let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

	// The shader generators the asset bakes run live under `src/rendering`, and what they generate is stored in the
	// resources of every project. The hash of the whole directory is part of the generators' identity, so an edit to
	// any of its files makes the shaders baked before it stale. A list of files would miss a helper in a file nobody
	// listed.
	let rendering = Path::new("src/rendering");
	let mut hash = FNV_OFFSET_BASIS;
	hash_directory(&mut hash, &manifest_dir, rendering);

	// Cargo scans a directory for changes to any file inside it.
	println!("cargo:rerun-if-changed={}", rendering.display());
	println!("cargo:rustc-env=BYTE_ENGINE_RENDERING_CODE_HASH={hash:016x}");
}
