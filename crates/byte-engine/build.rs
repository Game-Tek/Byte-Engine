use std::{env, fs, path::Path, path::PathBuf};

const FNV_OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
const RESOURCE_PRODUCER_FILES: [&str; 3] = [
	"src/rendering/common_shader_generator.rs",
	"src/rendering/pipelines/visibility/mod.rs",
	"src/rendering/pipelines/visibility/shader_generator.rs",
];

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
	for byte in bytes {
		*hash ^= u64::from(*byte);
		*hash = hash.wrapping_mul(FNV_PRIME);
	}
}

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

		println!("cargo:rerun-if-changed={}", relative_path.display());
		hash_bytes(hash, relative_path.to_string_lossy().as_bytes());
		hash_bytes(hash, &[0]);
		hash_bytes(hash, &fs::read(entry.path()).unwrap());
		hash_bytes(hash, &[0]);
	}
}

fn main() {
	let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());

	// Generated materials persist the visibility scope, its binding constants, and common helpers in the resource database.
	// Hashing that direct producer boundary makes an implementation edit invalidate retained values on the next debug run.
	let mut hash = FNV_OFFSET_BASIS;
	for relative_path in RESOURCE_PRODUCER_FILES {
		println!("cargo:rerun-if-changed={relative_path}");

		hash_bytes(&mut hash, relative_path.as_bytes());
		hash_bytes(&mut hash, &[0]);
		hash_bytes(&mut hash, &fs::read(manifest_dir.join(relative_path)).unwrap());
		hash_bytes(&mut hash, &[0]);
	}
	// Engine-owned source assets participate in debug lazy baking. Changing either a shader or its BEAD stage metadata must
	// invalidate the retained debug database so the next request compiles the new source instead of reusing stale bytes.
	hash_directory(&mut hash, &manifest_dir, Path::new("assets"));

	println!("cargo:rustc-env=BYTE_ENGINE_RESOURCE_PRODUCER_HASH={hash:016x}");
}
