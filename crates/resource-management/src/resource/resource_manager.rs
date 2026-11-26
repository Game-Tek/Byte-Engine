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
pub struct ResourceUpdateListener(flume::Receiver<ResourceUpdate>);

#[cfg(debug_assertions)]
impl ResourceUpdateListener {
	/// Returns the next queued update without blocking the consuming system.
	pub fn read(&self) -> Option<ResourceUpdate> {
		self.0.try_recv().ok()
	}
}

const BAKING_APP_RESOURCES_DOCS_PATH: &str = "use/resources/bake";

/// The `RequestError` enum reports why [`ResourceManager::request`] could not produce a resource.
///
/// Every variant carries the requested ID. Match on it to decide how to recover, or
/// format it with [`Display`](std::fmt::Display) to show the cause and the fix.
/// See [Bake application resources](/docs/use/resources/bake) for the recovery workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestError {
	/// The resource is not in storage and no asset manager can bake it.
	Missing { id: String },
	/// The development asset manager could not bake or find the resource.
	#[cfg(debug_assertions)]
	Bake {
		id: String,
		source: LoadMessages,
		/// Whether the engine's `assets/byte_engine` link is broken, which changes the suggested fix.
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
		) && (id == "byte_engine" || id.starts_with("byte_engine/"))
			&& asset_manager.source_directory_accessible(std::path::Path::new("byte_engine")) == Some(false);

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
							"Configure or repair the 'assets/byte_engine' directory link, then retry."
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
					LoadMessages::Excluded { asset } => (
						"Could not bake asset.",
						asset.as_str(),
						"A `assets.exclude` pattern matches the asset, so it is never baked.",
						"Stop requesting the asset, or remove its pattern from `assets.exclude` in config.json.",
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
/// Create it with [`Self::new`] over a store it reads directly, or in debug builds with [`Self::hosted`], whose store a
/// thread of its own keeps so [`Self::install_assets`] can add baking later. Then obtain typed resources through
/// [`Self::request`].
/// See [debug asset loading](/docs/use/resources/debug-loading)
/// and [resource loading](/docs/develop/resource-management/resources)
/// for the development and release workflows.
pub struct ResourceManager {
	store: Store,
}

/// The `Store` enum says where a [`ResourceManager`]'s store lives.
enum Store {
	/// The manager owns the store and reads it on the caller's executor.
	Owned(Box<dyn DynStorageBackend>),
	/// A server on another thread or task owns the store and answers each request over a channel.
	Remote(Remote),
}

/// The `Remote` struct is a [`ResourceManager`]'s end of the channel to the server that owns its store.
struct Remote {
	reads: RemoteReads,
	/// The thread that owns the store, which ends once the channel closes; `None` when the server is a bake that waits
	/// for this manager to drop.
	host: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Remote {
	fn drop(&mut self) {
		let Some(host) = self.host.take() else {
			return;
		};
		// The host stops serving once its channel closes, so the sender goes first. A closed placeholder takes its place.
		drop(std::mem::replace(&mut self.reads.0, flume::bounded(0).0));
		if host.join().is_err() {
			log::error!(
				"The resource host thread panicked. The most likely cause is a failed asset bake or store read, which the log above names."
			);
		}
	}
}

/// The `StoreRequest` enum is one request a [`ResourceManager`] sends to the server that owns its store.
pub(crate) enum StoreRequest {
	/// Reads the resource `id`, after making it current when the server bakes.
	Request {
		id: String,
		reply: oneshot::Sender<Result<(SerializableResource, MultiResourceReader), RequestError>>,
	},
	/// Reads the resource `id` as stored.
	Read {
		id: String,
		reply: oneshot::Sender<Option<(SerializableResource, MultiResourceReader)>>,
	},
	List {
		reply: oneshot::Sender<Result<Vec<SerializableResource>, String>>,
	},
	Query {
		query: Query,
		reply: oneshot::Sender<Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError>>,
	},
	/// Runs an operation with the server's asset workers. A server without them drops it.
	#[cfg(debug_assertions)]
	Assets(AssetsTask),
	/// Builds the asset manager that owns the store from here on. Only a host that bakes nothing yet accepts it.
	#[cfg(debug_assertions)]
	Install(InstallAssets),
	/// Sends each resource a source change rebakes to the listener.
	#[cfg(debug_assertions)]
	Subscribe(flume::Sender<ResourceUpdate>),
}

/// An operation [`ResourceManager::on_assets`] runs on the host's asset workers.
#[cfg(debug_assertions)]
type AssetsTask = Box<dyn for<'w, 's> FnOnce(&'w AssetWorkers<'s>) -> BoxedFuture<'w, ()> + Send>;

/// The builder [`ResourceManager::install_assets`] hands the host, which builds the asset manager over its store.
#[cfg(debug_assertions)]
type InstallAssets = Box<dyn FnOnce(Box<dyn DynStorageBackend>) -> AssetManager + Send>;

/// Guides the closure in [`ResourceManager::on_assets`] to the higher-ranked signature an [`AssetsTask`] has.
#[cfg(debug_assertions)]
fn assets_task(task: impl for<'w, 's> FnOnce(&'w AssetWorkers<'s>) -> BoxedFuture<'w, ()> + Send + 'static) -> AssetsTask {
	Box::new(task)
}

impl ResourceManager {
	/// Creates a resource manager that owns `storage_backend` and reads it directly.
	///
	/// Next, call [`Self::request`] for each typed runtime resource. To bake missing resources in debug builds, create
	/// it with [`Self::hosted`] instead.
	pub fn new<SB: StorageBackend + 'static>(storage_backend: SB) -> Self {
		ResourceManager {
			store: Store::Owned(Box::new(storage_backend)),
		}
	}

	/// Creates a resource manager over a store another owner handed over, such as the one
	/// [`AssetManager::into_storage`] returns after a bake.
	pub fn from_boxed(storage_backend: Box<dyn DynStorageBackend>) -> Self {
		ResourceManager {
			store: Store::Owned(storage_backend),
		}
	}

	/// Creates a resource manager whose store a thread of its own keeps, so a later [`Self::install_assets`] can bake
	/// into it while the manager is already shared.
	///
	/// Requests read the store as it is until assets are installed. An application creates its manager this way in
	/// debug builds; dropping the manager stops the thread after the bakes it started finish.
	#[cfg(debug_assertions)]
	pub fn hosted<SB: StorageBackend + 'static>(storage_backend: SB) -> Self {
		let (requests, received) = flume::unbounded();
		let store: Box<dyn DynStorageBackend> = Box::new(storage_backend);
		let host = std::thread::Builder::new()
			.name("Resource Host".into())
			.spawn(move || host(store, received))
			.expect("Failed to start the resource host thread. The most likely cause is that the system ran out of threads.");

		ResourceManager {
			store: Store::Remote(Remote {
				reads: RemoteReads(requests),
				host: Some(host),
			}),
		}
	}

	/// Creates a resource manager whose requests a bake serves over `requests`, for work a bake runs on a thread of its
	/// own. See [`crate::asset::handler::BakeContext::with_resource_thread`].
	pub(crate) fn remote(requests: flume::Sender<StoreRequest>) -> Self {
		ResourceManager {
			store: Store::Remote(Remote {
				reads: RemoteReads(requests),
				host: None,
			}),
		}
	}

	/// Makes this manager bake missing and stale resources from the sources in `storage_backend`, with the bake services
	/// and asset handlers `configure` installs.
	///
	/// The host thread builds the [`AssetManager`] over the manager's store and serves every later request through it.
	///
	/// # Panics
	///
	/// Panics when this manager reads its store directly, because it was created with [`Self::new`].
	#[cfg(debug_assertions)]
	pub fn install_assets<AS: crate::asset::StorageBackend + 'static>(
		&self,
		storage_backend: AS,
		configure: impl FnOnce(&mut AssetManager) + Send + 'static,
	) {
		let Store::Remote(remote) = &self.store else {
			panic!(
				"Failed to install asset management. The most likely cause is a resource manager created with `ResourceManager::new`, which reads its store directly; create it with `ResourceManager::hosted`."
			);
		};
		let install: InstallAssets = Box::new(move |store| {
			let mut asset_manager = AssetManager::from_boxed(Box::new(storage_backend), store);
			configure(&mut asset_manager);
			asset_manager
		});
		let _ = remote.reads.0.send(StoreRequest::Install(install));
	}

	/// Runs `operation` with the asset workers that bake this manager's store, and returns its result, or `None` when
	/// the manager bakes nothing.
	///
	/// Development tools reach the application's source assets through it, such as with
	/// `resources.on_assets(async move |workers| workers.manager().read_source(&id).await)`.
	#[cfg(debug_assertions)]
	pub async fn on_assets<T: Send + 'static>(
		&self,
		operation: impl AsyncFnOnce(&AssetWorkers<'_>) -> T + Send + 'static,
	) -> Option<T> {
		let Store::Remote(remote) = &self.store else {
			return None;
		};
		let (reply, result) = oneshot::channel();
		let task = assets_task(move |workers| {
			Box::pin(async move {
				let _ = reply.send(operation(workers).await);
			})
		});
		remote.reads.0.send(StoreRequest::Assets(task)).ok()?;
		result.await.ok()
	}

	/// Subscribes to resources replaced after successful development rebakes.
	#[cfg(debug_assertions)]
	pub fn resource_updates(&self) -> ResourceUpdateListener {
		let (sender, receiver) = flume::unbounded();
		// A manager that bakes nothing never replaces a resource, so its listener stays empty.
		if let Store::Remote(remote) = &self.store {
			let _ = remote.reads.0.send(StoreRequest::Subscribe(sender));
		}
		ResourceUpdateListener(receiver)
	}

	/// Returns the reader of the store, which solving a resource reads its dependencies through.
	fn reads(&self) -> &dyn DynReadStorageBackend {
		match &self.store {
			Store::Owned(store) => store.as_ref(),
			Store::Remote(remote) => &remote.reads,
		}
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
		T::Model::solve_stored(stored, reader, self.reads())
			.await
			.map_err(|source| RequestError::Solve {
				id: id.to_owned(),
				source,
			})
	}

	/// Bakes every discoverable source asset of `asset_type`, such as `fusion`, whose resource is missing or stale, so a
	/// following [`Self::query_ids`] finds them as it would in a baked store.
	///
	/// Development builds call this before querying resources no request names. A source that fails to bake is logged
	/// and skipped. Without asset management, it does nothing.
	#[cfg(debug_assertions)]
	pub async fn bake_discovered(&self, asset_type: &str) {
		let asset_type = asset_type.to_owned();
		self.on_assets(async move |workers| {
			let asset_manager = workers.manager();
			let ids = match asset_manager.discover().await {
				Ok(ids) => ids,
				Err(error) => {
					log::warn!(
						"Failed to list the source assets to bake ({error}). The most likely cause is an unreadable asset directory. Resources of type `{asset_type}` that were never baked stay missing."
					);
					return;
				}
			};
			for id in ids
				.iter()
				.filter(|id| crate::asset::ResourceId::new(id).get_asset_type() == asset_type)
			{
				if let Err(error) = workers.bake_if_stale(id).await {
					log::error!(
						"Failed to bake '{id}': {}. The most likely cause is an error in the asset, which its bake trace names.",
						RequestError::bake(id, error, asset_manager)
					);
				}
			}
		})
		.await;
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
		let page = self.reads().query(query.into()).await?;
		Ok(QueryPage {
			items: page.items.into_iter().map(|(stored, _)| stored.id().to_owned()).collect(),
			cursor: page.cursor,
		})
	}

	/// Reads the stored record of `id`, after the server made it current when it bakes.
	async fn read_stored(&self, id: &str) -> Result<(SerializableResource, MultiResourceReader), RequestError> {
		match &self.store {
			Store::Owned(store) => store
				.read(ResourceId::new(id))
				.await
				.ok_or_else(|| RequestError::Missing { id: id.to_owned() }),
			Store::Remote(remote) => {
				let (reply, result) = oneshot::channel();
				remote
					.reads
					.0
					.send(StoreRequest::Request {
						id: id.to_owned(),
						reply,
					})
					.map_err(|_| RequestError::Missing { id: id.to_owned() })?;
				result.await.map_err(|_| RequestError::Missing { id: id.to_owned() })?
			}
		}
	}
}

