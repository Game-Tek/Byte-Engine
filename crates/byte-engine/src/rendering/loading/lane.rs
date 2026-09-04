//! The worker half of the loading couple: one sequential lane per async task.

use std::{hash::Hash, sync::Arc};

use ghi::{context::Context as _, context::ContextCreate as _, queue::Queue as _};

use super::client::LoaderClient;
use crate::rendering::{SharedContext, resource_loading::UploadStagingArena};

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
/// Every method runs on a loader thread. Implementations are shared across lanes by reference, so any
/// mutable loader-owned storage belongs behind interior mutability held by the implementation itself.
///
/// The returned future is deliberately not `Send`: resource reads are driven by a per-thread runtime, so a
/// lane's future never migrates between threads once it starts.
pub trait LoadPipeline: Send + Sync + 'static {
	/// Stable logical identity used to coalesce duplicate scene requests.
	type Key: Clone + Eq + Hash + Send + 'static;
	/// Owned input moved from the render thread to one lane.
	type Request: Send + 'static;
	/// The finished value the render thread adopts. Its GPU objects are already interned.
	type Resident: Send + 'static;

	/// Derives the stable logical identity used to coalesce one request.
	fn key(request: &Self::Request) -> Self::Key;

	/// Loads one resource end to end.
	///
	/// Do the expensive work first and without touching the context: fetch, decode, write staging memory,
	/// and create detached objects through [`LoaderLane::factory`]. Take the context only at the end,
	/// through [`LoaderLane::commit`], to intern those objects and record their transfers.
	fn load(
		&self,
		request: Self::Request,
		lane: &mut LoaderLane<Self>,
	) -> impl Future<Output = Result<Loaded<Self>, LoadError>>;
}

/// The `Loaded` struct pairs a resident resource with the further requests its content implies.
///
/// Dependencies return to the client rather than being requested by the lane, so one registry coalesces
/// every request in flight.
pub struct Loaded<P: LoadPipeline + ?Sized> {
	pub resident: P::Resident,
	pub dependencies: Vec<P::Request>,
}

impl<P: LoadPipeline + ?Sized> Loaded<P> {
	/// Creates a result that implies no further loading.
	pub fn new(resident: P::Resident) -> Self {
		Self {
			resident,
			dependencies: Vec::new(),
		}
	}
}

/// The `LoaderLane` struct is one sequential worker with its own context-free GPU toolbox.
///
/// Run each lane on its own async task. Lanes compete for the same request stream, so lane count is the
/// loading concurrency.
pub struct LoaderLane<P: LoadPipeline + ?Sized> {
	pipeline: Arc<P>,
	factory: ghi::implementation::Factory,
	staging: Arc<UploadStagingArena>,
	context: SharedContext,
	command_buffer: ghi::CommandBufferHandle,
	synchronizer: ghi::SynchronizerHandle,
	requests: kanal::AsyncReceiver<(P::Key, P::Request)>,
	results: kanal::AsyncSender<(P::Key, Result<Loaded<P>, LoadError>)>,
}

impl<P: LoadPipeline> LoaderLane<P> {
	/// Creates detached GPU objects without touching the context.
	pub fn factory(&mut self) -> &mut ghi::implementation::Factory {
		&mut self.factory
	}

	/// Returns the shared staging arena backing this lane's uploads.
	pub fn staging(&self) -> &Arc<UploadStagingArena> {
		&self.staging
	}

	/// Takes the context to intern factory objects, then releases it.
	///
	/// The render thread holds the context for a whole frame, so a commit can wait that long. Batch the
	/// work of several resources into one commit rather than committing each one separately.
	pub fn commit<T>(&self, work: impl FnOnce(&mut ghi::implementation::Context) -> T) -> T {
		work(&mut self.context.lock())
	}

	/// Records transfers on this lane's command buffer, submits them, and waits for the copies to finish.
	///
	/// On return the copies have completed, which is what makes it safe to drop a
	/// [`StagingLease`](crate::rendering::resource_loading::StagingLease) at the end of a load.
	///
	/// The context is taken twice, once to record and submit and once to wait, so the render thread can run a
	/// frame in between. The wait itself still holds the context, because two of the three backends recycle
	/// command state while waiting. Giving the loader its own queue and a borrow-free wait would remove that
	/// last stall.
	pub fn transfer<T>(&self, record: impl FnOnce(&mut ghi::implementation::CommandBufferRecording<'_>) -> T) -> T {
		use ghi::command_buffer::CommandBufferRecording as _;

		let value = self.commit(|context| {
			let mut recording = context.create_command_buffer_recording(self.command_buffer);
			let value = record(&mut recording);
			recording.execute(self.synchronizer);
			value
		});
		self.commit(|context| context.wait_for_synchronizer(self.synchronizer));
		value
	}

	/// Serves requests until the client is dropped.
	pub async fn run(mut self) {
		while let Ok((key, request)) = self.requests.recv().await {
			let pipeline = self.pipeline.clone();
			let result = pipeline.load(request, &mut self).await;
			if self.results.send((key, result)).await.is_err() {
				break;
			}
		}
	}
}

/// Creates the single client and shared lane pool for one rendering pipeline.
///
/// Keep the client on the render thread and move every lane to an application-owned async task. The
/// application must join those tasks before dropping the renderer or the arena's backing buffer.
///
/// # Panics
///
/// Panics when the context cannot produce a detached factory for a lane.
pub fn spawn<P: LoadPipeline>(
	context: &SharedContext,
	queue: ghi::QueueHandle,
	pipeline: P,
	staging: Arc<UploadStagingArena>,
	lane_count: usize,
	queue_capacity: usize,
) -> (LoaderClient<P>, Vec<LoaderLane<P>>) {
	let pipeline = Arc::new(pipeline);
	let (request_sender, request_receiver) = kanal::unbounded_async();
	let (result_sender, result_receiver) = kanal::bounded_async(queue_capacity);

	let lanes = (0..lane_count.max(1))
		.map(|lane| {
			let mut owner = context.lock();
			let factory = owner.create_factory().expect(
				"Failed to create a loader factory. The most likely cause is that the graphics device does not support detached resource creation.",
			);
			let command_buffer = owner
				.queue(queue)
				.create_command_buffer(Some(&format!("Resource Loader Lane {lane}")));
			let synchronizer = owner.create_synchronizer(Some(&format!("Resource Loader Lane {lane}")), false);
			drop(owner);
			LoaderLane {
				pipeline: pipeline.clone(),
				factory,
				staging: staging.clone(),
				context: context.clone(),
				command_buffer,
				synchronizer,
				requests: request_receiver.clone(),
				results: result_sender.clone(),
			}
		})
		.collect();

	(LoaderClient::new(request_sender, result_receiver), lanes)
}
