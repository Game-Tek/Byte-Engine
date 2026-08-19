/// The `StorageBackend` trait provides source resolution and cheap version checks for asset baking.
pub trait StorageBackend: Send + Sync {
	/// Returns the local directory that can be watched for development asset changes.
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf> {
		None
	}

	/// Reports whether a source directory exists and can be read when the backend exposes paths.
	fn directory_accessible(&self, _path: &Path) -> Option<bool> {
		None
	}

	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = ResolveResult<'a>> + 'a {
		read_asset_from_source(url, None, &std::alloc::Global)
	}

	/// Resolves an asset while using the provided allocator for source bytes.
	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> impl Future<Output = ResolveResult<'a>> + 'a {
		read_asset_from_source(url, None, allocator)
	}

	/// Returns the source version used to decide whether an existing baked resource is still fresh.
	///
	/// Backends with native metadata should override this method to avoid reading source bytes.
	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = Result<AssetVersion, ()>> + 'a {
		async move {
			let (source, sidecar, _) = self.resolve(url).await?;
			AssetVersion::from_resolved(&source, sidecar.as_ref())
		}
	}
}

pub trait DynStorageBackend: Send + Sync {
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf>;
	fn directory_accessible(&self, path: &Path) -> Option<bool>;
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>>;
	fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>>;
	fn version<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<AssetVersion, ()>>;
}

impl<T: StorageBackend> DynStorageBackend for T {
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf> {
		self.watch_root()
	}

	fn directory_accessible(&self, path: &Path) -> Option<bool> {
		self.directory_accessible(path)
	}

	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>> {
		Box::pin(self.resolve(url))
	}

	fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>> {
		Box::pin(self.resolve_in(url, allocator))
	}

	fn version<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<AssetVersion, ()>> {
		Box::pin(self.version(url))
	}
}

/// The metadata implementation used to version local source files.
pub type AssetMetadata = compio::fs::Metadata;

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

/// The `AssetFileVersion` struct captures the metadata or content identity used to compare one source file.
#[derive(
	Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub(crate) struct AssetFileVersion {
	size: u64,
	modified: Option<(u64, u32)>,
	content_hash: Option<u64>,
}

impl AssetFileVersion {
	/// Creates a version from file metadata for inexpensive debug freshness checks.
	fn from_metadata(metadata: &AssetMetadata) -> Self {
		let modified = metadata
			.modified()
			.ok()
			.and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
			.map(|modified| (modified.as_secs(), modified.subsec_nanos()));

		Self {
			size: metadata.len(),
			modified,
			content_hash: None,
		}
	}

	/// Creates a content-backed version for storage backends that do not expose file metadata.
	fn from_bytes(bytes: &[u8]) -> Self {
		let mut hasher = GxHasher::with_seed(961961961961961);
		hasher.write(bytes);

		Self {
			size: bytes.len() as u64,
			modified: None,
			content_hash: Some(hasher.finish()),
		}
	}
}

/// The `AssetVersion` struct identifies the source and optional BEAD sidecar used to bake one asset.
///
/// Return this value from [`StorageBackend::version`] when a custom backend can provide cheaper identity metadata.
#[derive(
	Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct AssetVersion {
	source: AssetFileVersion,
	sidecar: Option<AssetFileVersion>,
}

impl AssetVersion {
	/// Creates a version from local file metadata without reading either file.
	pub fn from_metadata(source: &AssetMetadata, sidecar: Option<&AssetMetadata>) -> Self {
		Self {
			source: AssetFileVersion::from_metadata(source),
			sidecar: sidecar.map(AssetFileVersion::from_metadata),
		}
	}

	/// Creates a version from source and optional sidecar content.
	pub fn from_content(source: &[u8], sidecar: Option<&[u8]>) -> Self {
		Self {
			source: AssetFileVersion::from_bytes(source),
			sidecar: sidecar.map(AssetFileVersion::from_bytes),
		}
	}