/// The `RemoteReads` struct reads a store another thread or task owns, by asking its server.
struct RemoteReads(flume::Sender<StoreRequest>);

impl RemoteReads {
	/// Sends the request `build` makes and waits for its reply, or returns `None` when the server is gone.
	async fn ask<T>(&self, build: impl FnOnce(oneshot::Sender<T>) -> StoreRequest) -> Option<T> {
		let (reply, result) = oneshot::channel();
		self.0.send(build(reply)).ok()?;
		result.await.ok()
	}
}

impl super::ReadStorageBackend for RemoteReads {
	async fn list(&self) -> Result<Vec<SerializableResource>, String> {
		self.ask(|reply| StoreRequest::List { reply })
			.await
			.unwrap_or_else(|| Err(SERVER_GONE.to_string()))
	}

	fn read<'a>(
		&'a self,
		id: ResourceId<'a>,
	) -> impl std::future::Future<Output = Option<(SerializableResource, MultiResourceReader)>> + 'a {
		async move {
			self.ask(|reply| StoreRequest::Read {
				id: id.as_ref().to_owned(),
				reply,
			})
			.await
			.flatten()
		}
	}

	async fn query(&self, query: Query) -> Result<QueryPage<(SerializableResource, MultiResourceReader)>, QueryError> {
		self.ask(|reply| StoreRequest::Query { query, reply })
			.await
			.unwrap_or_else(|| Err(QueryError::StorageFailure))
	}

	fn read_provenance(&self, _: ResourceId<'_>) -> Result<Option<crate::asset::Provenance>, String> {
		Err("A resource manager whose store another thread keeps doesn't read provenance. The most likely cause is a tool that reads provenance through the application's resource manager; open the store with BELD instead.".to_string())
	}
}

