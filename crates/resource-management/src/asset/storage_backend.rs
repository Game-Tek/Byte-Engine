/// The `AssetSource` struct identifies one enumerable source asset and whether its BEAD sidecar exists.
#[derive(Debug, Eq, PartialEq)]
pub struct AssetSource {
	id: String,
	has_sidecar: bool,
}

impl AssetSource {
	/// Creates an enumerable source description for an asset storage backend.
	pub fn new(id: String, has_sidecar: bool) -> Self {
		Self { id, has_sidecar }
	}

	/// Returns the source resource ID.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Returns whether the source has a BEAD sidecar.
	pub fn has_sidecar(&self) -> bool {
		self.has_sidecar
	}

	/// Returns the owned source resource ID.
	pub fn into_id(self) -> String {
		self.id
	}
}

/// The `StorageBackend` trait provides source resolution and cheap version checks for asset baking.
pub trait StorageBackend: Send + Sync {
	/// Enumerates source assets when the backend exposes a discoverable asset namespace.
	///
	/// Pass the result to [`crate::asset::manager::AssetManager::should_discover`] so registered handlers select bakeable
	/// sources.
	fn discover(&self) -> impl Future<Output = Result<Vec<AssetSource>, String>> {
		async {
			Err(
				"Asset discovery is unavailable. The most likely cause is that the storage backend does not expose an enumerable source namespace."
					.to_string(),
			)
		}
	}

	/// Returns the local directory that can be watched for development asset changes.
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf> {
		None
	}

	/// Reports whether a source directory exists and can be read when the backend exposes paths.
	fn directory_accessible(&self, _path: &Path) -> Option<bool> {
		None
	}

	/// Loads only the requested source. Call [`Self::load_sidecar`] explicitly for BEAD settings.
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = ResolveResult<'a>> + 'a;

	/// Loads source bytes with the provided allocator. Custom backends can override this to avoid copying.
	fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> impl Future<Output = ResolveResult<'a>> + 'a {
		async move {
			let (source, format) = self.resolve(url).await?;
			let mut allocated = Vec::new_in(allocator);
			allocated.try_reserve_exact(source.len()).map_err(|_| ())?;
			allocated.extend_from_slice(&source);
			Ok((AssetStorageBytes::Allocated(allocated.into_boxed_slice()), format))
		}
	}

	/// Loads optional JSON5 settings adjacent to the source, using this backend's namespace.
	///
	/// During baking, use [`crate::asset::handler::BakeContext::load_sidecar`] to record the sidecar dependency,
	/// including its absence. Read the source separately with [`Self::resolve`].
	fn load_sidecar<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = Result<Option<BEADType>, ()>> + 'a {
		async move {
			let path = format!("{}.bead", url.get_base().as_ref());
			let id = ResourceId::new(&path);
			if self.version(id).await?.source.is_none() {
				return Ok(None);
			}
			let (bytes, _) = self.resolve(id).await?;
			let text = std::str::from_utf8(&bytes).map_err(|_| ())?;
			parse_json(text).map(Some).map_err(|_| ())
		}
	}

	/// Returns the exact file's version, including an absent file, without reading adjacent files.
	///
	/// Return [`AssetVersion::missing`] only for missing files; propagate other access errors.
	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl Future<Output = Result<AssetVersion, ()>> + 'a;
}

/// The `DynStorageBackend` trait provides object-safe asset source resolution and version checks.
pub trait DynStorageBackend: Send + Sync {
	fn discover(&self) -> BoxedFuture<'_, Result<Vec<AssetSource>, String>>;
	#[cfg(debug_assertions)]
	fn watch_root(&self) -> Option<PathBuf>;
	fn directory_accessible(&self, path: &Path) -> Option<bool>;
	fn resolve<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, ResolveResult<'a>>;
	fn resolve_in<'a>(&'a self, url: ResourceId<'a>, allocator: &'a dyn Allocator) -> BoxedFuture<'a, ResolveResult<'a>>;
	fn load_sidecar<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<Option<BEADType>, ()>>;
	fn version<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<AssetVersion, ()>>;
}