	/// Builds a version from resolved bytes when the backend cannot provide file metadata.
	fn from_resolved(source: &[u8], sidecar: Option<&BEADType>) -> Result<Self, ()> {
		let sidecar = sidecar.map(serde_json::to_vec).transpose().map_err(|_| ())?;

		Ok(Self::from_content(source, sidecar.as_deref()))
	}
}

/// The `AssetDependency` struct records one source asset and the version used by a baked resource.
#[derive(
	Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub(crate) struct AssetDependency {
	id: String,
	version: AssetVersion,
}

impl AssetDependency {
	/// Creates persisted provenance for one resolved source asset.
	pub(crate) fn new(id: ResourceId<'_>, version: AssetVersion) -> Self {
		Self {
			id: id.get_base().as_ref().to_string(),
			version,
		}
	}

	/// Returns the source ID used for a freshness check.
	pub(crate) fn id(&self) -> &str {
		&self.id
	}

	/// Returns the recorded source version.
	pub(crate) fn version(&self) -> &AssetVersion {
		&self.version
	}
}

type ResolveResult<'a> = Result<(AssetStorageBytes<'a>, Option<BEADType>, String), ()>;

/// The `FileStorageBackend` struct resolves source assets relative to one local directory.
pub struct FileStorageBackend {
	base_path: PathBuf,
}

impl FileStorageBackend {
	/// Creates a local source backend rooted at `base_path`.
	pub fn new(base_path: PathBuf) -> Self {
		std::fs::create_dir_all(&base_path).expect("Failed to create base path");

		Self { base_path }
	}
}

impl StorageBackend for FileStorageBackend {
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf> {
		Some(self.base_path.clone())
	}

