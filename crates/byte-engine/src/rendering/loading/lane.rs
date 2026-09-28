//! The worker half of the loading couple: one sequential lane per async task.

use std::{
	collections::HashSet,
	hash::Hash,
	sync::{
		Arc, Mutex,
		atomic::{AtomicUsize, Ordering},
	},
};

use smallvec::SmallVec;

use super::{
	client::LoaderClient,
	loader::{BufferRegion, ImageUpload, Loader, NativeImageUpload, UploadRequest},
};
use crate::rendering::resource_loading::{StagingLease, UploadStagingArena};

/// The `LoadError` struct reports why one resource could not be made resident.
#[derive(Debug)]
pub struct LoadError(pub String);

impl std::fmt::Display for LoadError {
	fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		formatter.write_str(&self.0)
	}
}

/// The `LoadPipeline` trait defines how one rendering pipeline turns all its requests into resident GPU resources.
///
/// Implement this trait once per rendering pipeline. Represent meshes, materials, textures, and other
/// resource families as variants of the associated types so they share one request registry and lane pool.
/// Every method runs on the loading thread. Implementations are shared across lanes by reference, so any
/// mutable loader-owned storage belongs behind interior mutability held by the implementation itself.
///
/// The returned future is deliberately not `Send`: resource reads are driven by a per-thread runtime, so a
/// lane's future never migrates between threads once it starts.
pub trait LoadPipeline: Send + Sync + 'static {
	/// Stable logical identity used to coalesce duplicate requests.
	type Key: Clone + Eq + Hash + Send + 'static;
	/// Owned input moved from the render thread or another lane to one lane.
	type Request: Send + 'static;
	/// The finished value the render thread adopts.
	///
	/// Its uploads are complete. Detached images in it get their render handles when the render thread interns
	/// them while adopting the value.
	type Resident: Send + 'static;

	/// Derives the stable logical identity used to coalesce one request.
	fn key(request: &Self::Request) -> Self::Key;

	/// Loads one resource end to end.
	///
	/// Request the resources this one depends on with [`LoaderLane::request`] as soon as you know them, so they load
	/// while this one does. Fetch and decode, write the bytes into a [`StagingLease`], then hand the GPU work to the
	/// loader through [`LoaderLane::upload`] or [`LoaderLane::upload_native_image`]. The loader batches it with other
	/// lanes' uploads and answers once the copies finished.
	fn load(
		&self,
		request: Self::Request,
		lane: &mut LoaderLane<Self>,
	) -> impl Future<Output = Result<Self::Resident, LoadError>>;
}

/// The `Submission` struct carries one request through the stream that the client and every lane send to.
pub(super) struct Submission<P: LoadPipeline + ?Sized> {
	pub(super) key: P::Key,
	pub(super) request: P::Request,
	/// Loads the resource again even when its key is already loading or resident.
	pub(super) reload: bool,
}

/// The `LoaderLane` struct is one sequential worker that prepares uploads for the [`Loader`].
///
/// Run each lane on its own async task on the loading thread. Lanes compete for the same request stream, so lane
/// count is how many resources read and decode at once. The GPU work of every lane goes through the one loader.
pub struct LoaderLane<P: LoadPipeline + ?Sized> {
	pipeline: Arc<P>,
	staging: Arc<UploadStagingArena>,
	uploads: kanal::AsyncSender<UploadRequest>,
	/// Raised while this lane loads a resource, so the loader knows when waiting cannot collect more uploads.
	busy_lanes: Arc<AtomicUsize>,
	/// Keys the pipeline's lanes load or loaded. A failed key leaves it so the resource can be requested again.
	///
	/// Only this pipeline's lanes use it, so the render thread never waits on the lock.
	registry: Arc<Mutex<HashSet<P::Key>>>,
	/// Feeds dependencies into the stream the client sends to, so one registry coalesces both.
	dependencies: kanal::AsyncSender<Submission<P>>,
	requests: kanal::AsyncReceiver<Submission<P>>,
	results: kanal::AsyncSender<(P::Key, Result<P::Resident, LoadError>)>,
}

impl<P: LoadPipeline> LoaderLane<P> {
	/// Returns the shared staging arena that upload bytes are written into.
	pub fn staging(&self) -> &Arc<UploadStagingArena> {
		&self.staging
	}

