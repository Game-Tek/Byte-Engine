const ASSETS_DOCS_PATH: &str = "develop/design/resource-management/assets";

trait AbstractAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;
	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

/// The `AssetManager` struct selects asset handlers and bakes source assets into resource storage.
///
/// Register each source format with [`Self::add_asset_handler`], then install the
/// manager on a [`crate::ResourceManager`] for debug loading or call
/// [`Self::bake`] from an explicit baking workflow.
/// See the [assets guide](https://byte-engine.0x44491229.dev/docs/develop/design/resource-management/assets)
/// for supported source families and processing behavior.
pub struct AssetManager {
	asset_handlers: Vec<Box<dyn AbstractAssetHandler>>,
	storage_backend: Box<dyn StorageBackend>,
	in_flight_bakes: Mutex<HashMap<String, announcement::Announcement<Result<(), LoadMessages>>>>,
	#[cfg(debug_assertions)]
	resource_trace: ResourceTrace,
}

/// The `LoadMessages` enum identifies failures while an asset is loaded, baked, or stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadMessages {
	/// The asset was not found in the storage backend.
	NoAsset,
	/// An I/O operation failed while loading the asset.
	IO,
	/// The asset description does not contain a URL.
	NoURL,
	/// No asset handler was found for the asset.
	NoAssetHandler,
	/// The asset or one of its dependencies could not be baked or loaded.
	FailedToBake { asset: String, error: LoadErrors },
	/// The asset could not be stored in the resource storage backend.
	FailedToStore { asset: String, error: String },
}

