use super::{
	storage_backend::{Query, QueryError, QueryPage},
	StorageBackend,
};
#[cfg(debug_assertions)]
use crate::asset::{
	asset_handler::LoadErrors,
	asset_manager::{AssetManager, LoadMessages},
	ResourceTrace,
};
use crate::{asset::ResourceId, online_docs_url, Model, Reference, ReferenceModel, Resource, SerializableResource, Solver};

#[cfg(debug_assertions)]
const BAKING_APP_RESOURCES_DOCS_PATH: &str = "develop/design/resource-management/baking-app-resources";

/// Adds engine asset setup guidance only when an engine asset failed to load and its source root is inaccessible.
#[cfg(debug_assertions)]
fn asset_lookup_error(message: &str, id: &str, error: &LoadMessages, asset_manager: &AssetManager) -> String {
	let byte_engine_root_inaccessible = matches!(
		error,
		LoadMessages::FailedToBake {
			error: LoadErrors::AssetCouldNotBeLoaded,
			..
		}
	) && (id == "byte-engine" || id.starts_with("byte-engine/"))
		&& asset_manager.source_directory_accessible(std::path::Path::new("byte-engine")) == Some(false);

	if byte_engine_root_inaccessible {
		format!(
			"{message} The 'byte-engine' path in the assets directory is inaccessible, so its symlink was probably not configured. See {}.",
			online_docs_url(BAKING_APP_RESOURCES_DOCS_PATH)
		)
	} else {
		message.to_string()
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
/// See [debug asset loading](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management/debug-loading)
/// and [resource loading](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management/resources)
/// for the development and release workflows.
pub struct ResourceManager {
	#[cfg(debug_assertions)]
	asset_manager: std::sync::OnceLock<AssetManager>,

	storage_backend: std::sync::Arc<dyn StorageBackend>,
}

impl ResourceManager {
	/// Creates a resource manager over the selected storage backend.
	///
	/// In debug builds, optionally install an asset manager before the first
	/// request. Next, call [`Self::request`] for each typed runtime resource.
	pub fn new<SB: StorageBackend>(storage_backend: SB) -> Self {
		Self::new_shared(std::sync::Arc::new(storage_backend))
	}

	/// Creates a resource manager that shares its store with an asset manager.
	pub fn new_shared(storage_backend: std::sync::Arc<dyn StorageBackend>) -> Self {
		ResourceManager {
			#[cfg(debug_assertions)]
			asset_manager: std::sync::OnceLock::new(),
			storage_backend,
		}
	}

	/// Returns the shared destination store used for resource reads and asset bakes.
	pub fn storage_backend(&self) -> std::sync::Arc<dyn StorageBackend> {
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
		self.asset_manager.set(asset_manager)
	}

	/// Returns the development trace for asset-backed resource bakes when asset management is installed.
	///
	/// Next, call [`ResourceTrace::items`] with the resource ID shown by the
	/// editor or other development tool.
	#[cfg(debug_assertions)]
	pub fn resource_trace(&self) -> Option<&ResourceTrace> {
		self.asset_manager.get().map(AssetManager::resource_trace)
	}

	fn get_storage_backend(&self) -> &dyn StorageBackend {
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
	pub async fn request<T: Resource>(&self, id: &str) -> Result<Reference<T>, String>
	where
		for<'de> ReferenceModel<T::Model>: Solver<'de, Reference<T>>,
		SerializableResource: TryInto<ReferenceModel<T::Model>>,
	{
		let storage_backend = self.get_storage_backend();

		let reference_model: ReferenceModel<T::Model> = {
			#[cfg(debug_assertions)]
			{
				if let Some(asset_manager) = self.asset_manager.get() {
					asset_manager.bake_if_not_exists(id).await.map_err(|error| {
						asset_lookup_error(
							"Failed to load asset. The asset manager could not bake the resource.",
							id,
							&error,
							asset_manager,
						)
					})?
				} else if let Some((resource, _)) = storage_backend.read(ResourceId::new(id)).await {
					resource.into()
				} else {
					return Err("Resource does not exist and an asset manager is not available.".to_string());
				}
			}
			#[cfg(not(debug_assertions))]
			{
				if let Some((resource, _)) = storage_backend.read(ResourceId::new(id)).await {
					resource.into()
				} else {
					return Err("Resource does not exist in the baked release resource store.".to_string());
				}
			}
		};

		let reference: Reference<T> = reference_model
			.solve(self.get_storage_backend())
			.await
			.map_err(|error| Into::<&'static str>::into(error).to_string())?;

		Ok(reference)
	}

	/// Returns one page of typed resources that match indexed metadata.
	///
	/// Await this query, then use each
	/// [`Reference::resource`](crate::Reference::resource) for metadata and await
	/// [`Reference::load`](crate::Reference::load) only when the binary payload is
	/// needed.
	pub async fn query<T: Resource>(&self, query: Query) -> Result<QueryPage<Reference<T>>, QueryError>
	where
		for<'de> ReferenceModel<T::Model>: Solver<'de, Reference<T>>,
		SerializableResource: Into<ReferenceModel<T::Model>>,
	{
		let page = self
			.get_storage_backend()
			.query(Query {
				class: T::Model::get_class().to_string(),
				..query
			})
			.await?;

		let mut items = Vec::with_capacity(page.items.len());
		for (resource, _) in page.items {
			let model: ReferenceModel<T::Model> = resource.into();
			items.push(model.solve(self.get_storage_backend()).await.unwrap());
		}

		Ok(QueryPage {
			items,
			cursor: page.cursor,
		})
	}
}

#[cfg(test)]
mod tests {
	use super::ResourceManager;
	use crate::{
		asset::ResourceId,
		r#async,
		resource::{storage_backend::tests::TestStorageBackend, ReadTargetsMut, WriteStorageBackend},
		resources::audio::Audio,
		types::BitDepths,
		ProcessedAsset,
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
}

#[cfg(all(test, debug_assertions))]
mod debug_tests {
	use std::{
		fs,
		sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		},
		time::{SystemTime, UNIX_EPOCH},
	};

	use utils::sync::Mutex;

	use super::ResourceManager;
	use crate::{
		asset::{
			asset_handler::{AssetHandler, BakeContext, LoadErrors},
			asset_manager::AssetManager,
			storage_backend::{tests::TestStorageBackend as AssetTestStorageBackend, FileStorageBackend},
			ResourceId, ResourceTraceLevel,
		},
		r#async,
		resource::storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend,
		resources::material::{Shader, ShaderArtifact, ShaderInterface},
		types::ShaderTypes,
		ProcessedAsset,
	};

	struct ResolvingAssetHandler;

	impl AssetHandler for ResolvingAssetHandler {
		fn can_handle(&self, extension: &str) -> bool {
			extension == "test"
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
			context.store_primary(
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
			context.store_primary(
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
		let storage: Arc<dyn crate::resource::StorageBackend> = Arc::new(ResourceTestStorageBackend::new());
		let mut asset_manager = AssetManager::new_shared(FileStorageBackend::new(path), Arc::clone(&storage));
		asset_manager.add_asset_handler(ResolvingAssetHandler);
		let resource_manager = ResourceManager::new_shared(storage);
		resource_manager.set_asset_manager(asset_manager);
		resource_manager
	}

	#[test]
	fn asset_management_can_be_installed_after_the_resource_manager_is_shared() {
		let storage: Arc<dyn crate::resource::StorageBackend> = Arc::new(ResourceTestStorageBackend::new());
		let resource_manager = Arc::new(ResourceManager::new_shared(Arc::clone(&storage)));
		let renderer_reference = Arc::downgrade(&resource_manager);

		resource_manager.set_asset_manager(AssetManager::new_shared(AssetTestStorageBackend::new(), Arc::clone(&storage)));
		assert!(renderer_reference.upgrade().is_some());
		assert!(resource_manager.resource_trace().is_some());
		assert!(resource_manager
			.try_set_asset_manager(AssetManager::new_shared(AssetTestStorageBackend::new(), storage))
			.is_err());
	}

	#[r#async::test]
	async fn inaccessible_engine_asset_root_suggests_configuring_the_symlink() {
		let assets = temporary_asset_directory("missing-root");
		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("byte-engine/missing.test")
			.await
			.unwrap_err();

		assert!(error.contains("The 'byte-engine' path in the assets directory is inaccessible"));
		assert!(error.contains(&super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)));
		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn individual_asset_failure_omits_engine_symlink_hint_when_root_is_accessible() {
		let assets = temporary_asset_directory("accessible-root");
		fs::create_dir_all(assets.join("byte-engine")).unwrap();
		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("byte-engine/missing.test")
			.await
			.unwrap_err();

		assert_eq!(error, "Failed to load asset. The asset manager could not bake the resource.");
		let trace = resource_manager
			.resource_trace()
			.expect("installed asset management should expose its trace");
		let items = trace.items("byte-engine/missing.test");
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);
		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn concurrent_resource_requests_share_one_missing_asset_bake() {
		let invocations = Arc::new(AtomicUsize::new(0));
		let (started, started_announcement) = announcement::Announcement::new();
		let (release, release_announcement) = announcement::Announcement::new();
		let storage: Arc<dyn crate::resource::StorageBackend> = Arc::new(ResourceTestStorageBackend::new());
		let mut asset_manager = AssetManager::new_shared(AssetTestStorageBackend::new(), Arc::clone(&storage));
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
		let storage: Arc<dyn crate::resource::StorageBackend> = Arc::new(ResourceTestStorageBackend::new());
		let mut asset_manager = AssetManager::new_shared(asset_storage.clone(), Arc::clone(&storage));
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

		assert!(
			matches!(result, Err(error) if error.starts_with("Resource does not exist in the baked release resource store."))
		);
	}
}
