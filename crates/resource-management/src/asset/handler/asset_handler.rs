/// The `AssetHandler` trait provides source types for asset baking.
///
/// See the [assets guide](/docs/develop/resource-management/assets)
/// before implementing a new source-format handler.
pub trait AssetHandler {
	fn can_handle(&self, r#type: &str) -> bool;

	/// Returns whether recursive asset discovery should include a source handled by this implementation.
	fn should_discover(&self, _id: ResourceId<'_>, _has_sidecar: bool) -> bool {
		true
	}

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> impl Future<Output = Result<(), LoadErrors>>;
}

/// The `DynAssetHandler` trait lets the asset manager keep handlers of different types in one list.
///
/// Every [`AssetHandler`] implements it; register handlers with
/// [`AssetManager::add_asset_handler`](crate::asset::manager::AssetManager::add_asset_handler).
pub trait DynAssetHandler: Send + Sync {
	fn can_handle(&self, r#type: &str) -> bool;

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool;

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>>;
}

impl<T: AssetHandler + Send + Sync> DynAssetHandler for T {
	fn can_handle(&self, r#type: &str) -> bool {
		AssetHandler::can_handle(self, r#type)
	}

	fn should_discover(&self, id: ResourceId<'_>, has_sidecar: bool) -> bool {
		AssetHandler::should_discover(self, id, has_sidecar)
	}

	fn bake<'a>(&'a self, context: BakeContext<'a>, url: ResourceId<'a>) -> BoxedFuture<'a, Result<(), LoadErrors>> {
		Box::pin(AssetHandler::bake(self, context, url))
	}
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadErrors {
	FailedToProcess,
	AssetCouldNotBeRead,
	AssetCouldNotBeLoaded,
	UnsupportedType,
	FailedToStore,
	PrimaryResourceIdMismatch,
	PrimaryResourceNotStored,
	/// A dependency matches an excluded pattern, so it is never baked.
	DependencyExcluded,
}

// Only debug builds bake assets and record why a bake failed.
#[cfg(debug_assertions)]
impl LoadErrors {
	/// Returns the developer-facing cause for this asset loading failure.
	pub(crate) const fn message(&self) -> &'static str {
		match self {
			Self::FailedToProcess => "The source asset could not be processed.",
			Self::AssetCouldNotBeRead => "The source asset could not be read.",
			Self::AssetCouldNotBeLoaded => "The source asset could not be loaded.",
			Self::UnsupportedType => "The source asset type is unsupported.",
			Self::FailedToStore => "The baked resource could not be stored.",
			Self::PrimaryResourceIdMismatch => "The asset handler stored the primary resource under a different ID.",
			Self::PrimaryResourceNotStored => "The asset handler did not store the primary resource.",
			Self::DependencyExcluded => "A dependency of the source asset matches `resources.exclude`, so it is never baked.",
		}
	}

	/// Returns the recovery step most likely to resolve this asset loading failure.
	pub(crate) const fn fix(&self) -> &'static str {
		match self {
			Self::AssetCouldNotBeRead => {
				"Check the asset ID and configured assets directory. Engine asset IDs start with 'byte_engine/'."
			}
			Self::FailedToProcess | Self::AssetCouldNotBeLoaded => {
				"Check the source asset and its dependencies for invalid or unsupported data."
			}
			Self::UnsupportedType => "Use a supported asset type or register an asset handler for it.",
			Self::FailedToStore => "Check that the resource destination is writable, then retry.",
			Self::PrimaryResourceIdMismatch => "Store the primary resource under the requested asset ID.",
			Self::PrimaryResourceNotStored => "Make the asset handler store its primary resource before returning.",
			Self::DependencyExcluded => {
				"Stop using the excluded dependency, which the warning before this error names, or remove its pattern from `resources.exclude`."
			}
		}
	}
}

/// The `TrackingStorageBackend` struct records every source resolved during one asset bake.
///
/// It records a source's version only after a successful read whose version matched on both sides of the read.
pub(crate) struct TrackingStorageBackend<'a> {
	pub(in crate::asset) inner: &'a dyn asset::DynStorageBackend,
	/// The provenance attached to every resource the bake stores, kept sorted by source ID so it persists
	/// deterministically.
	pub(in crate::asset) dependencies: &'a Mutex<Vec<AssetDependency>>,
}