	fn directory_accessible(&self, path: &Path) -> Option<bool> {
		let path = self.base_path.join(path);
		Some(path.is_dir() && std::fs::read_dir(path).is_ok())
	}

	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = ResolveResult<'a>> + 'a {
		future(read_asset_from_source(url, Some(&self.base_path), &std::alloc::Global))
	}

	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> impl Future<Output = ResolveResult<'a>> + 'a {
		future(read_asset_from_source(url, Some(&self.base_path), allocator))
	}

	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = Result<AssetVersion, ()>> + 'a {
		future(async move {
			let source_path = self.base_path.join(url.get_base().as_ref());
			let sidecar_path = source_path.with_added_extension("bead");

			let source = AsyncFile::open(source_path).await.map_err(|_| ())?;
			let source = source.metadata().await.map_err(|_| ())?;
			let sidecar = match AsyncFile::open(sidecar_path).await {
				Ok(file) => Some(file.metadata().await.map_err(|_| ())?),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
				Err(_) => return Err(()),
			};

			Ok(AssetVersion::from_metadata(&source, sidecar.as_ref()))
		})
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
		fs::{self, FileTimes, OpenOptions},
		io::Write,
		sync::{Arc, Mutex},
		time::{Duration, SystemTime, UNIX_EPOCH},
	};

	use super::{parse_json, AssetStorageBytes, FileStorageBackend, ResolveResult, StorageBackend};
	use crate::{
		asset::ResourceId,
		r#async::{read, BoxedFuture},
		tests::ASSETS_PATH,
	};

	/// The `TestStorageBackend` struct provides in-memory source files with an asset-directory fallback for tests.
	#[derive(Clone)]
	pub struct TestStorageBackend(Arc<Mutex<HashMap<String, Box<[u8]>>>>);

	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::new())))
		}

		pub fn add_file(&self, name: &'static str, data: &[u8]) {
			self.0.lock().unwrap().insert(name.to_string(), data.into());
		}

		pub fn remove_file(&self, name: &str) {
			self.0.lock().unwrap().remove(name);
		}
	}

	impl StorageBackend for TestStorageBackend {
		fn resolve<'a>(&'a self, url: ResourceId<'a>) -> impl std::future::Future<Output = ResolveResult<'a>> + 'a {
			Box::pin(async move {
				let mocked_data = { self.0.lock().unwrap().get(url.as_ref()).cloned() };
				if let Some(data) = mocked_data {
					let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");
					let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();
					let spec = if let Some(spec_data) = spec_data {
						let spec = std::str::from_utf8(&spec_data).or(Err(()))?;
						Some(parse_json(spec).or(Err(()))?)
					} else {
						None
					};
					return Ok((AssetStorageBytes::Owned(data), spec, url.get_extension().to_string()));
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
					let spec = parse_json(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
						Err(_) => return Err(()),
					};

					if let Some(spec_bytes) = spec_bytes {
						let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
						let spec = parse_json(spec).or(Err(()))?;
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

		fn resolve_in<'a>(
			&'a self,
			url: ResourceId<'a>,
			allocator: &'a dyn Allocator,
		) -> impl std::future::Future<Output = ResolveResult<'a>> + 'a {
			Box::pin(async move {
				let mocked_data = { self.0.lock().unwrap().get(url.as_ref()).cloned() };
				if let Some(data) = mocked_data {
					let spec_path = std::path::Path::new(url.get_base().as_ref()).with_added_extension("bead");
					let spec_data = self.0.lock().unwrap().get(spec_path.to_str().unwrap()).cloned();
					let spec = if let Some(spec_data) = spec_data {
						let spec = std::str::from_utf8(&spec_data).or(Err(()))?;
						Some(parse_json(spec).or(Err(()))?)
					} else {
						None
					};
					return Ok((super::move_bytes_in(data, allocator), spec, url.get_extension().to_string()));
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
					let spec = parse_json(spec).or(Err(()))?;
					Some(spec)
				} else {
					let spec_bytes = match read(&spec_path).await {
						Ok(bytes) => Some(bytes),
						Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
						Err(_) => return Err(()),
					};

					if let Some(spec_bytes) = spec_bytes {
						let spec = std::str::from_utf8(&spec_bytes).or(Err(()))?;
						let spec = parse_json(spec).or(Err(()))?;
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

	#[crate::r#async::test]
	async fn file_asset_versions_cover_size_modification_time_and_sidecar_presence() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(&directory).unwrap();
		let source_path = directory.join("shader.bin");
		fs::write(&source_path, b"same").unwrap();
		let storage_backend = FileStorageBackend::new(directory.clone());

		let first = storage_backend
			.version(ResourceId::new("shader.bin"))
			.await
			.expect("source metadata should be available");

		assert_eq!(first.source.size, 4);
		assert!(first.sidecar.is_none());

		let mut source = OpenOptions::new().write(true).truncate(true).open(&source_path).unwrap();
		source.write_all(b"size").unwrap();
		source
			.set_times(FileTimes::new().set_modified(SystemTime::now() + Duration::from_secs(2)))
			.unwrap();
		let modified = storage_backend
			.version(ResourceId::new("shader.bin"))
			.await
			.expect("modified source metadata should be available");

		assert_ne!(first, modified);

		fs::write(directory.join("shader.bin.bead"), b"{}").unwrap();
		let with_sidecar = storage_backend
			.version(ResourceId::new("shader.bin"))
			.await
			.expect("sidecar metadata should be available");

		assert!(with_sidecar.sidecar.is_some());

		fs::remove_dir_all(directory).unwrap();
	}
}

use std::{
	alloc::Allocator,
	future::Future,
	hash::Hasher,
	ops::Deref,
	path::{Path, PathBuf},
	time::UNIX_EPOCH,
};

use gxhash::GxHasher;

use super::{parse_json, read_asset_from_source, BEADType, ResourceId};
use crate::{
	r#async::{future, BoxedFuture, File as AsyncFile},
	resource::reader::MappedFileBacking,
};
