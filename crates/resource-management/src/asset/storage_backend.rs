use std::{alloc::Allocator, ops::Deref, path::PathBuf};

use super::{read_asset_from_source, BEADType, ResourceId};
use crate::{
	r#async::{future, BoxedFuture},
	resource::reader::MappedFileBacking,
};

/// The `AssetStorageBytes` enum owns asset source storage while exposing it as a borrowed byte slice.
#[derive(Debug)]
pub enum AssetStorageBytes<'a> {
	Owned(Box<[u8]>),
	Allocated(Box<[u8], &'a dyn Allocator>),
	MappedFile(MappedFileBacking),
}

impl AssetStorageBytes<'_> {
	/// Returns the asset source bytes from the current backing storage.
	pub fn as_slice(&self) -> &[u8] {
		match self {
			AssetStorageBytes::Owned(bytes) => bytes,
			AssetStorageBytes::Allocated(bytes) => bytes,
			AssetStorageBytes::MappedFile(mapped_file) => mapped_file.as_slice(),
		}
	}
}

impl AsRef<[u8]> for AssetStorageBytes<'_> {
	fn as_ref(&self) -> &[u8] {
		self.as_slice()
	}
}

impl Deref for AssetStorageBytes<'_> {
	type Target = [u8];

	fn deref(&self) -> &Self::Target {
		self.as_slice()
	}
}

type ResolveResult<'a> = Result<(AssetStorageBytes<'a>, Option<BEADType>, String), ()>;

pub trait StorageBackend: Send + Sync {
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>> {
		future(read_asset_from_source(url, None, &std::alloc::Global))
	}

	/// Resolves an asset while using the provided allocator for source bytes.
	fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>> {
		future(read_asset_from_source(url, None, allocator))
	}
}

pub struct FileStorageBackend {
	base_path: PathBuf,
}

impl FileStorageBackend {
	pub fn new(base_path: PathBuf) -> Self {
		std::fs::create_dir_all(&base_path).expect("Failed to create base path");

		Self { base_path }
	}
}

impl StorageBackend for FileStorageBackend {
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>> {
		future(read_asset_from_source(url, Some(&self.base_path), &std::alloc::Global))
	}

	fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>> {
		future(read_asset_from_source(url, Some(&self.base_path), allocator))
	}
}

#[cfg(test)]
fn move_bytes_in<'a>(bytes: impl AsRef<[u8]>, allocator: &'a dyn Allocator) -> AssetStorageBytes<'a> {
	let bytes = bytes.as_ref();
	let mut output = Vec::with_capacity_in(bytes.len(), allocator);
	output.extend_from_slice(bytes);
	AssetStorageBytes::Allocated(output.into_boxed_slice())
}

#[cfg(test)]
pub mod tests {
	use std::{
		alloc::Allocator,
		collections::HashMap,
		fs,
		sync::{Arc, Mutex},
		time::{SystemTime, UNIX_EPOCH},
	};

	use utils::json;

	use super::{AssetStorageBytes, FileStorageBackend, ResolveResult, StorageBackend};
	use crate::{
		asset::ResourceId,
		r#async::{read, BoxedFuture},
		tests::ASSETS_PATH,
	};

