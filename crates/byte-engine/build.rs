use std::{env, fs, path::PathBuf};

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

	println!("cargo:rustc-env=BYTE_ENGINE_RESOURCE_PRODUCER_HASH={hash:016x}");
}