impl TrackingStorageBackend<'_> {
	/// Runs `read` between two version checks of `id` and records its version, rejecting the result when `id` changed
	/// during the read.
	///
	/// A read that fails because `id` doesn't exist records `id` as missing, so creating it later rebakes the resource.
	/// Handlers that look for optional files, such as the two files a BESL module may live in, rely on this.
	async fn tracked<T, F: Future<Output = Result<T, ()>>>(
		&self,
		id: ResourceId<'_>,
		read: impl FnOnce() -> F,
	) -> Result<T, ()> {
		let before = self.inner.version(id).await?;
		let resolved = match read().await {
			Ok(resolved) => resolved,
			Err(()) => {
				if before == AssetVersion::missing() {
					upsert_dependency(&mut self.dependencies.lock(), AssetDependency::new(id, before));
				}
				return Err(());
			}
		};
		let after = self.inner.version(id).await?;

		if before != after {
			log::warn!(
				"Asset changed while it was being read. The most likely cause is that '{}' was saved during the bake; retry the request.",
				id.as_ref()
			);

			return Err(());
		}

		// Records the latest observed version once when handlers resolve the same source repeatedly.
		upsert_dependency(&mut self.dependencies.lock(), AssetDependency::new(id, after));

		Ok(resolved)
	}
}

impl asset::StorageBackend for TrackingStorageBackend<'_> {
	fn directory_accessible(&self, path: &std::path::Path) -> Option<bool> {
		self.inner.directory_accessible(path)
	}

	async fn resolve<'a>(&'a self, url: ResourceId<'a>) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(url, || self.inner.resolve(url)).await
	}

	async fn resolve_in<'a>(
		&'a self,
		url: ResourceId<'a>,
		allocator: &'a dyn Allocator,
	) -> Result<(AssetStorageBytes<'a>, String), ()> {
		self.tracked(url, || self.inner.resolve_in(url, allocator)).await
	}

	async fn load_bead<'a>(&'a self, id: ResourceId<'a>) -> Result<Option<BEADType>, ()> {
		// Track absence as well as content, so creating a sidecar or directory default invalidates the baked resource.
		self.tracked(id, || self.inner.load_bead(id)).await
	}

	fn version<'a>(&'a self, url: ResourceId<'a>) -> impl std::future::Future<Output = Result<AssetVersion, ()>> + 'a {
		self.inner.version(url)
	}
}

/// The `BakeContext` struct provides format handlers with the shared facilities used during one asset bake.
#[derive(Clone, Copy)]
pub struct BakeContext<'a> {
	pub(in crate::asset) asset_manager: &'a Arc<AssetManagerState>,
	pub(in crate::asset) asset_storage_backend: &'a TrackingStorageBackend<'a>,
	pub(in crate::asset) allocator: &'a BakeAllocator,
	pub(in crate::asset) primary_id: ResourceId<'a>,
	pub(in crate::asset) primary_stored: &'a Cell<bool>,
}

impl<'a> BakeContext<'a> {
	/// Adds an informational item to this resource's development trace and terminal log.
	pub fn info(&self, message: impl fmt::Display) {
		self.log(log::Level::Info, message);
	}

	/// Adds a warning item to this resource's development trace and terminal log.
	pub fn warn(&self, message: impl fmt::Display) {
		self.log(log::Level::Warn, message);
	}

	/// Adds an error item to this resource's development trace and terminal log.
	///
	/// The item remains available when the handler returns an error and does not
	/// store the requested resource.
	pub fn error(&self, message: impl fmt::Display) {
		self.log(log::Level::Error, message);
	}

	/// Writes one message to the terminal log and, in development builds, to this resource's trace.
	fn log(&self, level: log::Level, message: impl fmt::Display) {
		#[cfg(debug_assertions)]
		let message = message.to_string();

		log::log!(level, "{message}");

		#[cfg(debug_assertions)]
		self.asset_manager.resource_trace.record(
			self.primary_id,
			match level {
				log::Level::Error => ResourceTraceLevel::Error,
				log::Level::Warn => ResourceTraceLevel::Warn,
				log::Level::Info | log::Level::Debug | log::Level::Trace => ResourceTraceLevel::Info,
			},
			message,
		);
	}

