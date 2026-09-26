/// The `ResourceUpdate` struct identifies a successfully rebaked development resource.
#[cfg(debug_assertions)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceUpdate {
	id: String,
	class: String,
}

#[cfg(debug_assertions)]
impl ResourceUpdate {
	pub(crate) fn new(id: String, class: String) -> Self {
		Self { id, class }
	}

	/// Returns the stable ID of the replaced resource.
	pub fn id(&self) -> &str {
		&self.id
	}

	/// Returns the resource class used to select interested systems.
	pub fn class(&self) -> &str {
		&self.class
	}
}

/// The `ResourceUpdateListener` struct receives successful development resource replacements.
#[cfg(debug_assertions)]
pub struct ResourceUpdateListener(std::sync::mpsc::Receiver<ResourceUpdate>);

#[cfg(debug_assertions)]
impl ResourceUpdateListener {
	/// Returns the next queued update without blocking the consuming system.
	pub fn read(&self) -> Option<ResourceUpdate> {
		self.0.try_recv().ok()
	}
}

/// The `ResourceUpdateBroadcaster` struct connects development asset baking to resource consumers.
#[cfg(debug_assertions)]
#[derive(Default)]
pub(crate) struct ResourceUpdateBroadcaster(utils::sync::Mutex<Vec<std::sync::mpsc::Sender<ResourceUpdate>>>);

#[cfg(debug_assertions)]
impl ResourceUpdateBroadcaster {
	pub(crate) fn listener(&self) -> ResourceUpdateListener {
		let (sender, receiver) = std::sync::mpsc::channel();

		self.0.lock().push(sender);

		ResourceUpdateListener(receiver)
	}

	pub(crate) fn send(&self, update: ResourceUpdate) {
		self.0.lock().retain(|listener| listener.send(update.clone()).is_ok());
	}
}

const BAKING_APP_RESOURCES_DOCS_PATH: &str = "develop/resource-management/baking-app-resources";

/// The `RequestError` enum reports why [`ResourceManager::request`] could not produce a resource.
///
/// Every variant carries the requested ID. Match on it to decide how to recover, or
/// format it with [`Display`](std::fmt::Display) to show the cause and the fix.
/// See [baking app resources](/docs/develop/resource-management/baking-app-resources) for the recovery workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
	/// The resource is not in storage and no asset manager can bake it.
	Missing { id: String },
	/// The development asset manager could not bake or find the resource.
	#[cfg(debug_assertions)]
	Bake {
		id: String,
		source: LoadMessages,
		/// Whether the engine's `assets/byte-engine` link is broken, which changes the suggested fix.
		engine_assets_inaccessible: bool,
	},
	/// The stored record could not become the requested typed resource.
	Solve { id: String, source: SolveError },
}

impl RequestError {
	/// Returns the ID the caller requested.
	pub fn id(&self) -> &str {
		match self {
			RequestError::Missing { id } | RequestError::Solve { id, .. } => id,
			#[cfg(debug_assertions)]
			RequestError::Bake { id, .. } => id,
		}
	}

	/// Classifies one asset-manager failure, checking whether the engine asset link explains it.
	#[cfg(debug_assertions)]
	fn bake(id: &str, source: LoadMessages, asset_manager: &AssetManager) -> Self {
		let engine_assets_inaccessible = matches!(
			source,
			LoadMessages::FailedToBake {
				error: LoadErrors::AssetCouldNotBeRead,
				..
			}
		) && (id == "byte-engine" || id.starts_with("byte-engine/"))
			&& asset_manager.source_directory_accessible(std::path::Path::new("byte-engine")) == Some(false);

		RequestError::Bake {
			id: id.to_owned(),
			source,
			engine_assets_inaccessible,
		}
	}
}