impl AssetManager {
	/// Creates an asset manager over the source-asset storage backend.
	///
	/// Next, register all required formats with [`Self::add_asset_handler`] before
	/// installing the manager or starting a bake.
	pub fn new<SB: StorageBackend + 'static>(storage_backend: SB) -> AssetManager {
		Self {
			asset_handlers: Vec::with_capacity(8),
			storage_backend: Box::new(storage_backend),
			in_flight_bakes: Mutex::new(HashMap::with_capacity(32)),
			#[cfg(debug_assertions)]
			resource_trace: ResourceTrace::default(),
		}
	}

	/// Registers a handler for one family of source assets.
	///
	/// After all handlers are registered, install this manager on a
	/// [`crate::ResourceManager`] in a debug build or call [`Self::bake`].
	pub fn add_asset_handler<T: AssetHandler + Send + Sync + 'static>(&mut self, asset_handler: T) {
		struct AssetHandlerWrapper<T: AssetHandler + Send + Sync>(T);

		impl<T: AssetHandler + Send + Sync> AbstractAssetHandler for AssetHandlerWrapper<T> {
			fn can_handle(&self, r#type: &str) -> bool {
				self.0.can_handle(r#type)
			}

			fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool {
				self.0.should_discover(id, has_sidecar)
			}

			fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>> {
				Box::pin(self.0.bake(context, url))
			}
		}

		self.asset_handlers.push(Box::new(AssetHandlerWrapper(asset_handler)));
	}

	pub fn get_storage_backend(&self) -> &dyn StorageBackend {
		self.storage_backend.as_ref()
	}

	/// Reports whether a source directory can be read when the storage backend exposes paths.
	pub(crate) fn source_directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.storage_backend.directory_accessible(path)
	}

	/// Returns the development trace populated by this manager's latest resource bakes.
	///
	/// Next, call [`ResourceTrace::items`] with a requested resource ID.
	#[cfg(debug_assertions)]
	pub fn resource_trace(&self) -> &ResourceTrace {
		&self.resource_trace
	}

	/// Copies the latest in-memory trace into development resource storage for external tools.
	#[cfg(debug_assertions)]
	fn persist_resource_trace(&self, id: ResourceId<'_>, storage: &dyn ResourceStorageBackend) {
		if let Err(error) = storage.replace_trace(id, &self.resource_trace.items(id.as_ref())) {
			log::warn!(
				"Failed to store the resource trace for '{}'. The most likely cause is that development resource storage is not writable. Error: {}",
				id.as_ref(),
				error
			);
		}
	}

	/// Returns whether a registered asset handler can bake the given source ID.
	pub fn supports(&self, id: &str) -> bool {
		let id = ResourceId::new(id);
		self.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_extension()))
	}

	/// Returns whether recursive discovery should include the given supported source asset.
	pub fn should_discover(&self, id: &str, has_sidecar: bool) -> bool {
		let id = ResourceId::new(id);
		self.asset_handlers
			.iter()
			.any(|handler| handler.can_handle(id.get_extension()) && handler.should_discover(id, has_sidecar))
	}

	/// Bakes the asset at `id` without checking for an existing stored resource.
	///
	/// Next, await [`crate::ResourceManager::request`] for the stored output or
	/// inspect it through the storage backend.
	pub async fn bake<'a>(&self, id: &str, resource_storage_backend: &dyn ResourceStorageBackend) -> Result<(), LoadMessages> {
		self.bake_in(id, resource_storage_backend, &Global).await
	}

	/// Bakes an asset while using the provided allocator for generation-time buffers.
	pub async fn bake_in<'a>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
		allocator: &dyn Allocator,
	) -> Result<(), LoadMessages> {
		enum Role {
			Leader(announcement::Announcer<Result<(), LoadMessages>>),
			Follower(announcement::Listener<Result<(), LoadMessages>>),
		}

		let notification = {
			let mut registry = self.in_flight_bakes.lock();

			match registry.entry(id.to_owned()) {
				Occupied(entry) => Role::Follower(entry.get().listener()),
				Vacant(entry) => {
					let (announcer, announcement) = announcement::Announcement::new();

					entry.insert(announcement);

					Role::Leader(announcer)
				}
			}
		};

		match notification {
			Role::Leader(notification) => {
				let result = self.bake_uncoalesced(id, resource_storage_backend, allocator).await;

				notification.announce(result.clone()).unwrap();

				self.in_flight_bakes.lock().remove(id);

				result
			}
			Role::Follower(notification) => notification.listen().await.unwrap(), // This will panic when the announcer is closed, eg: when the bake is cancelled
		}
	}

	/// Runs one asset handler invocation without consulting the in-flight registry.
	///
	/// Call this method directly when no coalescing is desired.
	async fn bake_uncoalesced(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
		allocator: &dyn Allocator,
	) -> Result<(), LoadMessages> {
		let id = ResourceId::new(id);

		#[cfg(debug_assertions)]
		{
			self.resource_trace.clear(id);
			self.persist_resource_trace(id, resource_storage_backend);
		}

		let asset_handler = match self
			.asset_handlers
			.iter()
			.find(|handler| handler.can_handle(id.get_extension()))
		{
			Some(handler) => handler,
			None => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					format!(
						"No asset handler found for '{}'. The most likely cause is an unsupported file extension or missing handler registration. See {}.",
						id.as_ref(),
						online_docs_url(ASSETS_DOCS_PATH)
					),
				);
				#[cfg(debug_assertions)]
				self.persist_resource_trace(id, resource_storage_backend);
				log::warn!(
					"No asset handler found for asset: {:#?}. The most likely cause is an unsupported file extension or missing handler registration. See {}.",
					id,
					online_docs_url(ASSETS_DOCS_PATH)
				);
				return Err(LoadMessages::NoAssetHandler);
			}
		};

		let start_time = std::time::Instant::now();

		// The shared flag enforces the primary-write contract without rereading potentially expensive storage.
		let primary_stored = Cell::new(false); // TODO: revise this

		let context = BakeContext::new(
			self,
			resource_storage_backend,
			self.storage_backend.as_ref(),
			allocator,
			id,
			&primary_stored,
			#[cfg(debug_assertions)]
			&self.resource_trace,
		);
		let result = match asset_handler.bake(context, id).await {
			Ok(()) if primary_stored.get() => Ok(()),
			Ok(()) => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					"The asset handler completed without storing the requested primary resource. The most likely cause is a missing store_primary call."
						.to_string(),
				);
				Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error: LoadErrors::PrimaryResourceNotStored,
				})
			}
			Err(LoadErrors::FailedToStore) => {
				#[cfg(debug_assertions)]
				self.resource_trace.record(
					id,
					ResourceTraceLevel::Error,
					"Failed to store the requested resource. The resource storage backend likely rejected the primary resource write."
						.to_string(),
				);
				Err(LoadMessages::FailedToStore {
					asset: id.to_string(),
					error: format!(
						"Failed to store asset {:#?}. The resource storage backend likely rejected the primary resource write.",
						id
					),
				})
			}
			Err(error) => {
				#[cfg(debug_assertions)]
				if !self.resource_trace.has_error(id) {
					self.resource_trace.record(
						id,
						ResourceTraceLevel::Error,
						format!(
							"Failed to bake resource '{}': {error:?}. The most likely cause is invalid or unsupported source data. See {}.",
							id.as_ref(),
							online_docs_url(ASSETS_DOCS_PATH)
						),
					);
				}
				log::error!(
					"Failed to bake asset: {:#?}. The most likely cause is invalid or unsupported source data. See {}.",
					error,
					online_docs_url(ASSETS_DOCS_PATH)
				);
				Err(LoadMessages::FailedToBake {
					asset: id.to_string(),
					error,
				})
			}
		};

		#[cfg(debug_assertions)]
		self.persist_resource_trace(id, resource_storage_backend);

		result?;

		log::trace!("Baked '{:#?}' resource in {:#?}", id, start_time.elapsed());

		Ok(())
	}

	/// Returns the stored asset, or bakes it when no resource with a matching hash exists.
	pub async fn bake_if_not_exists<'a, M: Model>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
	) -> Result<ReferenceModel<M>, LoadMessages> {
		self.bake_if_not_exists_in(id, resource_storage_backend, &Global).await
	}

	/// Bakes an asset with the provided allocator if the resource does not already exist.
	pub async fn bake_if_not_exists_in<'a, M: Model>(
		&self,
		id: &str,
		resource_storage_backend: &dyn ResourceStorageBackend,
		allocator: &dyn Allocator,
	) -> Result<ReferenceModel<M>, LoadMessages> {
		let id = ResourceId::new(id);

		if resource_storage_backend.read(id).await.is_none() {
			self.bake_in(id.as_ref(), resource_storage_backend, allocator).await?;
		}

		if let Some(result) = resource_storage_backend.read(id).await {
			let (resource, _) = result;
			let resource: ReferenceModel<M> = resource.into();
			return Ok(resource);
		}

		Err(LoadMessages::NoAsset)
	}
}