/// The error of a read whose store server stopped.
const SERVER_GONE: &str = "The resource store's server stopped. The most likely cause is that the application is shutting down, or the bake that served the store ended.";

/// How [`serve`] ended.
pub(crate) enum Served {
	/// Every requester dropped its end of the channel.
	Disconnected,
	/// A builder of the asset manager arrived, with the listeners subscribed so far.
	#[cfg(debug_assertions)]
	Install(InstallAssets, Vec<flume::Sender<ResourceUpdate>>),
}

/// Answers the requests that arrive on `requests` from `reads`, concurrently, until every requester is gone.
///
/// With `workers`, a request makes its resource current first, and `sources_changed` checks the tracked roots again.
/// Without them, the first install request ends the loop, after the requests already started finish.
pub(crate) async fn serve(
	reads: &dyn DynReadStorageBackend,
	workers: Option<&AssetWorkers<'_>>,
	requests: &flume::Receiver<StoreRequest>,
	sources_changed: Option<&flume::Receiver<()>>,
) -> Served {
	use utils::r#async::{FutureExt as _, StreamExt as _, select_biased, stream::FuturesUnordered};

	let mut running = FuturesUnordered::<BoxedFuture<'_, ()>>::new();
	#[cfg(debug_assertions)]
	let mut listeners = Vec::new();

	let ended = loop {
		// Resolves once a batch of sources changed. Without a watcher, or once it stopped, no change ever arrives, so the
		// future never resolves instead of resolving on every pass of the loop.
		let changed = async {
			if let Some(changed) = sources_changed
				&& changed.recv_async().await.is_ok()
			{
				return;
			}
			std::future::pending::<()>().await
		};
		let changed = std::pin::pin!(changed);

		let request = select_biased! {
			() = running.select_next_some() => continue,
			() = changed.fuse() => {
				#[cfg(debug_assertions)]
				if let Some(workers) = workers {
					workers.reload_changed();
				}
				continue;
			}
			request = requests.recv_async().fuse() => match request {
				Ok(request) => request,
				Err(_) => break Served::Disconnected,
			},
		};

		match request {
			StoreRequest::Request { id, reply } => running.push(Box::pin(async move {
				let _ = reply.send(request_resource(reads, workers, &id).await);
			})),
			StoreRequest::Read { id, reply } => running.push(Box::pin(async move {
				let _ = reply.send(reads.read(ResourceId::new(&id)).await);
			})),
			StoreRequest::List { reply } => running.push(Box::pin(async move {
				let _ = reply.send(reads.list().await);
			})),
			StoreRequest::Query { query, reply } => running.push(Box::pin(async move {
				let _ = reply.send(reads.query(query).await);
			})),
			#[cfg(debug_assertions)]
			StoreRequest::Assets(task) => {
				// Without workers the task is dropped, and its caller finds no asset management.
				if let Some(workers) = workers {
					running.push(task(workers));
				}
			}
			#[cfg(debug_assertions)]
			StoreRequest::Install(install) => match workers {
				None => break Served::Install(install, std::mem::take(&mut listeners)),
				Some(_) => log::error!(
					"Failed to install asset management. The most likely cause is that the application installed it twice; install it once."
				),
			},
			#[cfg(debug_assertions)]
			StoreRequest::Subscribe(listener) => match workers {
				Some(workers) => workers.manager().subscribe(listener),
				None => listeners.push(listener),
			},
		}
	};

	// Every started request borrows the store, so each finishes before the caller may hand the store on.
	while running.next().await.is_some() {}

	ended
}