impl std::fmt::Display for RequestError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		let guide = online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH);

		match self {
			RequestError::Missing { id } => {
				let (cause, fix) = if cfg!(debug_assertions) {
					(
						"The resource does not exist and no asset manager is available.",
						"Install an asset manager or bake the application resources with BELD.",
					)
				} else {
					(
						"The resource is missing from the baked release store.",
						"Bake the application resources with BELD and include the resource store in the application bundle.",
					)
				};

				write!(
					f,
					"Could not load resource.\n\n  Resource: {id}\n  Cause: {cause}\n  Fix: {fix}\n  Guide: {guide}"
				)
			}
			#[cfg(debug_assertions)]
			RequestError::Bake {
				id,
				source,
				engine_assets_inaccessible,
			} => {
				let (summary, asset, cause, fix) = match source {
					LoadMessages::NoAsset => (
						"Could not load asset.",
						id.as_str(),
						"The asset manager did not produce a resource.",
						"Verify the source asset and its dependencies, then bake the application resources with BELD.",
					),
					LoadMessages::IO => (
						"Could not load asset.",
						id.as_str(),
						"The asset source could not be read.",
						"Verify that the asset source is accessible, then bake the application resources with BELD.",
					),
					LoadMessages::NoURL => (
						"Could not load asset.",
						id.as_str(),
						"The asset description has no source URL.",
						"Add the source URL, then bake the application resources with BELD.",
					),
					LoadMessages::NoAssetHandler => (
						"Could not bake asset.",
						id.as_str(),
						"No asset handler supports this asset type.",
						"Use a supported asset type or register its handler, then bake the application resources with BELD.",
					),
					LoadMessages::FailedToBake { asset, error } => (
						"Could not bake asset.",
						asset.as_str(),
						error.message(),
						if *engine_assets_inaccessible {
							"Configure or repair the 'assets/byte-engine' directory link, then retry."
						} else {
							error.fix()
						},
					),
					LoadMessages::FailedToStore { asset, error } => (
						"Could not store baked asset.",
						asset.as_str(),
						error.as_str(),
						"Verify that the resource destination is writable, then bake the application resources with BELD.",
					),
					LoadMessages::ExecutionUnavailable => (
						"Could not bake asset.",
						id.as_str(),
						"No asset worker was available.",
						"Verify the asset-processing runtime, then bake the application resources with BELD.",
					),
				};

				write!(
					f,
					"{summary}\n\n  Asset: {asset}\n  Cause: {cause}\n  Fix: {fix}\n  Guide: {guide}"
				)
			}
			RequestError::Solve { id, source } => write!(
				f,
				"Could not load resource.\n\n  Resource: {id}\n  Cause: {source}\n  Fix: Bake the application resources again with BELD.\n  Guide: {guide}"
			),
		}
	}
}

impl std::error::Error for RequestError {
	fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
		match self {
			RequestError::Solve { source, .. } => Some(source),
			_ => None,
		}
	}
}

/// The `ResourceManager` struct provides typed resource loading and caching across storage backends.
///
/// Debug builds can use an asset manager to bake missing source assets on demand.
/// Release builds load only resources that already exist in the configured backend.
///
/// File-system paths are relative to the assets directory.
/// After construction, optionally install an asset manager in debug builds,
/// then obtain typed resources through [`Self::request`].
/// See [debug asset loading](/docs/develop/resource-management/debug-loading)
/// and [resource loading](/docs/develop/resource-management/resources)
/// for the development and release workflows.
pub struct ResourceManager {
	#[cfg(debug_assertions)]
	asset_manager: std::sync::OnceLock<AssetManager>,
	#[cfg(debug_assertions)]
	resource_updates: std::sync::Arc<ResourceUpdateBroadcaster>,

	storage_backend: std::sync::Arc<dyn DynStorageBackend>,
}