	/// Resolves only the requested source bytes with the bake allocator.
	pub async fn resolve<'b>(&'b self, id: ResourceId<'b>) -> Result<(AssetStorageBytes<'b>, String), LoadErrors> {
		self.asset_storage_backend
			.resolve_in(id, self.allocator)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Loads optional BEAD settings, the directory `.bead` defaults merged under the source's sidecar, and records the
	/// presence or absence of each file as a bake dependency.
	///
	/// Call this only when the handler consumes sidecar settings, then read the source with [`Self::resolve`].
	pub async fn load_sidecar(&self, id: ResourceId<'_>) -> Result<Option<BEADType>, LoadErrors> {
		self.asset_storage_backend
			.load_sidecar(id)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)
	}

	/// Returns whether the source asset `id` exists, without reading it, and records its version as a bake dependency.
	///
	/// Use it to choose among optional files: an absent file is recorded as missing, so creating it later makes the
	/// resource stale, and a present one rebakes the resource when it changes.
	pub async fn source_exists(&self, id: ResourceId<'_>) -> Result<bool, LoadErrors> {
		let version = self
			.asset_storage_backend
			.inner
			.version(id)
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)?;
		let exists = version != AssetVersion::missing();
		upsert_dependency(
			&mut self.asset_storage_backend.dependencies.lock(),
			AssetDependency::new(id, version),
		);
		Ok(exists)
	}

	/// Returns the IDs of every stored resource `query` matches, as the resource store holds them now.
	///
	/// The bake records no dependency on what it finds, so a resource built from a query is stale only when it is baked
	/// again on purpose: BELD bakes such assets after every other one, and development builds rebake them on request.
	pub async fn query_resources(&self, query: crate::resource::storage_backend::Query) -> Result<Vec<String>, LoadErrors> {
		let page = self
			.asset_manager
			.resource_storage_backend
			.query(query)
			.await
			.map_err(|_| LoadErrors::FailedToProcess)?;
		Ok(page.items.into_iter().map(|(stored, _)| stored.id().to_owned()).collect())
	}

	/// Bakes a referenced source asset on the shared worker pool when necessary and returns its stored model.
	///
	/// Concurrent calls from one handler bake on separate workers, each in its own arena.
	pub async fn bake_dependency<M: Model>(&self, id: &str) -> Result<ReferenceModel<M>, LoadErrors> {
		Ok(self.bake_dependencies(&[id.to_owned()], 1).await?.remove(0))
	}

	/// Bakes independent dependencies on the shared worker pool while bounding active requests.
	///
	/// Results preserve the input order. Each completed request returns its already-read
	/// resource so provenance does not require another serialized storage pass.
	pub async fn bake_dependencies<M: Model>(
		&self,
		ids: &[String],
		max_concurrency: usize,
	) -> Result<Vec<ReferenceModel<M>>, LoadErrors> {
		use utils::r#async::StreamExt as _;

		let requests = ids.iter().enumerate().map(|(index, id)| async move {
			self.asset_manager
				.dispatch_bake_in_scope(
					id,
					true,
					crate::asset::manager::BakeOrigin::Dependency(self.allocator.memory_scope().cloned()),
				)
				.await
				.map_err(dependency_load_error)?;

			let Some((resource, _)) = self.asset_manager.resource_storage_backend.read(ResourceId::new(id)).await else {
				return Err(LoadErrors::FailedToProcess);
			};

			Ok((index, resource))
		});

		let mut completed = utils::r#async::stream::iter(requests)
			.buffer_unordered(max_concurrency.max(1))
			.collect::<Vec<_>>()
			.await
			.into_iter()
			.collect::<Result<Vec<_>, _>>()?;

		completed.sort_unstable_by_key(|(index, _)| *index);

		let mut dependencies = Vec::with_capacity(completed.len());

		for (_, resource) in completed {
			self.inherit_dependency_provenance(&resource);

			dependencies.push(resource.into());
		}

		Ok(dependencies)
	}

	/// Solves a dependency [`Self::bake_dependency`] or [`Self::bake_dependencies`] returned into its runtime resource,
	/// for a handler that reads what it baked, like a motion database that samples its clips.
	///
	/// Fails when the stored resource is of another class than `M`'s, or doesn't solve.
	pub async fn solve<M: StoredModel>(&self, dependency: ReferenceModel<M>) -> Result<Reference<M::Resource>, LoadErrors> {
		let storage: &dyn crate::resource::DynReadStorageBackend = self.asset_manager.resource_storage_backend.as_ref();
		let (stored, reader) = storage.read(dependency.id()).await.ok_or(LoadErrors::FailedToProcess)?;
		if stored.class() != M::get_class() {
			return Err(LoadErrors::FailedToProcess);
		}
		M::solve_stored(stored, reader, storage)
			.await
			.map_err(|_| LoadErrors::FailedToProcess)
	}

	/// Returns a resource manager over the resources this bake stores, for a handler that reads its baked dependencies
	/// through the runtime's own loading code, like a lightmap bake that loads the meshes and materials the renderer
	/// draws.
	///
	/// Bake each dependency with [`Self::bake_dependency`] first: the manager only reads what is already stored.
	pub fn resource_manager(&self) -> crate::ResourceManager {
		crate::ResourceManager::new_shared(self.asset_manager.resource_storage_backend.clone())
	}

	/// Returns the ID of every source asset the asset storage can enumerate, for a handler that selects sources by
	/// pattern, like a motion database's clips.
	///
	/// The bake records no dependency on the list, so a source added later reaches the resource only when it is baked
	/// again.
	pub async fn source_ids(&self) -> Result<Vec<String>, LoadErrors> {
		let sources = self
			.asset_storage_backend
			.inner
			.discover()
			.await
			.map_err(|_| LoadErrors::AssetCouldNotBeRead)?;
		Ok(sources.into_iter().map(|source| source.into_id()).collect())
	}

	/// Adds one stored dependency's transitive source versions to the parent bake.
	fn inherit_dependency_provenance(&self, resource: &SerializableResource) {
		let mut dependencies = self.asset_storage_backend.dependencies.lock();

		for dependency in resource.asset_dependencies() {
			upsert_dependency(&mut dependencies, dependency.clone());
		}
	}

	/// Returns a stored resource that the handler may reference instead of producing it again.
	///
	/// Use it only for content-addressed IDs, where the ID changes whenever anything that affects the content
	/// changes. Generated material shaders use it with
	/// [`PreparedBeslShader::cache_key`](crate::asset::handler::implementations::besl::PreparedBeslShader::cache_key).
	/// Resources baked before the cutoff set by
	/// [`AssetManager::rebuild_resources_baked_before`](crate::asset::manager::AssetManager::rebuild_resources_baked_before)
	/// are not returned, so a forced rebuild regenerates them.
	pub(crate) async fn reusable_resource(&self, id: ResourceId<'_>) -> Option<SerializableResource> {
		let (resource, _) = self.asset_manager.resource_storage_backend.read(id).await?;

		(!self.asset_manager.precedes_rebuild_cutoff(&resource)).then_some(resource)
	}

	/// Reserves exact resource storage before a processor starts writing its payload.
	///
	/// This incremental authoring path always stores bytes uncompressed. Use
	/// [`Self::store_resource`] when the complete payload is available.
	///
	/// Write exactly `size` bytes through [`resource::ResourceTransaction::write_all`], then pass the
	/// transaction to [`Self::commit_primary`] or [`Self::commit_resource`].
	pub async fn begin_resource(
		&self,
		id: ResourceId<'_>,
		size: usize,
	) -> Result<resource::ResourceTransaction<'_>, LoadErrors> {
		self.asset_manager
			.resource_storage_backend
			.begin_resource(id, size)
			.await
			.map_err(|_| LoadErrors::FailedToStore)
	}

	/// Commits the requested primary resource after its transaction has written the declared payload.
	pub async fn commit_primary(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.commit_resource(transaction, resource).await.map(|_| ())
	}

	/// Commits a resource and records it as primary when its ID matches the current bake.
	///
	/// Generated dependencies use this path too; parent resources reference the returned metadata.
	pub async fn commit_resource(
		&self,
		transaction: resource::ResourceTransaction<'_>,
		resource: ProcessedAsset,
	) -> Result<SerializableResource, LoadErrors> {
		let resource = resource.with_asset_dependencies(self.asset_storage_backend.dependencies.lock().clone());
		let stored = transaction
			.commit(resource, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		Ok(self.mark_if_primary(stored))
	}

	/// Stores the requested resource after all of its generated dependencies are ready.
	pub async fn store_primary(&self, resource: ProcessedAsset, data: &[u8]) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.store_resource(resource, data).await.map(|_| ())
	}

	/// Stores an owned primary payload and moves large buffers directly into asynchronous file writes.
	pub async fn store_primary_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<(), LoadErrors> {
		self.ensure_primary(&resource)?;
		self.store_resource_owned(resource, data).await.map(|_| ())
	}

	/// Stores a resource and records it as the requested primary when its ID matches the current bake.
	///
	/// Generated dependencies use this path too; parent resources reference the returned metadata.
	pub async fn store_resource(&self, resource: ProcessedAsset, data: &[u8]) -> Result<SerializableResource, LoadErrors> {
		let resource = resource.with_asset_dependencies(self.asset_storage_backend.dependencies.lock().clone());

		let stored = self
			.asset_manager
			.resource_storage_backend
			.store_in(resource, data, self.allocator)
			.await
			.map_err(|_| LoadErrors::FailedToStore)?;

		Ok(self.mark_if_primary(stored))
	}

	/// Stores an owned payload and records it as primary when its ID matches the current bake.
	pub async fn store_resource_owned<T: compio::buf::IoBuf>(
		&self,
		resource: ProcessedAsset,
		data: T,
	) -> Result<SerializableResource, LoadErrors> {
		let id = ResourceId::new(resource.id());
		let storage = self.asset_manager.resource_storage_backend.as_ref();
		let transaction = resource::storage_backend::write_complete_owned_resource(
			data,
			resource.streams.as_deref(),
			storage.cpu_compression_policy(&resource),
			|size| storage.begin_resource(id, size),
		)
		.await
		.map_err(|_| LoadErrors::FailedToStore)?;

		self.commit_resource(transaction, resource).await
	}

	/// Rejects a primary store whose resource ID differs from the requested asset.
	fn ensure_primary(&self, resource: &ProcessedAsset) -> Result<(), LoadErrors> {
		if resource.id() == self.primary_id.as_ref() {
			Ok(())
		} else {
			Err(LoadErrors::PrimaryResourceIdMismatch)
		}
	}

	/// Records that the requested primary resource is stored when `stored` is that resource.
	fn mark_if_primary(&self, stored: SerializableResource) -> SerializableResource {
		if stored.id() == self.primary_id.as_ref() {
			self.primary_stored.set(true);
		}

		stored
	}

	pub(crate) fn asset_storage_backend(&self) -> &'a dyn asset::DynStorageBackend {
		self.asset_storage_backend
	}

	/// Returns the image processor shared by every handler that produces images.
	///
	/// Decode pixels into an [`ImageSource`](crate::processors::image::ImageSource), then call
	/// [`ImageProcessor::process_in`] instead of building an image resource by hand, so every source format gets the
	/// same mips, compression, and packing.
	pub fn image_processor(&self) -> &'a ImageProcessor {
		&self.asset_manager.image_processor
	}

	/// Returns the allocator shared by source resolution, processing, and resource storage for this bake.
	pub fn allocator(&self) -> &'a dyn Allocator {
		self.allocator
	}

	/// Returns the renderer's program generator, which every handler that bakes materials shares.
	///
	/// Reports the error and fails when the manager has none. Next, build the material's program with
	/// [`ProgramGenerator::generate`].
	pub fn program_generator(&self) -> Result<&'a dyn ProgramGenerator, LoadErrors> {
		self.asset_manager.program_generator.as_deref().ok_or_else(|| {
			self.error(
				"Material generation is unavailable. The most likely cause is an asset manager configured without the renderer's program generator. Install one with `AssetManager::set_program_generator`, as `configure_default_baking` does.",
			);
			LoadErrors::FailedToProcess
		})
	}

	/// Returns the renderer's transform for standalone BESL programs, or `None` when the manager has none.
	pub fn program_transform(&self) -> Option<&'a dyn ProgramTransform> {
		self.asset_manager.program_transform.as_deref()
	}

	/// Emits an intermediate product of this bake to the manager's
	/// [`BakeInterceptor`](crate::asset::intercept::BakeInterceptor), if one wants `name`.
	///
	/// `name` is the ID of the resource the product belongs to, a `.`, and the usual file extension of its format, such
	/// as `wood.bema#shader.besl`; see [`crate::asset::intercept`]. `produce` runs only when an interceptor wants the
	/// product, so emitting one is free for a bake nobody intercepts.
	pub fn intercept(&self, name: impl fmt::Display, produce: impl FnOnce() -> Vec<u8>) {
		let Some(interceptor) = &self.asset_manager.interceptor else {
			return;
		};
		let name = name.to_string();
		if interceptor.wants(&name) {
			interceptor.receive(name, produce());
		}
	}

	/// Links and lowers `program` without compiling it, for shaders whose resource ID hashes their lowered source.
	///
	/// While an interceptor is installed, the result keeps the program's BESL source, which [`Self::compile_prepared`]
	/// emits once the ID is known. Next, name the shader, then call [`Self::compile_prepared`].
	pub(crate) fn prepare_shader(
		&self,
		program: besl::parser::Node<'_>,
		settings: &ShaderGenerationSettings,
	) -> Result<PreparedBeslShader, String> {
		let source = self.asset_manager.interceptor.is_some().then(|| besl::write(&program));
		let mut prepared = prepare_besl_shader(program, settings)?;
		prepared.source = source;
		Ok(prepared)
	}

	/// Compiles `shader` with the manager's shader compiler into the shader `id`. Every shader a bake compiles goes
	/// through here.
	///
	/// Before compiling, it emits the shader's BESL source as the `<id>.besl` product, and the platform source it lowered
	/// to as `<id>.msl`, `<id>.hlsl`, or `<id>.glsl`, in the platform's shading language. After compiling, it emits the
	/// binary as `<id>.metallib` or `<id>.spv`, so tools that inspect compiled shaders, such as pipeline statistics, read
	/// exactly what the bake stored.
	pub(crate) async fn compile_prepared(
		&self,
		id: &str,
		mut shader: PreparedBeslShader,
	) -> Result<(Shader, Box<[u8]>), String> {
		if let Some(source) = shader.source.take() {
			self.intercept(format_args!("{id}.besl"), || source.into_bytes());
		}
		let language = PlatformShaderLanguage::current_platform();
		self.intercept(format_args!("{id}.{}", language.extension()), || {
			shader.platform_source().as_bytes().to_vec()
		});
		let (shader, binary) = self.asset_manager.shader_compiler.compile(shader, id).await?;
		if let Some(extension) = language.binary_extension() {
			self.intercept(format_args!("{id}.{extension}"), || binary.to_vec());
		}
		Ok((shader, binary))
	}

	/// Prepares and compiles `program` into the shader named by `settings`, for shaders whose ID is known up front.
	pub(crate) async fn compile_shader(
		&self,
		program: besl::parser::Node<'_>,
		settings: ShaderGenerationSettings,
	) -> Result<(Shader, Box<[u8]>), String> {
		let shader = self.prepare_shader(program, &settings)?;
		self.compile_prepared(&settings.name, shader).await
	}
}