#[cfg(test)]
pub mod tests {
	use std::{
		future::Future,
		sync::{
			atomic::{AtomicUsize, Ordering},
			Arc,
		},
	};

	use super::*;
	#[cfg(debug_assertions)]
	use crate::asset::ResourceTraceLevel;
	use crate::{
		asset::{asset_handler::LoadErrors, storage_backend::tests::TestStorageBackend},
		r#async::{self, BoxedFuture},
		resource::{storage_backend::tests::TestStorageBackend as ResourceTestStorageBackend, ReadStorageBackend},
		Model, ProcessedAsset,
	};

	#[derive(serde::Serialize, serde::Deserialize, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
	struct TestResource {}

	impl Model for TestResource {
		fn get_class() -> &'static str {
			"TestResource"
		}
	}

	struct TestAssetHandler {}

	impl TestAssetHandler {
		fn new() -> TestAssetHandler {
			TestAssetHandler {}
		}
	}

	struct CoordinatingAssetHandler {
		invocations: Arc<AtomicUsize>,
		started: Arc<Vec<Mutex<Option<announcement::Announcer<()>>>>>,
		release: announcement::Listener<()>,
		fail: bool,
		block_first_only: bool,
	}

	impl AssetHandler for CoordinatingAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			let invocation = self.invocations.fetch_add(1, Ordering::SeqCst);
			self.started[invocation]
				.lock()
				.take()
				.expect("each test invocation should announce once")
				.announce(())
				.expect("test invocation announcement should be open");
			if !self.block_first_only || invocation == 0 {
				self.release
					.listen()
					.await
					.expect("test release announcement should remain open");
			}

			if self.fail {
				Err(LoadErrors::FailedToProcess)
			} else {
				context.store_primary(ProcessedAsset::new(id, TestResource {}), &[])
			}
		}
	}

	fn coordinating_asset_manager(
		fail: bool,
		block_first_only: bool,
	) -> (
		AssetManager,
		Arc<AtomicUsize>,
		Vec<announcement::Listener<()>>,
		announcement::Announcer<()>,
	) {
		let invocations = Arc::new(AtomicUsize::new(0));
		let mut started_announcers = Vec::with_capacity(8);
		let mut started_listeners = Vec::with_capacity(8);
		for _ in 0..8 {
			let (announcer, announcement) = announcement::Announcement::new();
			started_announcers.push(Mutex::new(Some(announcer)));
			started_listeners.push(announcement.listener());
		}
		let started = Arc::new(started_announcers);
		let (release, release_announcement) = announcement::Announcement::new();
		let mut manager = AssetManager::new(TestStorageBackend::new());
		manager.add_asset_handler(CoordinatingAssetHandler {
			invocations: Arc::clone(&invocations),
			started: Arc::clone(&started),
			release: release_announcement.listener(),
			fail,
			block_first_only,
		});
		(manager, invocations, started_listeners, release)
	}

	impl AssetHandler for TestAssetHandler {
		fn can_handle(&self, id: &str) -> bool {
			id == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<(), LoadErrors> {
			match id.get_base().as_ref() {
				"example.test" => context.store_primary(ProcessedAsset::new(id, TestResource {}), &[]),
				"messages.test" => {
					context.info("Imported test metadata.");
					context.warn(format_args!("Discarded {} optional test value.", 1));
					context.store_primary(ProcessedAsset::new(id, TestResource {}), &[])
				}
				"failed.test" => {
					context
						.error("Test resource is malformed. The most likely cause is the intentionally invalid fixture data.");
					Err(LoadErrors::FailedToProcess)
				}
				"unstored.test" => Ok(()),
				"mismatched.test" => {
					context.store_primary(ProcessedAsset::new(ResourceId::new("other.test"), TestResource {}), &[])
				}
				_ => Err(LoadErrors::AssetCouldNotBeLoaded),
			}
		}
	}

	pub fn new_testing_asset_manager() -> AssetManager {
		let storage_backend = TestStorageBackend::new();
		AssetManager::new(storage_backend)
	}

	#[test]
	fn test_new() {
		let _ = new_testing_asset_manager();
	}

	#[test]
	fn test_add_asset_manager() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);

		let test_asset_handler = TestAssetHandler::new();

		asset_manager.add_asset_handler(test_asset_handler);
	}

	#[test]
	fn asset_manager_reports_support_for_registered_asset_types() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.supports("nested/example.test"));
		assert!(asset_manager.supports("nested/example.test#fragment"));
		assert!(!asset_manager.supports("nested/example.unknown"));
	}

	#[test]
	fn registered_handlers_are_discoverable_by_default() {
		let storage_backend = TestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		assert!(asset_manager.should_discover("nested/example.test", false));
		assert!(asset_manager.should_discover("nested/example.test", true));
		assert!(!asset_manager.should_discover("nested/example.unknown", true));
	}

	#[r#async::test]
	async fn test_bake_with_asset_manager() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		asset_manager
			.bake("example.test", &resource_storage_backend)
			.await
			.expect("registered asset handler should bake its resource");

		let resource = resource_storage_backend
			.get_resource(ResourceId::new("example.test"))
			.expect("baked resource should be stored");
		assert_eq!(resource.class, "TestResource");
	}

	#[r#async::test]
	async fn concurrent_bakes_for_one_asset_and_store_share_one_invocation() {
		let (asset_manager, invocations, started, release) = coordinating_asset_manager(false, false);
		let resource_storage_backend = ResourceTestStorageBackend::new();

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");
			release.announce(()).expect("release should be announced once");
		};
		let requests = async {
			std::future::join!(
				asset_manager.bake("coalesced.test", &resource_storage_backend),
				asset_manager.bake("coalesced.test", &resource_storage_backend),
				asset_manager.bake("coalesced.test", &resource_storage_backend),
			)
			.await
		};
		let (_, results) = std::future::join!(release_handler, requests).await;

		assert_eq!(results, (Ok(()), Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 1);
	}

	#[r#async::test]
	async fn concurrent_failures_are_shared_but_later_bakes_retry() {
		let (asset_manager, invocations, started, release) = coordinating_asset_manager(true, false);
		let resource_storage_backend = ResourceTestStorageBackend::new();

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");
			release.announce(()).expect("release should be announced once");
		};
		let requests = async {
			std::future::join!(
				asset_manager.bake("failed.test", &resource_storage_backend),
				asset_manager.bake("failed.test", &resource_storage_backend),
			)
			.await
		};
		let (_, (first, follower)) = std::future::join!(release_handler, requests).await;

		assert_eq!(first, follower);
		assert_eq!(invocations.load(Ordering::SeqCst), 1);

		let retry = asset_manager.bake("failed.test", &resource_storage_backend).await;

		assert_eq!(retry, first);
		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn completed_explicit_bake_is_not_memoized() {
		let (asset_manager, invocations, started, release) = coordinating_asset_manager(false, true);
		let resource_storage_backend = ResourceTestStorageBackend::new();

		let release_handler = async {
			started[0].listen().await.expect("first invocation should start");
			release.announce(()).expect("release should be announced once");
		};
		let (_, first) =
			std::future::join!(release_handler, asset_manager.bake("repeat.test", &resource_storage_backend),).await;
		assert_eq!(first, Ok(()));
		assert_eq!(asset_manager.bake("repeat.test", &resource_storage_backend).await, Ok(()));

		assert_eq!(invocations.load(Ordering::SeqCst), 2);
	}

	#[r#async::test]
	async fn different_assets_and_destination_stores_do_not_coalesce() {
		let (asset_manager, invocations, started, release) = coordinating_asset_manager(false, false);
		let first_storage = ResourceTestStorageBackend::new();
		let second_storage = ResourceTestStorageBackend::new();

		let release_handler = async {
			// Three invocations are enough to prove progress without hanging when two stores are incorrectly coalesced.
			started[2].listen().await.expect("three independent invocations should start");
			release.announce(()).expect("release should be announced once");
		};
		let requests = async {
			std::future::join!(
				asset_manager.bake("first.test", &first_storage),
				asset_manager.bake("second.test", &first_storage),
				asset_manager.bake("shared.test", &first_storage),
				asset_manager.bake("shared.test", &second_storage),
			)
			.await
		};
		let (_, results) = std::future::join!(release_handler, requests).await;

		assert_eq!(results, (Ok(()), Ok(()), Ok(()), Ok(())));
		assert_eq!(invocations.load(Ordering::SeqCst), 4);
	}

	#[r#async::test]
	async fn test_bake_no_asset_handler() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let asset_manager = AssetManager::new(storage_backend);

		let result = asset_manager.bake("example.unknown", &resource_storage_backend).await;

		assert_eq!(result, Err(LoadMessages::NoAssetHandler));
		#[cfg(debug_assertions)]
		assert_eq!(
			asset_manager.resource_trace().items("example.unknown")[0].level(),
			ResourceTraceLevel::Error
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn handler_trace_keeps_ordered_info_and_warning_items_for_a_baked_resource() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		asset_manager
			.bake("messages.test", &resource_storage_backend)
			.await
			.expect("message fixture should bake");
		// A new bake replaces the prior trace instead of accumulating stale messages.
		asset_manager
			.bake("messages.test", &resource_storage_backend)
			.await
			.expect("message fixture should rebake");

		let items = asset_manager.resource_trace().items("messages.test");
		assert_eq!(items.len(), 2);
		assert_eq!(items[0].level(), ResourceTraceLevel::Info);
		assert_eq!(items[0].message(), "Imported test metadata.");
		assert_eq!(items[1].level(), ResourceTraceLevel::Warn);
		assert_eq!(items[1].message(), "Discarded 1 optional test value.");
		assert_eq!(
			resource_storage_backend
				.read_trace(ResourceId::new("messages.test"))
				.await
				.unwrap(),
			items
		);
	}

	#[cfg(debug_assertions)]
	#[r#async::test]
	async fn handler_error_trace_survives_when_the_resource_bake_fails() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("failed.test", &resource_storage_backend).await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "failed.test".to_string(),
				error: LoadErrors::FailedToProcess,
			})
		);
		assert!(resource_storage_backend
			.get_resource(ResourceId::new("failed.test"))
			.is_none());
		let items = asset_manager.resource_trace().items("failed.test");
		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);
		assert_eq!(
			items[0].message(),
			"Test resource is malformed. The most likely cause is the intentionally invalid fixture data."
		);
		assert_eq!(
			resource_storage_backend
				.read_trace(ResourceId::new("failed.test"))
				.await
				.unwrap(),
			items
		);
		assert_eq!(asset_manager.resource_trace().resource_ids(), vec!["failed.test"]);
	}

	#[r#async::test]
	async fn successful_handler_must_store_the_requested_primary_resource() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("unstored.test", &resource_storage_backend).await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "unstored.test".to_string(),
				error: LoadErrors::PrimaryResourceNotStored,
			})
		);
	}

	#[r#async::test]
	async fn handler_cannot_store_a_different_resource_as_the_primary() {
		let storage_backend = TestStorageBackend::new();
		let resource_storage_backend = ResourceTestStorageBackend::new();
		let mut asset_manager = AssetManager::new(storage_backend);
		asset_manager.add_asset_handler(TestAssetHandler::new());

		let result = asset_manager.bake("mismatched.test", &resource_storage_backend).await;

		assert_eq!(
			result,
			Err(LoadMessages::FailedToBake {
				asset: "mismatched.test".to_string(),
				error: LoadErrors::PrimaryResourceIdMismatch,
			})
		);
		assert!(resource_storage_backend.get_resources().is_empty());
	}
}

use std::{
	alloc::{Allocator, Global},
	cell::Cell,
	collections::hash_map::Entry::{Occupied, Vacant},
	ops::Deref,
	sync::Arc,
};

use announcement;
use gxhash::HashMapExt;
use utils::{hash::HashMap, sync::Mutex};

#[cfg(debug_assertions)]
use super::resource_trace::{ResourceTrace, ResourceTraceLevel};
use super::{
	asset_handler::{AssetHandler, BakeContext},
	StorageBackend,
};
use crate::{
	asset::{self, asset_handler::LoadErrors, ResourceId},
	online_docs_url,
	r#async::BoxedFuture,
	resource::{self, StorageBackend as ResourceStorageBackend},
	Model, ProcessedAsset, ReferenceModel,
};