impl ResourceManager {
	/// Creates a resource manager over the selected storage backend.
	///
	/// In debug builds, optionally install an asset manager before the first
	/// request. Next, call [`Self::request`] for each typed runtime resource.
	pub fn new<SB: StorageBackend + 'static>(storage_backend: SB) -> Self {
		Self::new_shared(Arc::new(storage_backend))
	}

	/// Creates a resource manager that shares its store with an asset manager.
	pub fn new_shared(storage_backend: std::sync::Arc<dyn DynStorageBackend>) -> Self {
		ResourceManager {
			#[cfg(debug_assertions)]
			asset_manager: std::sync::OnceLock::new(),
			#[cfg(debug_assertions)]
			resource_updates: std::sync::Arc::new(ResourceUpdateBroadcaster::default()),
			storage_backend,
		}
	}

	/// Returns the shared destination store used for resource reads and asset bakes.
	pub fn storage_backend(&self) -> std::sync::Arc<dyn DynStorageBackend> {
		std::sync::Arc::clone(&self.storage_backend)
	}

	/// Installs an asset manager that can bake missing assets on demand in debug builds.
	///
	/// # Panics
	///
	/// Panics when asset management was already installed on this resource manager.
	#[cfg(debug_assertions)]
	pub fn set_asset_manager(&self, asset_manager: AssetManager) {
		assert!(
			self.try_set_asset_manager(asset_manager).is_ok(),
			"Failed to set up resource manager. The most likely cause is that asset management was installed more than once or uses a different destination resource store."
		);
	}

	/// Attempts to install the development asset manager without replacing an existing one.
	#[cfg(debug_assertions)]
	pub fn try_set_asset_manager(&self, asset_manager: AssetManager) -> Result<(), AssetManager> {
		if !asset_manager.uses_resource_storage(&self.storage_backend) {
			return Err(asset_manager);
		}

		self.asset_manager.set(asset_manager)?;

		self.asset_manager
			.get()
			.unwrap()
			.start_watching(std::sync::Arc::clone(&self.resource_updates));

		Ok(())
	}

	/// Subscribes to resources replaced after successful development rebakes.
	#[cfg(debug_assertions)]
	pub fn resource_updates(&self) -> ResourceUpdateListener {
		self.resource_updates.listener()
	}

	/// Returns the development trace for asset-backed resource bakes when asset management is installed.
	///
	/// Next, call [`ResourceTrace::items`] with the resource ID shown by the
	/// editor or other development tool.
	#[cfg(debug_assertions)]
	pub fn resource_trace(&self) -> Option<&ResourceTrace> {
		self.asset_manager.get().map(AssetManager::resource_trace)
	}

	fn get_storage_backend(&self) -> &dyn DynStorageBackend {
		self.storage_backend.as_ref()
	}

	/// Loads resource metadata and dependencies, then returns a deferred binary-data [`Reference`].
	///
	/// Await the request because development builds may need to bake a missing
	/// source asset before resolving the stored resource.
	///
	/// Use [`Reference::load`](crate::Reference::load) to load the binary data into
	/// caller-provided memory or reader-owned storage. After loading, access the
	/// typed metadata through [`Reference::resource`](crate::Reference::resource).
	pub async fn request<T: Resource>(&self, id: &str) -> Result<Reference<T>, RequestError>
	where
		T::Model: StoredModel<Resource = T>,
	{
		// The record read here is solved directly, so the requested resource is read from storage once.
		let (stored, reader) = self.read_stored(id).await?;

		T::Model::solve_stored(stored, reader, self.get_storage_backend())
			.await
			.map_err(|source| RequestError::Solve {
				id: id.to_owned(),
				source,
			})
	}

	/// Returns the stored class of `id`, such as `Mesh`, `Variant`, or `Image`.
	///
	/// Use this to route a resource whose type the caller does not know, then
	/// call [`Self::request`] with the matching resource type.
	pub async fn class(&self, id: &str) -> Result<String, RequestError> {
		let (stored, _) = self.read_stored(id).await?;
		Ok(stored.class().to_owned())
	}

	/// Returns one page of resource IDs that match indexed metadata, without resolving any resource.
	///
	/// Every returned ID has the class named by `query`. Development builds only
	/// see resources that were already baked.
	pub async fn query_ids(&self, query: impl Into<Query>) -> Result<QueryPage<String>, QueryError> {
		let page = self.get_storage_backend().query(query.into()).await?;
		Ok(QueryPage {
			items: page.items.into_iter().map(|(stored, _)| stored.id().to_owned()).collect(),
			cursor: page.cursor,
		})
	}

	/// Bakes `id` when stale in development builds, then reads its stored record.
	async fn read_stored(&self, id: &str) -> Result<(SerializableResource, MultiResourceReader), RequestError> {
		let storage_backend = self.get_storage_backend();

		#[cfg(debug_assertions)]
		let asset_manager = self.asset_manager.get();

		#[cfg(debug_assertions)]
		if let Some(asset_manager) = asset_manager {
			asset_manager
				.bake_if_stale(id)
				.await
				.map_err(|source| RequestError::bake(id, source, asset_manager))?;
		}

		let Some((stored, reader)) = storage_backend.read(ResourceId::new(id)).await else {
			#[cfg(debug_assertions)]
			if let Some(asset_manager) = asset_manager {
				return Err(RequestError::bake(id, LoadMessages::NoAsset, asset_manager));
			}

			return Err(RequestError::Missing { id: id.to_owned() });
		};

		#[cfg(debug_assertions)]
		if let Some(asset_manager) = asset_manager {
			asset_manager.track_resource(&stored);
		}

		Ok((stored, reader))
	}

	/// Loads independent resources concurrently while preserving the requested order.
	///
	/// Use this method when every ID is known before any individual result is
	/// needed. `max_concurrency` bounds debug baking and storage pressure.
	pub async fn request_many<T: Resource>(
		&self,
		ids: &[String],
		max_concurrency: usize,
	) -> Result<Vec<Reference<T>>, RequestError>
	where
		T::Model: StoredModel<Resource = T>,
	{
		use utils::r#async::StreamExt as _;

		let requests = ids
			.iter()
			.enumerate()
			.map(|(index, id)| async move { self.request(id).await.map(|resource| (index, resource)) });

		let completed = utils::r#async::stream::iter(requests)
			.buffer_unordered(max_concurrency.max(1))
			.collect::<Vec<_>>()
			.await;

		let mut completed = completed.into_iter().collect::<Result<Vec<_>, _>>()?;

		completed.sort_unstable_by_key(|(index, _)| *index);

		Ok(completed.into_iter().map(|(_, resource)| resource).collect())
	}

	/// Returns one page of typed resources that match indexed metadata.
	///
	/// Await this query, then use each
	/// [`Reference::resource`](crate::Reference::resource) for metadata and await
	/// [`Reference::load`](crate::Reference::load) only when the binary payload is
	/// needed.
	pub async fn query<T: Resource>(&self, query: impl Into<Query>) -> Result<QueryPage<Reference<T>>, QueryError>
	where
		T::Model: StoredModel<Resource = T>,
	{
		let page = self
			.get_storage_backend()
			.query(Query {
				class: T::Model::get_class().to_string(),
				..query.into()
			})
			.await?;

		let mut items = Vec::with_capacity(page.items.len());

		// Each query item already carries its record and reader, so solving it needs no second read.
		for (stored, reader) in page.items {
			// Keep the failing record's ID, because the page may hold many records of the same class.
			let id = stored.id().to_owned();
			let item = T::Model::solve_stored(stored, reader, self.get_storage_backend())
				.await
				.map_err(|source| QueryError::Solve { id, source })?;
			items.push(item);
		}

		Ok(QueryPage {
			items,
			cursor: page.cursor,
		})
	}
}