/// Reads `id` for a request, after `workers` made it current, and tracks it for hot reload.
async fn request_resource(
	reads: &dyn DynReadStorageBackend,
	workers: Option<&AssetWorkers<'_>>,
	id: &str,
) -> Result<(SerializableResource, MultiResourceReader), RequestError> {
	let Some(workers) = workers else {
		return reads
			.read(ResourceId::new(id))
			.await
			.ok_or_else(|| RequestError::Missing { id: id.to_owned() });
	};
	// Release builds only bake in tools, whose requests don't explain bake failures.
	#[cfg(debug_assertions)]
	let missing = |source| RequestError::bake(id, source, workers.manager());
	#[cfg(not(debug_assertions))]
	let missing = |_: LoadMessages| RequestError::Missing { id: id.to_owned() };
	workers.bake_if_stale(id).await.map_err(missing)?;
	let Some((stored, reader)) = reads.read(ResourceId::new(id)).await else {
		return Err(missing(LoadMessages::NoAsset));
	};
	#[cfg(debug_assertions)]
	workers.manager().track_resource(stored.id(), stored.baked_at());
	Ok((stored, reader))
}

/// Serves a hosted [`ResourceManager`]: reads its store as it is until assets are installed, then bakes into it with
/// the installed asset manager's workers until the manager is dropped.
#[cfg(debug_assertions)]
fn host(store: Box<dyn DynStorageBackend>, requests: flume::Receiver<StoreRequest>) {
	let runtime = compio::runtime::Runtime::new().expect(
		"Failed to start the resource host runtime. The most likely cause is that the platform I/O driver could not be initialized.",
	);
	let served = runtime.block_on(serve(store.as_ref(), None, &requests, None));
	drop(runtime);
	let Served::Install(install, listeners) = served else {
		return;
	};

	let asset_manager = install(store);
	for listener in listeners {
		asset_manager.subscribe(listener);
	}
	// The watcher's callback only signals; the serving loop below runs the checks on the workers.
	let (changed, sources_changed) = flume::unbounded();
	let _watcher = asset_manager.watch(move || {
		let _ = changed.send(());
	});

	asset_manager.with_workers(async |workers| {
		serve(
			asset_manager.resource_storage(),
			Some(workers),
			&requests,
			Some(&sources_changed),
		)
		.await
	});
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
			storage_backend::{MemoryStorageBackend, Query, QueryError},
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
		let storage = MemoryStorageBackend::new();

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
	async fn request_reports_an_unreadable_dependency_by_id() {
		let storage = MemoryStorageBackend::new();
		let image = Image {
			format: Formats::RGBA8,
			gamma: Gamma::Linear,
			extent: [1, 1, 0],
			mip_count: 1,
			cpu_readable: false,
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
		sync::atomic::{AtomicUsize, Ordering},
		time::{SystemTime, UNIX_EPOCH},
	};

	use utils::sync::Mutex;

	use super::ResourceManager;
	use crate::{
		ProcessedAsset,
		asset::{
			ResourceId, ResourceTraceLevel,
			handler::{AssetHandler, BakeContext, LoadErrors, PrimaryStored},
			manager::AssetManager,
			storage_backend::{FileStorageBackend, tests::TestStorageBackend as AssetTestStorageBackend},
		},
		r#async,
		resource::storage_backend::MemoryStorageBackend,
		resources::material::{Shader, ShaderArtifact, ShaderInterface},
		types::ShaderTypes,
	};

	struct ResolvingAssetHandler;

	impl AssetHandler for ResolvingAssetHandler {
		fn can_handle(&self, extension: &str) -> bool {
			matches!(extension, "besl" | "test")
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
			context.resolve(id).await?;
			// The tests only request missing sources, so a bake never gets past the read.
			Err(LoadErrors::FailedToProcess)
		}
	}

	struct CoordinatingShaderHandler {
		invocations: AtomicUsize,
		started: Mutex<Option<announcement::Announcer<()>>>,
		release: announcement::Listener<()>,
	}

	struct VersionedShaderHandler {
		invocations: AtomicUsize,
	}

	impl AssetHandler for VersionedShaderHandler {
		fn can_handle(&self, extension: &str) -> bool {
			extension == "test"
		}

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
			self.invocations.fetch_add(1, Ordering::SeqCst);

			let source = context.resolve(id).await?;

			context
				.store_primary(
					ProcessedAsset::new(
						id,
						Shader {
							id: id.to_string(),
							stage: ShaderTypes::Compute,
							interface: ShaderInterface {
								workgroup_size: None,
								sets: Vec::new(),
								ray_payload: None,
								tile: None,
							},
							artifact: ShaderArtifact::Spirv,
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

		async fn bake<'a>(&'a self, context: BakeContext<'a>, id: ResourceId<'a>) -> Result<PrimaryStored, LoadErrors> {
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
								sets: Vec::new(),
								ray_payload: None,
								tile: None,
							},
							artifact: ShaderArtifact::Spirv,
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
		let resource_manager = ResourceManager::hosted(MemoryStorageBackend::new());

		resource_manager.install_assets(FileStorageBackend::new(path), |asset_manager| {
			asset_manager.add_asset_handler(ResolvingAssetHandler);
		});

		resource_manager
	}

	/// Returns how many times the host's `T` handler baked, as `count` reads it.
	async fn bakes<T: 'static>(resource_manager: &ResourceManager, count: fn(&T) -> usize) -> usize {
		resource_manager
			.on_assets(async move |workers| count(workers.manager().handler_as::<T>()))
			.await
			.expect("the test manager hosts asset management")
	}

	#[r#async::test]
	async fn inaccessible_engine_asset_root_suggests_configuring_the_symlink() {
		let assets = temporary_asset_directory("missing-root");

		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("byte_engine/missing.test")
			.await
			.unwrap_err();

		assert_eq!(
			error.to_string(),
			format!(
				"Could not bake asset.\n\n  Asset: byte_engine/missing.test\n  Cause: The source asset could not be read.\n  Fix: Configure or repair the 'assets/byte_engine' directory link, then retry.\n  Guide: {}",
				super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)
			)
		);

		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn missing_engine_asset_namespace_suggests_checking_the_asset_id() {
		let assets = temporary_asset_directory("accessible-root");

		fs::create_dir_all(assets.join("byte_engine")).unwrap();

		let resource_manager = resource_manager_with_file_assets(assets.clone());

		let error = resource_manager
			.request::<Shader>("rendering/simple/vertex.besl")
			.await
			.unwrap_err();

		assert_eq!(
			error.to_string(),
			format!(
				"Could not bake asset.\n\n  Asset: rendering/simple/vertex.besl\n  Cause: The source asset could not be read.\n  Fix: Check the asset ID and configured assets directory. Engine asset IDs start with 'byte_engine/'.\n  Guide: {}",
				super::online_docs_url(super::BAKING_APP_RESOURCES_DOCS_PATH)
			)
		);

		let items = resource_manager
			.on_assets(async |workers| workers.manager().resource_trace().items("rendering/simple/vertex.besl"))
			.await
			.expect("installed asset management should expose its trace");

		assert_eq!(items.len(), 1);
		assert_eq!(items[0].level(), ResourceTraceLevel::Error);

		fs::remove_dir_all(assets).unwrap();
	}

	#[r#async::test]
	async fn concurrent_resource_requests_share_one_missing_asset_bake() {
		let (started, started_announcement) = announcement::Announcement::new();

		let (release, release_announcement) = announcement::Announcement::new();

		let resource_manager = ResourceManager::hosted(MemoryStorageBackend::new());

		resource_manager.install_assets(AssetTestStorageBackend::new(), move |asset_manager| {
			asset_manager.add_asset_handler(CoordinatingShaderHandler {
				invocations: AtomicUsize::new(0),
				started: Mutex::new(Some(started)),
				release: release_announcement.listener(),
			});
		});

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
		assert_eq!(
			bakes(&resource_manager, |handler: &CoordinatingShaderHandler| handler
				.invocations
				.load(Ordering::SeqCst))
			.await,
			1
		);
	}

	#[r#async::test]
	async fn debug_resource_requests_rebake_only_after_the_requested_asset_changes() {
		let asset_storage = AssetTestStorageBackend::new();

		asset_storage.add_file("versioned.test", b"first shader");

		let resource_manager = ResourceManager::hosted(MemoryStorageBackend::new());

		resource_manager.install_assets(asset_storage, |asset_manager| {
			asset_manager.add_asset_handler(VersionedShaderHandler {
				invocations: AtomicUsize::new(0),
			});
		});

		let invocations = async || {
			bakes(&resource_manager, |handler: &VersionedShaderHandler| {
				handler.invocations.load(Ordering::SeqCst)
			})
			.await
		};

		let first = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("initial debug request should bake");

		let unchanged = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("unchanged debug request should reuse the resource");

		assert_eq!(first.hash(), unchanged.hash());
		assert_eq!(invocations().await, 1);

		resource_manager
			.on_assets(async |workers| {
				workers
					.manager()
					.test_sources()
					.add_file("versioned.test", b"changed shader source")
			})
			.await
			.unwrap();

		let changed = resource_manager
			.request::<Shader>("versioned.test")
			.await
			.expect("changed debug source should rebake");

		assert_ne!(first.hash(), changed.hash());
		assert_eq!(invocations().await, 2);
	}
}

use utils::r#async::oneshot;

use super::{
	DynReadStorageBackend, DynStorageBackend, StorageBackend,
	resource_handler::MultiResourceReader,
	storage_backend::{Query, QueryError, QueryPage},
};
#[cfg(debug_assertions)]
use crate::asset::handler::LoadErrors;
use crate::asset::{
	manager::{AssetManager, LoadMessages},
	workers::AssetWorkers,
};
use crate::{
	Reference, Resource, SerializableResource, StoredModel, asset::ResourceId, r#async::BoxedFuture, online_docs_url,
	solver::SolveError,
};