/// Replaces the recorded version of a source already in `dependencies`, or inserts it so the list stays sorted by ID.
fn upsert_dependency(dependencies: &mut Vec<AssetDependency>, dependency: AssetDependency) {
	match dependencies.binary_search_by(|existing| existing.id().cmp(dependency.id())) {
		Ok(index) => dependencies[index] = dependency,
		Err(index) => dependencies.insert(index, dependency),
	}
}

/// Maps a failed dependency bake to the error its parent handler returns.
fn dependency_load_error(error: crate::asset::manager::LoadMessages) -> LoadErrors {
	match error {
		crate::asset::manager::LoadMessages::FailedToStore { .. } => LoadErrors::FailedToStore,
		crate::asset::manager::LoadMessages::Excluded { .. } => LoadErrors::DependencyExcluded,
		_ => LoadErrors::FailedToProcess,
	}
}

use std::{alloc::Allocator, cell::Cell, fmt, future::Future, sync::Arc};

use utils::sync::Mutex;

#[cfg(debug_assertions)]
use crate::asset::resource_trace::ResourceTraceLevel;
use crate::asset::{
	AssetStorageBytes, BEADType, ResourceId, StorageBackend as _,
	bake_memory::BakeAllocator,
	manager::AssetManagerState,
	storage_backend::{AssetDependency, AssetVersion},
};
use crate::{
	Model, ProcessedAsset, Reference, ReferenceModel, SerializableResource, StoredModel, asset,
	asset::handler::implementations::{
		ProgramGenerator, ProgramTransform,
		besl::{PreparedBeslShader, prepare_besl_shader},
	},
	r#async::BoxedFuture,
	processors::image::ImageProcessor,
	resource,
	resources::material::Shader,
	shader::{besl::backends::platform::PlatformShaderLanguage, generator::ShaderGenerationSettings},
};