#[cfg(test)]
mod tests {

	use super::{RequestError, ResourceManager};
	use crate::{
		ProcessedAsset, ReferenceModel,
		asset::ResourceId,
		r#async,
		resource::{
			ReDBStorageBackend, ReadTargetsMut, ResourceStorageMode, WriteStorageBackend,
			storage_backend::{Query, QueryError, tests::TestStorageBackend},
		},
		resources::{
			audio::Audio,
			flipbook::{Flipbook, FlipbookModel},
			image::Image,
		},
		solver::SolveError,
		types::{BitDepths, Formats, Gamma},
	};

	#[r#async::test]
	async fn stored_request_awaits_metadata_and_preserves_deferred_payload_loading() {
		let storage = TestStorageBackend::new();

		let audio = Audio {
			bit_depth: BitDepths::Sixteen,
			channel_count: 1,
			sample_rate: 48_000,
			sample_count: 2,
		};

		storage
			.store(ProcessedAsset::new(ResourceId::new("audio/loop.wav"), audio), &[1, 2, 3, 4])
			.await
			.unwrap();

		let resource_manager = ResourceManager::new(storage);

		let mut reference = resource_manager
			.request::<Audio>("audio/loop.wav")
			.await
			.expect("stored audio resource");

		assert_eq!(reference.resource().bit_depth, audio.bit_depth);
		assert_eq!(reference.resource().channel_count, audio.channel_count);
		assert_eq!(reference.resource().sample_rate, audio.sample_rate);
		assert_eq!(reference.resource().sample_count, audio.sample_count);

		let loaded = reference
			.load(ReadTargetsMut::backing_storage())
			.await
			.expect("deferred payload");

		assert_eq!(loaded.buffer(), Some([1, 2, 3, 4].as_slice()));
	}