	/// Requests a resource that the one being loaded depends on.
	///
	/// A lane picks it up right away, without a round trip through the render thread, and the render thread adopts
	/// it like any other result. Requests for a key already loading or resident are ignored.
	pub fn request(&self, request: P::Request) {
		// A closed stream means the client is gone, so nothing would adopt the result.
		let _ = self.dependencies.as_sync().try_send(Submission {
			key: P::key(&request),
			request,
			reload: false,
		});
	}

	/// Creates images from staged mips, copies staged bytes into render buffers, and waits until the loader
	/// finished every copy.
	///
	/// Every buffer destination is a handle from [`Loader::import_buffer`]. Write only ranges no frame reads yet.
	/// The loader returns `staging` to the arena once the copies finished. The images come back detached, in upload
	/// order, ready for the render thread to intern.
	pub async fn upload<const N: usize>(
		&self,
		staging: StagingLease,
		images: [ImageUpload; N],
		buffers: SmallVec<[BufferRegion; 16]>,
	) -> Result<[ghi::implementation::DetachedImage; N], LoadError> {
		let images = self
			.send(|reply| UploadRequest::Staged {
				staging,
				images: SmallVec::from_iter(images),
				buffers,
				reply,
			})
			.await?;
		let mut images = images.into_iter();
		Ok(std::array::from_fn(|_| {
			images
				.next()
				.expect("The loader returns one image per upload. The most likely cause is a mismatched loader reply.")
		}))
	}

	/// Creates one image, fills it straight from its file through native I/O, and waits until the reads finished.
	pub async fn upload_native_image(
		&self,
		upload: NativeImageUpload,
	) -> Result<ghi::implementation::DetachedImage, LoadError> {
		self.send(|reply| UploadRequest::NativeImage { upload, reply })
			.await?
			.map_err(LoadError)
	}

	/// Sends one upload to the loader and waits for its result without blocking other lanes.
	async fn send<T>(&self, upload: impl FnOnce(kanal::Sender<T>) -> UploadRequest) -> Result<T, LoadError> {
		let (reply, result) = kanal::bounded_async(1);
		let stopped = || {
			LoadError(
				"The loader stopped before finishing an upload. The most likely cause is that the application is shutting down."
					.to_string(),
			)
		};
		self.uploads.send(upload(reply.to_sync())).await.map_err(|_| stopped())?;
		result.recv().await.map_err(|_| stopped())
	}

	/// Serves requests until the client is dropped.
	pub async fn run(mut self) {
		while let Ok(Submission { key, request, reload }) = self.requests.recv().await {
			// The client and every lane send to this stream, so coalesce here. A reload still records its key.
			if !self
				.registry
				.lock()
				.unwrap_or_else(|error| error.into_inner())
				.insert(key.clone())
				&& !reload
			{
				continue;
			}
			let pipeline = self.pipeline.clone();
			self.busy_lanes.fetch_add(1, Ordering::AcqRel);
			let result = pipeline.load(request, &mut self).await;
			self.busy_lanes.fetch_sub(1, Ordering::AcqRel);
			if result.is_err() {
				self.registry.lock().unwrap_or_else(|error| error.into_inner()).remove(&key);
			}
			if self.results.send((key, result)).await.is_err() {
				break;
			}
		}
	}
}

/// Creates the single client and lane pool for one rendering pipeline.
///
/// Keep the client on the render thread and move every lane to a task on the loading thread, where
/// [`Loader::run`] runs too.
pub fn spawn<P: LoadPipeline>(
	loader: &Loader,
	pipeline: P,
	lane_count: usize,
	queue_capacity: usize,
) -> (LoaderClient<P>, Vec<LoaderLane<P>>) {
	let pipeline = Arc::new(pipeline);
	let registry = Arc::new(Mutex::new(HashSet::new()));
	let (request_sender, request_receiver) = kanal::unbounded_async();
	let (result_sender, result_receiver) = kanal::bounded_async(queue_capacity);

	let lanes = (0..lane_count.max(1))
		.map(|_| LoaderLane {
			pipeline: pipeline.clone(),
			staging: loader.batcher.staging.clone(),
			uploads: loader.uploads.clone(),
			busy_lanes: loader.batcher.busy_lanes.clone(),
			registry: registry.clone(),
			dependencies: request_sender.clone(),
			requests: request_receiver.clone(),
			results: result_sender.clone(),
		})
		.collect();

	(LoaderClient::new(request_sender, result_receiver), lanes)
}