	/// A storage backend that can be used for tests.
	/// It allows you to add files to the storage backend. This way you can test custom files without having to create them on the filesystem.
	/// For any requested file that was not "mocked" it will try to read the file from the assets directory.
	#[derive(Clone)]
	pub struct TestStorageBackend(Arc<Mutex<HashMap<String, Box<[u8]>>>>);

	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::new())))
		}

		pub fn add_file(&self, name: &'static str, data: &[u8]) {
			self.0.lock().unwrap().insert(name.to_string(), data.into());
		}
	}

	impl StorageBackend for TestStorageBackend {
		fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>> {
			Box::pin(async move {
				if let Some(data) = self.0.lock().unwrap().get(url.as_ref()).cloned() {
					return Ok((AssetStorageBytes::Owned(data), None, url.get_extension().to_string()));
				}

				// NOTE: Don't return value from else because it would be a reborrow of self.0.lock().unwrap()

				let path = std::path::Path::new(ASSETS_PATH);
				let path = path.join(url.get_base().as_ref());

				// Check if the file name exitst in our map
				let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");

				let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();

				// If case file needs to be looked for in the fs use the real path
				let spec_path = path.with_added_extension("bead");

				let spec = if let Some(data) = spec_data {
					let spec = std::str::from_utf8(&data).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
						Err(_) => return Err(()),
					};

					if let Some(spec_bytes) = spec_bytes {
						let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
						let spec: json::Value = json::from_str(spec).or(Err(()))?;
						Some(spec)
					} else {
						None
					}
				};

				let format = path
					.extension()
					.and_then(|extension| extension.to_str())
					.unwrap_or_default()
					.to_string();

				let source_bytes = read(&path).await.or(Err(()))?;

				Ok((AssetStorageBytes::Owned(source_bytes.into_boxed_slice()), spec, format))
			})
		}

		fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>> {
			Box::pin(async move {
				if let Some(data) = self.0.lock().unwrap().get(url.as_ref()).cloned() {
					return Ok((super::move_bytes_in(data, allocator), None, url.get_extension().to_string()));
				}

				// NOTE: Don't return value from else because it would be a reborrow of self.0.lock().unwrap()

				let path = std::path::Path::new(ASSETS_PATH);
				let path = path.join(url.get_base().as_ref());

				// Check if the file name exists in our map.
				let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");

				let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();

				// If the file needs to be looked for in the fs use the real path.
				let spec_path = path.with_added_extension("bead");

				let spec = if let Some(data) = spec_data {
					let spec = std::str::from_utf8(&data).or(Err(()))?;
					let spec: json::Value = json::from_str(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
						Err(_) => return Err(()),
					};

					if let Some(spec_bytes) = spec_bytes {
						let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
						let spec: json::Value = json::from_str(spec).or(Err(()))?;
						Some(spec)
					} else {
						None
					}
				};

				let format = path
					.extension()
					.and_then(|extension| extension.to_str())
					.unwrap_or_default()
					.to_string();

				let source_bytes = read(&path).await.or(Err(()))?;

				Ok((super::move_bytes_in(source_bytes, allocator), spec, format))
			})
		}
	}

	fn temporary_asset_directory() -> std::path::PathBuf {
		std::env::temp_dir().join(format!(
			"byte-engine-asset-storage-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		))
	}

	#[crate::r#async::test]
	async fn file_storage_backend_resolves_assets_as_mapped_slices() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(&directory).unwrap();
		let path = directory.join("shader.bin");
		let expected = b"asset-bytes";
		fs::write(&path, expected).unwrap();

		let storage_backend = FileStorageBackend::new(directory.clone());
		let (bytes, spec, format) = storage_backend
			.resolve(ResourceId::new("shader.bin"))
			.await
			.expect("asset should resolve");

		assert!(matches!(bytes, AssetStorageBytes::MappedFile(_)));
		assert_eq!(bytes.as_slice(), expected);
		assert!(spec.is_none());
		assert_eq!(format, "bin");

		fs::remove_dir_all(directory).unwrap();
	}

	#[crate::r#async::test]
	async fn file_storage_backend_resolves_extensionless_dependency_bytes() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(&directory).unwrap();
		let path = directory.join("skeleton");
		let expected = b"buffer-bytes";
		fs::write(&path, expected).unwrap();

		let storage_backend = FileStorageBackend::new(directory.clone());
		let (bytes, spec, format) = storage_backend
			.resolve(ResourceId::new("skeleton"))
			.await
			.expect("extensionless dependency should resolve");

		assert_eq!(bytes.as_slice(), expected);
		assert!(spec.is_none());
		assert_eq!(format, "");

		fs::remove_dir_all(directory).unwrap();
	}
}