	#[r#async::test]
	async fn query_reports_the_record_that_cannot_be_solved() {
		let directory = std::env::temp_dir().join(format!(
			"byte-engine-resource-manager-query-{}-{}",
			std::process::id(),
			std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.unwrap()
				.as_nanos()
		));
		let storage = ReDBStorageBackend::new_writable_with_mode(directory.clone(), ResourceStorageMode::Files).unwrap();

		storage
			.store(
				ProcessedAsset::new_with_serialized("broken.image", "Image", vec![1, 2, 3]),
				&[],
			)
			.await
			.unwrap();

		let resource_manager = ResourceManager::new(storage);

		let error = resource_manager
			.query::<Image>(Query::new("Image").eq("name", "broken.image"))
			.await
			.unwrap_err();

		assert!(matches!(
			error,
			QueryError::Solve {
				id,
				source: SolveError::DeserializationFailed(_),
			} if id == "broken.image"
		));

		drop(resource_manager);
		std::fs::remove_dir_all(directory).unwrap();
	}

	#[r#async::test]
	async fn request_reports_an_unreadable_dependency_by_id() {
		let storage = TestStorageBackend::new();
		let image = Image {
			format: Formats::RGBA8,
			gamma: Gamma::Linear,
			extent: [1, 1, 0],
			mip_count: 1,
			ibl: None,
			photometry: None,
		};
		let flipbook = FlipbookModel {
			frames_per_second: 12,
			images: vec![ReferenceModel::new("frames.image", 0, 0, &image, None)],
		};

		storage
			.store(ProcessedAsset::new(ResourceId::new("run.flipbook"), flipbook), &[])
			.await
			.unwrap();

		let error = ResourceManager::new(storage)
			.request::<Flipbook>("run.flipbook")
			.await
			.unwrap_err();

		assert!(matches!(
			error,
			RequestError::Solve {
				id,
				source: SolveError::UnreadableDependency { id: dependency },
			} if id == "run.flipbook" && dependency == "frames.image"
		));
	}
}

#[cfg(all(test, debug_assertions))]
mod debug_tests {
	use std::{
		fs,
		sync::{
			Arc,
			atomic::{AtomicUsize, Ordering},
		},
		time::{SystemTime, UNIX_EPOCH},
	};

	use utils::sync::Mutex;