impl<T: StorageBackend> DynStorageBackend for T {
	fn discover(&self) -> BoxedFuture<'_, Result<Vec<AssetSource>, String>> {
		Box::pin(self.discover())
	}

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

	fn load_sidecar<'a>(&'a self, url: ResourceId<'a>) -> BoxedFuture<'a, Result<Option<BEADType>, ()>> {
		Box::pin(self.load_sidecar(url))
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
		// Changing the content hasher makes older stored freshness hashes stale on the next comparison.
		let mut hasher = FxHasher::with_seed(961961961961961);
		hasher.write(bytes);

		Self {
			size: bytes.len() as u64,
			modified: None,
			content_hash: Some(hasher.finish()),
		}
	}
}

/// The `AssetVersion` struct preserves one requested file's identity for freshness checks.
///
/// Return this value from [`StorageBackend::version`], including when an optional file is missing.
#[derive(
	Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize,
)]
pub struct AssetVersion {
	source: Option<AssetFileVersion>,
}

impl AssetVersion {
	/// Creates a version from local file metadata.
	pub fn from_metadata(source: &AssetMetadata) -> Self {
		Self {
			source: Some(AssetFileVersion::from_metadata(source)),
		}
	}

	/// Creates a version from the exact file's content.
	pub fn from_content(source: &[u8]) -> Self {
		Self {
			source: Some(AssetFileVersion::from_bytes(source)),
		}
	}

	/// Records absence so creating an explicitly requested optional file invalidates its dependents.
	pub fn missing() -> Self {
		Self { source: None }
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

type ResolveResult<'a> = Result<(AssetStorageBytes<'a>, String), ()>;

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

	/// Creates the source directory asynchronously before returning its local backend.
	///
	/// Use [`StorageBackend::resolve`] next to read an asset relative to this directory.
	pub async fn open(base_path: PathBuf) -> std::io::Result<Self> {
		compio::fs::create_dir_all(&base_path).await?;

		Ok(Self { base_path })
	}
}

impl StorageBackend for FileStorageBackend {
	fn discover(&self) -> impl Future<Output = Result<Vec<AssetSource>, String>> {
		let root = self.base_path.clone();

		async move {
			// Compio does not provide directory iteration or canonicalization, so keep the complete blocking walk on one worker.
			match crate::r#async::offload(move || discover_file_sources(&root)).await {
				Ok(result) => result,
				Err(error) => {
					error.resume_unwind();
					Err("Asset discovery stopped. The most likely cause is that its blocking worker was cancelled.".to_string())
				}
			}
		}
	}

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
			let path = self.base_path.join(url.get_base().as_ref());
			match AsyncFile::open(path).await {
				Ok(file) => Ok(AssetVersion::from_metadata(&file.metadata().await.map_err(|_| ())?)),
				Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AssetVersion::missing()),
				Err(_) => Err(()),
			}
		})
	}
}

/// Finds local source files without revisiting an active symlink ancestor.
fn discover_file_sources(root: &Path) -> Result<Vec<AssetSource>, String> {
	let canonical_root = std::fs::canonicalize(root).map_err(|error| {
		format!(
			"Failed to resolve assets directory '{}'. The most likely cause is that the directory does not exist or cannot be accessed. Error: {error}",
			root.display()
		)
	})?;
	let mut active_directories = std::collections::HashSet::from([canonical_root]);
	let mut sources = Vec::new();

	discover_file_sources_in(root, root, &mut active_directories, &mut sources)?;

	Ok(sources)
}

