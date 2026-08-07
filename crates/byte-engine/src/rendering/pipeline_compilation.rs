//! Asynchronous pipeline compilation and frame-boundary publication.

/// The `PipelineKey` struct identifies one complete pipeline compilation input.
///
/// Build the value from every shader and fixed-function input that affects the
/// resulting pipeline, then pass it to [`PipelineManagerClient::request_compute`]
/// or [`PipelineManagerClient::request_raster`].
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineKey(u64);

impl PipelineKey {
	/// Creates a stable key from a hash of the complete pipeline description.
	pub const fn new(value: u64) -> Self {
		Self(value)
	}
}

/// The `PipelineRef` struct keeps a stable reference to a requested pipeline.
///
/// Poll it with [`PipelineManagerClient::get`] during frame preparation. A
/// compiled pipeline becomes visible only after the renderer publishes results
/// at the start of a frame.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PipelineRef(PipelineKey);

/// The `PipelineState` enum reports the published state of a pipeline request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PipelineState {
	Pending,
	Ready(ghi::PipelineHandle),
	Failed,
}

/// The `PipelineManagerClient` struct lets renderer dependants request and poll
/// asynchronously compiled pipelines without blocking.
///
/// Clone this client for each dependant. Requests with the same [`PipelineKey`]
/// are coalesced before they reach a compilation server.
#[derive(Clone)]
pub struct PipelineManagerClient {
	shared: Arc<PipelineManagerShared>,
	requests: kanal::Sender<PipelineRequest>,
}

impl PipelineManagerClient {
	/// Requests a detached compute pipeline and returns its stable reference.
	pub fn request_compute(
		&self,
		key: PipelineKey,
		compile: impl FnOnce(&mut ghi::implementation::Factory) -> Result<ghi::factory::ComputePipeline, String> + Send + 'static,
	) -> PipelineRef {
		self.request(key, Box::new(move |factory| compile(factory).map(DetachedPipeline::Compute)))
	}

	/// Requests a detached raster pipeline and returns its stable reference.
	pub fn request_raster(
		&self,
		key: PipelineKey,
		compile: impl FnOnce(&mut ghi::implementation::Factory) -> Result<ghi::factory::RasterPipeline, String> + Send + 'static,
	) -> PipelineRef {
		self.request(key, Box::new(move |factory| compile(factory).map(DetachedPipeline::Raster)))
	}

	/// Returns the state published for a pipeline without draining worker results.
	pub fn get(&self, pipeline: PipelineRef) -> PipelineState {
		self.shared
			.entries
			.read()
			.get(&pipeline.0)
			.copied()
			.unwrap_or(PipelineState::Failed)
	}

	/// Coalesces a request before placing compilation work on the shared queue.
	fn request(&self, key: PipelineKey, compile: PipelineCompiler) -> PipelineRef {
		let reference = PipelineRef(key);
		{
			let mut entries = self.shared.entries.write();
			if entries.contains_key(&key) {
				return reference;
			}
			entries.insert(key, PipelineState::Pending);
		}

		if self.requests.send(PipelineRequest { key, compile }).is_err() {
			self.shared.entries.write().insert(key, PipelineState::Failed);
			log::error!(
				"Pipeline request failed. The most likely cause is that every pipeline compilation server has stopped."
			);
		}

		reference
	}
}

/// The `PipelineManagerServer` struct compiles requests using one detached GHI
/// factory.
///
/// Call [`Self::run`] directly from a dedicated thread. The server does not own
/// or spawn that thread, so a future thread pool can run the same work loop.
pub struct PipelineManagerServer {
	factory: ghi::implementation::Factory,
	requests: kanal::Receiver<PipelineRequest>,
	completions: kanal::Sender<PipelineCompletion>,
}

impl PipelineManagerServer {
	/// Compiles requests until every client sender is dropped.
	pub fn run(mut self) {
		while self.serve_next() {}
	}