	use super::ResourceManager;
	use crate::{
		ProcessedAsset,
		asset::{
			ResourceId, ResourceTraceLevel,
			handler::{AssetHandler, BakeContext, LoadErrors},
			manager::AssetManager,
			storage_backend::{FileStorageBackend, tests::TestStorageBackend as AssetTestStorageBackend},
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::{Shader, ShaderArtifact, ShaderInterface},
		types::ShaderTypes,
	};

	struct ResolvingAssetHandler;

	impl AssetHandler for ResolvingAssetHandler {
		fn can_handle(&self, extension: &str) -> bool {
			matches!(extension, "besl" | "test")
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			context.resolve(id).await.map(|_| ())
		}
	}

	struct CoordinatingShaderHandler {
		invocations: Arc<AtomicUsize>,
		started: Mutex<Option<announcement::Announcer<()>>>,
		release: announcement::Listener<()>,
	}

	struct VersionedShaderHandler {
		invocations: Arc<AtomicUsize>,
	}

	impl AssetHandler for VersionedShaderHandler {
		fn can_handle(&self, extension: &str) -> bool {
			extension == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			self.invocations.fetch_add(1, Ordering::SeqCst);

			let (source, ..) = context.resolve(id).await?;

			context
				.store_primary(
					ProcessedAsset::new(
						id,
						Shader {
							id: id.to_string(),
							stage: ShaderTypes::Compute,
							interface: ShaderInterface {
								workgroup_size: None,
								bindings: Vec::new(),
							},
							artifact: ShaderArtifact::Spirv,
							source_hash: 0,
						},
					),
					&source,
				)
				.await
		}
	}

	impl AssetHandler for CoordinatingShaderHandler {
		fn can_handle(&self, extension: &str) -> bool {
			extension == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			self.invocations.fetch_add(1, Ordering::SeqCst);

			self.started
				.lock()
				.take()
				.expect("the test handler should start once")
				.announce(())
				.expect("test startup announcement should be open");

			self.release
				.listen()
				.await
				.expect("test release announcement should remain open");

			context
				.store_primary(
					ProcessedAsset::new(
						id,
						Shader {
							id: id.to_string(),
							stage: ShaderTypes::Compute,
							interface: ShaderInterface {
								workgroup_size: None,
								bindings: Vec::new(),
							},
							artifact: ShaderArtifact::Spirv,
							source_hash: 0,
						},
					),
					&[],
				)
				.await
		}
	}

	fn temporary_asset_directory(name: &str) -> std::path::PathBuf {
		std::env::temp_dir().join(format!(
			"byte-engine-resource-manager-{name}-{}-{}",
			std::process::id(),
			SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
		))
	}

	fn resource_manager_with_file_assets(path: std::path::PathBuf) -> ResourceManager {
		let storage = Arc::new(ResourceTestStorageBackend::new());

		let mut asset_manager = AssetManager::new_shared(FileStorageBackend::new(path), storage.clone());

		asset_manager.add_asset_handler(ResolvingAssetHandler);

		let resource_manager = ResourceManager::new_shared(storage);

		resource_manager.set_asset_manager(asset_manager);

		resource_manager
	}

	#[test]
	fn asset_management_can_be_installed_after_the_resource_manager_is_shared() {
		let storage = Arc::new(ResourceTestStorageBackend::new());

		let resource_manager = Arc::new(ResourceManager::new_shared(storage.clone()));

		let renderer_reference = Arc::downgrade(&resource_manager);

		resource_manager.set_asset_manager(AssetManager::new_shared(AssetTestStorageBackend::new(), storage.clone()));

		assert!(renderer_reference.upgrade().is_some());
		assert!(resource_manager.resource_trace().is_some());
		assert!(
			resource_manager
				.try_set_asset_manager(AssetManager::new_shared(AssetTestStorageBackend::new(), storage))
				.is_err()
		);
	}

	#[r#async::test]
	async fn inaccessible_engine_asset_root_suggests_configuring_the_symlink() {
		let assets = temporary_asset_directory("missing-root");

		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("byte-engine/missing.test")
			.await
			.unwrap_err();

		assert_eq!(
			error.to_string(),
			format!(
				"Could not bake asset.\n\n  Asset: byte-engine/missing.test\n  Cause: The source asset could not be read.\n  Fix: Configure or repair the 'assets/byte-engine' directory link, then retry.\n  Guide: {}",
				super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)
			)
		);

		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn missing_engine_asset_namespace_suggests_checking_the_asset_id() {
		let assets = temporary_asset_directory("accessible-root");

		fs::create_dir_all(assets.join("byte-engine")).unwrap();

		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("rendering/simple/vertex.besl")
			.await
			.unwrap_err();

		assert_eq!(
			error.to_string(),
			format!(
				"Could not bake asset.\n\n  Asset: rendering/simple/vertex.besl\n  Cause: The source asset could not be read.\n  Fix: Check the asset ID and configured assets directory. Engine asset IDs start with 'byte-engine/'.\n  Guide: {}",
				super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)
			)
		);

		let trace = resource_manager
			.resource_trace()
			.expect("installed asset management should expose its trace");

		let items = trace.items("rendering/simple/vertex.besl");

		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);

		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn concurrent_resource_requests_share_one_missing_asset_bake() {
		let invocations = Arc::new(AtomicUsize::new(0));

		let (started, started_announcement) = announcement::Announcement::new();

		let (release, release_announcement) = announcement::Announcement::new();

		let storage = Arc::new(ResourceTestStorageBackend::new());

		let mut asset_manager = AssetManager::new_shared(AssetTestStorageBackend::new(), storage.clone());

		asset_manager.add_asset_handler(CoordinatingShaderHandler {
			invocations: Arc::clone(&invocations),
			started: Mutex::new(Some(started)),
			release: release_announcement.listener(),
		});

		let resource_manager = ResourceManager::new_shared(storage);

		resource_manager.set_asset_manager(asset_manager);

		let release_handler = async {
			started_announcement
				.listener()
				.listen()
				.await
				.expect("asset bake should start");

			release.announce(()).expect("release should be announced once");
		};

		let requests = async {
			std::future::join!(
				resource_manager.request::<Shader>("shared.test"),
				resource_manager.request::<Shader>("shared.test"),
			)
			.await
		};

		let (_, (first, second)) = std::future::join!(release_handler, requests).await;

		assert!(first.is_ok());
		assert!(second.is_ok());
		assert_eq!(invocations.load(Ordering::SeqCst), 1);
	}