/// Adds files from one directory while preserving the logical path used to enter symlinked directories.
fn discover_file_sources_in(
	root: &Path,
	directory: &Path,
	active_directories: &mut std::collections::HashSet<PathBuf>,
	sources: &mut Vec<AssetSource>,
) -> Result<(), String> {
	let entries = std::fs::read_dir(directory).map_err(|error| {
		format!(
			"Failed to scan assets directory '{}'. The most likely cause is that the directory cannot be read. Error: {error}",
			directory.display()
		)
	})?;

	for entry in entries {
		let entry = entry.map_err(|error| {
			format!(
				"Failed to read an entry in assets directory '{}'. The most likely cause is that the directory changed during discovery. Error: {error}",
				directory.display()
			)
		})?;
		let path = directory.join(entry.file_name());
		let file_type = entry.file_type().map_err(|error| {
			format!(
				"Failed to inspect asset path '{}'. The most likely cause is that the path changed during discovery. Error: {error}",
				path.display()
			)
		})?;
		let (is_directory, is_file) = if file_type.is_symlink() {
			let metadata = std::fs::metadata(&path).map_err(|error| {
				format!(
					"Failed to follow asset symlink '{}'. The most likely cause is a broken link or inaccessible target. Error: {error}",
					path.display()
				)
			})?;

			(metadata.is_dir(), metadata.is_file())
		} else {
			(file_type.is_dir(), file_type.is_file())
		};

		if is_directory {
			let canonical_directory = std::fs::canonicalize(&path).map_err(|error| {
				format!(
					"Failed to resolve asset directory '{}'. The most likely cause is a broken symlink or inaccessible directory. Error: {error}",
					path.display()
				)
			})?;

			if !active_directories.insert(canonical_directory.clone()) {
				log::warn!("Skipping cyclic asset directory link '{}'.", path.display());
				continue;
			}

			let result = discover_file_sources_in(root, &path, active_directories, sources);
			active_directories.remove(&canonical_directory);
			result?;

			continue;
		}

		if !is_file {
			continue;
		}

		let Some(relative_path) = path.strip_prefix(root).ok() else {
			continue;
		};
		let Some(id) = resource_id_path(relative_path) else {
			log::warn!(
				"Skipping asset path '{}'. The most likely cause is a non-UTF-8 path that cannot be represented as a resource ID.",
				path.display()
			);
			continue;
		};
		let resource_id = ResourceId::new(&id);

		if resource_id.get_extension().eq_ignore_ascii_case("bead") && resource_id.get_asset_type().eq_ignore_ascii_case("bead")
		{
			continue;
		}

		let has_sidecar =
			!resource_id.get_extension().eq_ignore_ascii_case("bead") && path.with_added_extension("bead").is_file();

		sources.push(AssetSource::new(id, has_sidecar));
	}

	Ok(())
}

/// Converts a relative local path to a resource ID that uses `/` separators.
fn resource_id_path(path: &Path) -> Option<String> {
	let mut id = String::new();

	for component in path.components() {
		let component = component.as_os_str().to_str()?;

		if !id.is_empty() {
			id.push('/');
		}

		id.push_str(component);
	}

	Some(id)
}

#[cfg(test)]
pub mod tests {
	use std::{
		alloc::{Allocator, Global},
		collections::HashMap,
		fs::{self, FileTimes, OpenOptions},
		io::Write,
		sync::{Arc, Mutex},
		time::{Duration, SystemTime, UNIX_EPOCH},
	};

	use super::{AssetSource, AssetStorageBytes, AssetVersion, FileStorageBackend, ResolveResult, StorageBackend};
	use crate::{asset::ResourceId, tests::ASSETS_PATH};

	/// The `TestStorageBackend` struct provides in-memory source files with an asset-directory fallback for tests.
	#[derive(Clone)]
	pub struct TestStorageBackend(Arc<Mutex<HashMap<String, Box<[u8]>>>>);

	/// The `VirtualStorageBackend` struct proves allocator-aware reads stay within a custom backend namespace.
	struct VirtualStorageBackend;