	/// Compiles one request, blocking until work arrives.
	///
	/// Returns `false` after shutdown. A future thread-pool integration can call
	/// this method without changing request compilation or result delivery.
	pub fn serve_next(&mut self) -> bool {
		let Ok(request) = self.requests.recv() else {
			return false;
		};
		let key = request.key;
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (request.compile)(&mut self.factory)))
			.unwrap_or_else(|_| {
				Err("Pipeline compilation panicked. The most likely cause is invalid backend pipeline input.".to_string())
			});

		self.completions.send(PipelineCompletion { key, result }).is_ok()
	}

	/// Tries to compile one queued request without waiting for work.
	pub fn try_serve_next(&mut self) -> bool {
		let Ok(Some(request)) = self.requests.try_recv() else {
			return false;
		};
		let key = request.key;
		let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| (request.compile)(&mut self.factory)))
			.unwrap_or_else(|_| {
				Err("Pipeline compilation panicked. The most likely cause is invalid backend pipeline input.".to_string())
			});

		self.completions.send(PipelineCompletion { key, result }).is_ok()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Creates a client without a GHI factory so request behavior can be tested independently.
	fn client() -> (PipelineManagerClient, kanal::Receiver<PipelineRequest>) {
		let (requests, receiver) = kanal::unbounded();
		(
			PipelineManagerClient {
				shared: Arc::new(PipelineManagerShared {
					entries: RwLock::new(HashMap::new()),
				}),
				requests,
			},
			receiver,
		)
	}

	#[test]
	fn duplicate_requests_enqueue_one_compilation() {
		let (client, requests) = client();
		let key = PipelineKey::new(42);

		let first = client.request_compute(key, |_| unreachable!());
		let second = client.request_compute(key, |_| unreachable!());

		assert_eq!(first, second);
		assert!(matches!(client.get(first), PipelineState::Pending));
		assert!(matches!(requests.try_recv(), Ok(Some(_))));
		assert!(matches!(requests.try_recv(), Ok(None)));
	}

	#[test]
	fn unknown_pipeline_is_not_available() {
		let (client, _requests) = client();

		assert_eq!(client.get(PipelineRef(PipelineKey::new(7))), PipelineState::Failed);
	}
}

/// The `PipelineManager` struct owns compilation result publication for the
/// renderer.
pub(crate) struct PipelineManager {
	shared: Arc<PipelineManagerShared>,
	completions: kanal::Receiver<PipelineCompletion>,
}

impl PipelineManager {
	/// Creates a client and independent servers that may be moved directly onto threads.
	pub(crate) fn new(
		context: &mut ghi::implementation::Context,
		server_count: usize,
	) -> (PipelineManagerClient, Self, Vec<PipelineManagerServer>) {
		use ghi::context::ContextCreate as _;

		let (request_sender, request_receiver) = kanal::unbounded();
		let (completion_sender, completion_receiver) = kanal::unbounded();
		let shared = Arc::new(PipelineManagerShared {
			entries: RwLock::new(HashMap::new()),
		});
		let servers = (0..server_count.max(1))
			.filter_map(|_| {
				context.create_factory().map(|factory| PipelineManagerServer {
					factory,
					requests: request_receiver.clone(),
					completions: completion_sender.clone(),
				})
			})
			.collect();

		(
			PipelineManagerClient {
				shared: shared.clone(),
				requests: request_sender,
			},
			Self {
				shared,
				completions: completion_receiver,
			},
			servers,
		)
	}

	/// Interns all completed work and publishes one stable availability snapshot.
	pub(crate) fn publish(&mut self, frame: &mut ghi::implementation::Frame) {
		while let Ok(Some(completion)) = self.completions.try_recv() {
			let state = match completion.result {
				Ok(DetachedPipeline::Compute(pipeline)) => PipelineState::Ready(frame.intern_compute_pipeline(pipeline)),
				Ok(DetachedPipeline::Raster(pipeline)) => PipelineState::Ready(frame.intern_raster_pipeline(pipeline)),
				Err(reason) => {
					log::error!("Pipeline compilation failed: {reason}");
					PipelineState::Failed
				}
			};
			self.shared.entries.write().insert(completion.key, state);
		}
	}
}

struct PipelineManagerShared {
	entries: RwLock<HashMap<PipelineKey, PipelineState>>,
}

type PipelineCompiler = Box<dyn FnOnce(&mut ghi::implementation::Factory) -> Result<DetachedPipeline, String> + Send + 'static>;

struct PipelineRequest {
	key: PipelineKey,
	compile: PipelineCompiler,
}

struct PipelineCompletion {
	key: PipelineKey,
	result: Result<DetachedPipeline, String>,
}

enum DetachedPipeline {
	Compute(ghi::factory::ComputePipeline),
	Raster(ghi::factory::RasterPipeline),
}

use std::sync::Arc;

use ghi::frame::Frame as _;
use utils::{
	hash::{HashMap, HashMapExt},
	sync::RwLock,
};