	#[r#async::test]
	async fn debug_resource_requests_rebake_only_after_the_requested_asset_changes() {
		let invocations = Arc::new(AtomicUsize::new(0));

		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("versioned.test", b"first shader");

		let storage = Arc::new(ResourceTestStorageBackend::new());

		let mut asset_manager = AssetManager::new_shared(asset_storage.clone(), storage.clone());

		asset_manager.add_asset_handler(VersionedShaderHandler {
			invocations: Arc::clone(&invocations),
		});

		let resource_manager = ResourceManager::new_shared(storage);

		resource_manager.set_asset_manager(asset_manager);

		let first = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("initial debug request should bake");

		let unchanged = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("unchanged debug request should reuse the resource");

		assert_eq!(first.hash(), unchanged.hash());
		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		asset_storage.add_file("versioned.test", b"changed shader source");

		let changed = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("changed debug source should rebake");

		assert_ne!(first.hash(), changed.hash());
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}
}

#[cfg(all(test, not(debug_assertions)))]
mod release_tests {

	use super::ResourceManager;
	use crate::{r#async, resource::storage_backend::tests::TestStorageBackend, resources::material::Shader};

	#[r#async::test]
	async fn missing_release_resource_fails_without_running_asset_processors() {
		let resource_manager = ResourceManager::new(TestStorageBackend::new());

		let result = resource_manager.request::<Shader>("missing/render-pass.besl").await;

		assert!(matches!(
			result,
			Err(error)
				if error.to_string() == format!(
					"Could not load resource.\n\n  Resource: missing/render-pass.besl\n  Cause: The resource is missing from the baked release store.\n  Fix: Bake the application resources with BELD and include the resource store in the application bundle.\n  Guide: {}",
					super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)
				)
		));
	}
}

use std::sync::Arc;

use super::{
	DynStorageBackend, StorageBackend,
	resource_handler::MultiResourceReader,
	storage_backend::{Query, QueryError, QueryPage},
};
#[cfg(debug_assertions)]
use crate::asset::{
	ResourceTrace,
	handler::LoadErrors,
	manager::{AssetManager, LoadMessages},
};
use crate::{
	Model, Reference, Resource, SerializableResource, StoredModel, asset::ResourceId, online_docs_url, solver::SolveError,
};