	impl StorageBackend for VirtualStorageBackend {
		async fn resolve<'a>(&'a self, url: ResourceId<'a>) -> ResolveResult<'a> {
			Ok((
				AssetStorageBytes::Owned(url.as_ref().as_bytes().into()),
				url.get_asset_type().to_string(),
			))
		}
		async fn version<'a>(&'a self, url: ResourceId<'a>) -> Result<AssetVersion, ()> {
			Ok(AssetVersion::from_content(url.as_ref().as_bytes()))
		}
	}

	impl TestStorageBackend {
		pub fn new() -> Self {
			Self(Arc::new(Mutex::new(HashMap::default())))
		}

		pub fn add_file(&self, name: &'static str, data: &[u8]) {
			self.0.lock().unwrap().insert(name.to_string(), data.into());
		}

		pub fn remove_file(&self, name: &str) {
			self.0.lock().unwrap().remove(name);
		}
	}

	#[crate::r#async::test]
	async fn allocator_aware_reads_use_the_custom_backend_namespace() {
		let backend = VirtualStorageBackend;
		let (bytes, _) = backend
			.resolve_in(ResourceId::new("not-on-disk.environment-source"), &Global)
			.await
			.expect("the custom source must resolve without consulting the process filesystem");

		assert_eq!(bytes.as_slice(), b"not-on-disk.environment-source");
		assert!(matches!(bytes, AssetStorageBytes::Allocated(_)));
	}

	impl StorageBackend for TestStorageBackend {
		fn discover(&self) -> impl std::future::Future<Output = Result<Vec<super::AssetSource>, String>> {
			let files = self.0.lock().unwrap();
			let sources = files
				.keys()
				.filter_map(|id| {
					let resource_id = ResourceId::new(id);

					if resource_id.get_extension().eq_ignore_ascii_case("bead")
						&& resource_id.get_asset_type().eq_ignore_ascii_case("bead")
					{
						return None;
					}

					let sidecar = std::path::Path::new(id).with_added_extension("bead");
					let has_sidecar = !resource_id.get_extension().eq_ignore_ascii_case("bead")
						&& files.contains_key(sidecar.to_str().unwrap());
					Some(super::AssetSource::new(id.clone(), has_sidecar))
				})
				.collect();

			async move { Ok(sources) }
		}

		async fn resolve<'a>(&'a self, url: ResourceId<'a>) -> ResolveResult<'a> {
			let mocked = self.0.lock().unwrap().get(url.get_base().as_ref()).cloned();
			if let Some(bytes) = mocked {
				return Ok((AssetStorageBytes::Owned(bytes), url.get_asset_type().to_string()));
			}
			super::read_asset_from_source(url, Some(std::path::Path::new(ASSETS_PATH)), &Global).await
		}

		async fn version<'a>(&'a self, url: ResourceId<'a>) -> Result<super::AssetVersion, ()> {
			if let Some(bytes) = self.0.lock().unwrap().get(url.get_base().as_ref()) {
				return Ok(super::AssetVersion::from_content(bytes));
			}
			FileStorageBackend {
				base_path: ASSETS_PATH.into(),
			}
			.version(url)
			.await
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
	async fn file_storage_backend_discovers_nested_sources_and_sidecar_presence() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(directory.join("nested")).unwrap();
		fs::create_dir_all(directory.join("lighting")).unwrap();
		fs::write(directory.join("root.test"), []).unwrap();
		fs::write(directory.join("nested/source.bin"), []).unwrap();
		fs::write(directory.join("nested/source.bin.bead"), b"{}").unwrap();
		fs::write(directory.join("lighting/studio.environment.bead"), b"{}").unwrap();

		let storage_backend = FileStorageBackend::new(directory.clone());
		let mut sources = storage_backend.discover().await.unwrap();
		sources.sort_by(|left, right| left.id().cmp(right.id()));

		assert_eq!(
			sources,
			[
				AssetSource::new("lighting/studio.environment.bead".to_string(), false),
				AssetSource::new("nested/source.bin".to_string(), true),
				AssetSource::new("root.test".to_string(), false),
			]
		);

		fs::remove_dir_all(directory).unwrap();
	}

	#[crate::r#async::test]
	async fn file_storage_backend_resolves_assets_as_mapped_slices() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(&directory).unwrap();
		let path = directory.join("shader.bin");
		let expected = b"asset-bytes";
		fs::write(&path, expected).unwrap();

		let storage_backend = FileStorageBackend::new(directory.clone());
		let (bytes, format) = storage_backend
			.resolve(ResourceId::new("shader.bin"))
			.await
			.expect("asset should resolve");

		assert!(matches!(bytes, AssetStorageBytes::MappedFile(_)));
		assert_eq!(bytes.as_slice(), expected);
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
		let (bytes, format) = storage_backend
			.resolve(ResourceId::new("skeleton"))
			.await
			.expect("extensionless dependency should resolve");

		assert_eq!(bytes.as_slice(), expected);
		assert_eq!(format, "");

		fs::remove_dir_all(directory).unwrap();
	}

	#[crate::r#async::test]
	async fn file_storage_backend_resolves_compound_bead_types_without_a_second_sidecar() {
		let directory = temporary_asset_directory();
		fs::create_dir_all(&directory).unwrap();
		fs::write(directory.join("studio.environment.bead"), b"environment").unwrap();
		fs::write(directory.join("studio.environment.bead.bead"), b"{ ignored: true }").unwrap();

		let storage_backend = FileStorageBackend::new(directory.clone());
		let (bytes, asset_type) = storage_backend
			.resolve(ResourceId::new("studio.environment.bead"))
			.await
			.expect("the compound BEAD declaration should resolve");

		assert_eq!(bytes.as_slice(), b"environment");
		assert_eq!(asset_type, "environment.bead");

		let version = storage_backend
			.version(ResourceId::new("studio.environment.bead"))
			.await
			.expect("the compound BEAD declaration should have source metadata");

		assert!(version.source.is_some());

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

		assert_eq!(first.source.as_ref().unwrap().size, 4);

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

		assert_eq!(modified, with_sidecar);

		fs::remove_dir_all(directory).unwrap();
	}

	#[crate::r#async::test]
	async fn sidecars_are_explicit_optional_json5_reads() {
		let backend = TestStorageBackend::new();
		let id = ResourceId::new("explicit.test#fragment");
		backend.add_file("explicit.test", b"source");
		assert!(backend.load_sidecar(id).await.unwrap().is_none());
		backend.add_file("explicit.test.bead", b"{ value: 3, }");
		assert_eq!(backend.load_sidecar(id).await.unwrap().unwrap()["value"], 3);
		backend.add_file("explicit.test.bead", b"invalid json5");
		assert_eq!(backend.resolve(id).await.unwrap().0.as_slice(), b"source");
		assert!(backend.load_sidecar(id).await.is_err());
		backend.remove_file("explicit.test.bead");
		assert!(backend.load_sidecar(id).await.unwrap().is_none());
	}

	#[crate::r#async::test]
	async fn file_versions_track_source_and_sidecar_independently() {
		let storage_backend = TestStorageBackend::new();
		storage_backend.add_file("source.exr", b"source");
		storage_backend.add_file("source.exr.bead", b"{ exposure: 1 }");

		let raw_before = storage_backend.version(ResourceId::new("source.exr")).await.unwrap();
		let full_before = storage_backend.version(ResourceId::new("source.exr.bead")).await.unwrap();

		storage_backend.add_file("source.exr.bead", b"{ exposure: 2 }");

		let raw_after = storage_backend.version(ResourceId::new("source.exr")).await.unwrap();
		let full_after = storage_backend.version(ResourceId::new("source.exr.bead")).await.unwrap();

		assert_eq!(raw_before, raw_after);
		assert_ne!(full_before, full_after);
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

use utils::hash::FxHasher;

use super::{BEADType, ResourceId, parse_json, read_asset_from_source};
use crate::{
	r#async::{BoxedFuture, File as AsyncFile, future},
	resource::reader::MappedFileBacking,
};
