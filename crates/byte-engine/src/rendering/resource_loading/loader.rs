use std::{
	collections::{HashMap, VecDeque},
	future::Future,
	hash::Hash,
	sync::atomic::{AtomicU64, Ordering},
};

static NEXT_RESOURCE_LOADER_ID: AtomicU64 = AtomicU64::new(1);

/// The `RenderResource` trait defines one renderer's loading protocol without defining its storage policy.
///
/// Implement this once for a renderer or for one independently scheduled
/// resource family. The associated values form two boundaries: [`Self::Key`]
/// and [`Self::Request`] travel from the render thread to a preparer, while
/// [`Self::Prepared`] and [`Self::Error`] travel back. After implementing this
/// trait, implement [`ResourcePreparer`] for worker-side preparation and
/// [`super::ResourceUploadStore`] when the result needs transfer recording.
pub trait RenderResource: 'static {
	/// Stable logical identity used to coalesce duplicate scene requests.
	///
	/// Keep this independent from renderer allocation. A key should describe
	/// what is requested, not the eventual buffer offset or bindless slot.
	type Key: Clone + Eq + Hash + Send + 'static;
	/// Owned input moved from the render thread to one asynchronous preparer lane.
	///
	/// Store everything the preparer needs because it cannot borrow scene or
	/// renderer state while the request is in flight.
	type Request: Clone + Send + 'static;
	/// Renderer-specific value returned for render-thread adoption or upload.
	///
	/// This may contain staging leases, validated metadata, and detached factory
	/// objects. It should not contain renderer-assigned resident identities.
	type Prepared: Send + 'static;
	/// Preparation failure reported at the renderer's frame boundary.
	type Error: Send + 'static;
}

/// The `ResourceRef` struct provides stable logical identity within one renderer loader.
///
/// Store this in pending scene state to follow a resource across preparation,
/// cancellation, and retry. It is deliberately not a GPU handle or renderer
/// slot. Use [`ResourceLoader::key`] to recover the logical key and
/// [`ResourceLoader::token`] when starting revision-specific work.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResourceRef {
	registry_id: u64,
	index: u32,
}

impl ResourceRef {
	fn new(registry_id: u64, index: usize) -> Self {
		Self {
			registry_id,
			index: u32::try_from(index).expect("Render resource registry exhausted its 32-bit index space."),
		}
	}

	/// Returns the stable registry index for compact renderer-side lookup tables.
	///
	/// The index is meaningful only to the loader that issued this reference. It
	/// must not be used as a GPU buffer offset, texture slot, or resident handle.
	pub fn index(self) -> usize {
		self.index as usize
	}
}

/// The `ResourceToken` struct prevents stale work from publishing over a newer request revision.
///
/// Pass the token beside prepared, upload, or native-I/O work. Before changing
/// renderer storage, validate it through [`ResourceLoader::mark_uploading`].
/// The stable [`Self::reference`] remains the same when
/// [`ResourceLoader::retry`] creates a newer token.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ResourceToken {
	reference: ResourceRef,
	revision: u64,
}

impl ResourceToken {
	/// Returns the stable registry slot shared by every revision.
	pub fn reference(self) -> ResourceRef {
		self.reference
	}

	/// Returns the revision used to reject late preparation or upload results.
	pub fn revision(self) -> u64 {
		self.revision
	}
}

/// The `ResourceState` enum defines which subsystem owns one current resource revision.
///
/// The render-thread client owns [`Self::Queued`], a server lane owns
/// [`Self::Loading`], and renderer storage or native GPU I/O owns
/// [`Self::Uploading`]. [`Self::Ready`], [`Self::Failed`], and
/// [`Self::Cancelled`] are terminal until [`ResourceLoader::retry`] creates a
/// new revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResourceState {
	/// The request is waiting for bounded client submission capacity.
	Queued,
	/// A server lane owns or is waiting to receive the preparation work.
	Loading,
	/// Renderer storage or native GPU I/O owns the prepared work.
	Uploading,
	/// The renderer has published the resident resource.
	Ready,
	/// Preparation, storage adoption, or GPU I/O failed.
	Failed,
	/// Preparation was invalidated before renderer storage claimed it.
	Cancelled,
}

struct ResourceEntry<R: RenderResource> {
	key: R::Key,
	request: R::Request,
	revision: u64,
	state: ResourceState,
}

struct ResourceRequest<R: RenderResource> {
	token: ResourceToken,
	request: R::Request,
}

/// The `ResourceCompletion` struct returns one current preparation result to the renderer thread.
///
/// Consume completions at a frame boundary. On success, either publish an
/// immediately usable value with [`ResourceLoader::mark_ready`] or enqueue GPU
/// work. Preparation failures are already marked [`ResourceState::Failed`] by
/// [`ResourceLoader::take_completion`].
pub struct ResourceCompletion<R: RenderResource> {
	token: ResourceToken,
	result: Result<R::Prepared, R::Error>,
}

impl<R: RenderResource> ResourceCompletion<R> {
	/// Returns the exact request revision associated with this result.
	pub fn token(&self) -> ResourceToken {
		self.token
	}

	/// Moves the renderer-specific prepared value or error out of the completion.
	pub fn into_result(self) -> Result<R::Prepared, R::Error> {
		self.result
	}
}

/// The `ResourceLoader` struct keeps render-thread request identity and lifecycle changes nonblocking.
///
/// This is a client and registry, not renderer storage. Keep it beside the
/// renderer's resident map and [`super::FrameUploadQueue`]. Request from scene
/// adoption, drain completions in
/// [`crate::rendering::PipelineManager::begin_frame`], and record queued GPU
/// work in [`crate::rendering::PipelineManager::record_frame_uploads`].
pub struct ResourceLoader<R: RenderResource> {
	entries: Vec<ResourceEntry<R>>,
	by_key: HashMap<R::Key, ResourceRef>,
	queued: VecDeque<ResourceRequest<R>>,
	requests: kanal::Sender<ResourceRequest<R>>,
	completions: kanal::Receiver<ResourceCompletion<R>>,
	max_resources: usize,
	registry_id: u64,
}

impl<R: RenderResource> ResourceLoader<R> {
	/// Creates a loader with equal bounded request and completion capacities.
	///
	/// Use the returned loader on the render thread. Convert the endpoint with
	/// [`ResourceLoadingEndpoint::server`] and run the server on an
	/// application-owned async task. Use [`Self::with_capacity`] when preparation
	/// bursts and render-thread adoption need different bounds.
	pub fn new(max_resources: usize, queue_capacity: usize) -> (Self, ResourceLoadingEndpoint<R>) {
		Self::with_capacity(max_resources, queue_capacity, queue_capacity)
	}

	/// Creates a loader with independent request and completion backpressure bounds.
	///
	/// The render thread retains unsent requests locally when the request channel
	/// is full. Completion backpressure waits only in server tasks. This keeps
	/// frame work nonblocking while bounding cross-thread memory. Next, attach at
	/// least one server with [`ResourceLoadingEndpoint::server`].
	pub fn with_capacity(
		max_resources: usize,
		request_capacity: usize,
		completion_capacity: usize,
	) -> (Self, ResourceLoadingEndpoint<R>) {
		assert!(request_capacity != 0, "Render resource request capacity must be non-zero.");
		assert!(
			completion_capacity != 0,
			"Render resource completion capacity must be non-zero."
		);
		let registry_id = NEXT_RESOURCE_LOADER_ID
			.try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
			.expect(
				"Render resource loader identities are exhausted. The most likely cause is an impractically large number of loader constructions.",
			);
		let (requests, request_receiver) = kanal::bounded_async(request_capacity);
		let (completion_sender, completions) = kanal::bounded_async(completion_capacity);
		(
			Self {
				entries: Vec::new(),
				by_key: HashMap::new(),
				queued: VecDeque::new(),
				requests: requests.to_sync(),
				completions: completions.to_sync(),
				max_resources,
				registry_id,
			},
			ResourceLoadingEndpoint {
				requests: request_receiver,
				completions: completion_sender,
			},
		)
	}

	/// Coalesces one logical request and queues new work without touching a channel.
	///
	/// An existing key returns its original reference and ignores the new request
	/// value, regardless of current state. Call [`Self::retry`] explicitly for a
	/// failed or cancelled resource. On capacity failure, ownership of `request`
	/// is returned so the renderer can report or retain it.
	pub fn request(&mut self, key: R::Key, request: R::Request) -> Result<ResourceRef, R::Request> {
		if let Some(reference) = self.by_key.get(&key) {
			return Ok(*reference);
		}
		if self.entries.len() >= self.max_resources || self.entries.len() > u32::MAX as usize {
			return Err(request);
		}
		let reference = ResourceRef::new(self.registry_id, self.entries.len());
		let token = ResourceToken { reference, revision: 0 };
		let queued_request = request.clone();
		self.entries.push(ResourceEntry {
			key: key.clone(),
			request,
			revision: 0,
			state: ResourceState::Queued,
		});
		self.by_key.insert(key, reference);
		self.queued.push_back(ResourceRequest {
			token,
			request: queued_request,
		});
		Ok(reference)
	}

	/// Examines up to `max` queued entries and submits current work without blocking.
	///
	/// The budget counts examined entries, including stale entries skipped after
	/// cancellation. Call this from bounded frame work; a full channel leaves the
	/// first unsent request queued for a later call.
	pub fn submit_requests(&mut self, max: usize) -> usize {
		let mut submitted = 0;
		let mut examined = 0;
		while examined < max {
			let Some(queued_request) = self.queued.pop_front() else {
				break;
			};
			examined += 1;
			let token = queued_request.token;
			let Some(entry) = self.entry(token.reference) else {
				continue;
			};
			if entry.revision != token.revision || entry.state != ResourceState::Queued {
				continue;
			}
			let mut request = Some(queued_request);
			match self.requests.try_send_option(&mut request) {
				Ok(true) => {
					self.entries[token.reference.index()].state = ResourceState::Loading;
					submitted += 1;
				}
				Ok(false) => {
					self.queued
						.push_front(request.expect("A full request channel must return ownership to the client."));
					break;
				}
				Err(_) => {
					self.entries[token.reference.index()].state = ResourceState::Failed;
				}
			}
		}
		submitted
	}

	/// Returns the next current completion and discards cancelled or superseded work.
	///
	/// This method never waits. It marks an error result failed before returning
	/// it. A successful result remains loading so the renderer can choose
	/// [`Self::mark_ready`] for immediate adoption or let
	/// [`super::FrameUploadQueue::record_frame`] claim it as uploading.
	pub fn take_completion(&mut self) -> Option<ResourceCompletion<R>> {
		loop {
			let completion = self.completions.try_recv().ok().flatten()?;
			if self.is_current(completion.token) && self.state(completion.token.reference) == ResourceState::Loading {
				if completion.result.is_err() {
					self.mark_failed(completion.token);
				}
				return Some(completion);
			}
		}
	}

	/// Finds a previously registered logical resource for scene-side coalescing.
	pub fn reference(&self, key: &R::Key) -> Option<ResourceRef> {
		self.by_key.get(key).copied()
	}

	/// Returns the logical key used to publish a completion into renderer maps.
	pub fn key(&self, reference: ResourceRef) -> Option<&R::Key> {
		self.entry(reference).map(|entry| &entry.key)
	}

	/// Returns the slot's current revision token for renderer-side asynchronous work.
	pub fn token(&self, reference: ResourceRef) -> Option<ResourceToken> {
		self.entry(reference).map(|entry| ResourceToken {
			reference,
			revision: entry.revision,
		})
	}

	/// Returns the slot's current renderer-visible lifecycle state.
	///
	/// A reference issued by another loader reports [`ResourceState::Failed`]
	/// because it cannot name usable state in this registry.
	pub fn state(&self, reference: ResourceRef) -> ResourceState {
		self.entry(reference).map_or(ResourceState::Failed, |entry| entry.state)
	}

	/// Queues a fresh revision after a failure or cancellation.
	///
	/// The loader reuses its retained canonical request and stable reference, but
	/// increments the token revision. Any older completion then becomes stale.
	/// Returns `None` when the reference is foreign or its state is not retryable.
	pub fn retry(&mut self, reference: ResourceRef) -> Option<ResourceToken> {
		let (token, request) = {
			let entry = self.entry_mut(reference)?;
			if !matches!(entry.state, ResourceState::Failed | ResourceState::Cancelled) {
				return None;
			}
			entry.revision = entry.revision.checked_add(1).expect(
				"Render resource revision is exhausted. The most likely cause is an impractically large number of retries.",
			);
			entry.state = ResourceState::Queued;
			(
				ResourceToken {
					reference,
					revision: entry.revision,
				},
				entry.request.clone(),
			)
		};
		self.queued.push_back(ResourceRequest { token, request });
		Some(token)
	}

	/// Cancels queued or loading work so any preparation completion already in flight becomes stale.
	///
	/// Uploading and ready resources stay renderer-owned because the shared
	/// lifecycle has no authority to reclaim an implementation's storage.
	pub fn cancel(&mut self, reference: ResourceRef) -> bool {
		let Some(entry) = self.entry_mut(reference) else {
			return false;
		};
		if !matches!(entry.state, ResourceState::Queued | ResourceState::Loading) {
			return false;
		}
		entry.revision = entry.revision.checked_add(1).expect(
			"Render resource revision is exhausted. The most likely cause is an impractically large number of cancellations.",
		);
		entry.state = ResourceState::Cancelled;
		true
	}

	/// Claims current loading work before renderer storage or native GPU I/O changes.
	///
	/// Call this before the first irreversible renderer-specific action. After it
	/// succeeds, cancellation is intentionally unavailable because only the
	/// renderer knows how to reclaim partially assigned storage.
	pub fn mark_uploading(&mut self, token: ResourceToken) -> bool {
		self.transition(token, |state| state == ResourceState::Loading, ResourceState::Uploading)
	}

	/// Publishes current loading or uploading work after its resident state is usable.
	///
	/// For uploads, prefer [`super::FrameUploadQueue::retire_frame`] so readiness
	/// cannot precede GPU completion. Direct use is appropriate for CPU-only
	/// adoption, interned objects needing no transfer, or completed native I/O.
	pub fn mark_ready(&mut self, token: ResourceToken) -> bool {
		self.transition(
			token,
			|state| matches!(state, ResourceState::Loading | ResourceState::Uploading),
			ResourceState::Ready,
		)
	}

	/// Marks current loading or uploading work as failed after adoption cannot finish.
	///
	/// The renderer must first release or quarantine any storage it already
	/// assigned. Call [`Self::retry`] later when recovery is appropriate.
	pub fn mark_failed(&mut self, token: ResourceToken) -> bool {
		self.transition(
			token,
			|state| matches!(state, ResourceState::Loading | ResourceState::Uploading),
			ResourceState::Failed,
		)
	}

	fn transition(&mut self, token: ResourceToken, accepts: impl FnOnce(ResourceState) -> bool, to: ResourceState) -> bool {
		if !self.is_current(token) {
			return false;
		}
		let state = &mut self.entries[token.reference.index()].state;
		if !accepts(*state) {
			return false;
		}
		*state = to;
		true
	}

	/// Returns whether a completion, upload, or native callback belongs to the current revision.
	pub fn is_current(&self, token: ResourceToken) -> bool {
		self.entry(token.reference)
			.is_some_and(|entry| entry.revision == token.revision)
	}

	fn entry(&self, reference: ResourceRef) -> Option<&ResourceEntry<R>> {
		(reference.registry_id == self.registry_id)
			.then(|| self.entries.get(reference.index()))
			.flatten()
	}

	fn entry_mut(&mut self, reference: ResourceRef) -> Option<&mut ResourceEntry<R>> {
		(reference.registry_id == self.registry_id)
			.then(|| self.entries.get_mut(reference.index()))
			.flatten()
	}
}

/// The `ResourceLoadingEndpoint` struct lets independent server lanes consume one loader's work.
///
/// Clone this value to add parallel preparation lanes. Receivers compete for
/// requests rather than broadcasting them, and every server sends results to
/// the same completion queue. Give each server its own [`ResourcePreparer`] so
/// thread-local factories and conversion state do not need locks.
pub struct ResourceLoadingEndpoint<R: RenderResource> {
	requests: kanal::AsyncReceiver<ResourceRequest<R>>,
	completions: kanal::AsyncSender<ResourceCompletion<R>>,
}

impl<R: RenderResource> Clone for ResourceLoadingEndpoint<R> {
	fn clone(&self) -> Self {
		Self {
			requests: self.requests.clone(),
			completions: self.completions.clone(),
		}
	}
}

impl<R: RenderResource> ResourceLoadingEndpoint<R> {
	/// Attaches one renderer preparer without spawning its application-owned task.
	///
	/// Move the returned server into the application's task system and call
	/// [`ResourceLoadingServer::run`].
	pub fn server<P: ResourcePreparer<R>>(self, preparer: P) -> ResourceLoadingServer<R, P> {
		ResourceLoadingServer {
			endpoint: self,
			preparer,
		}
	}
}

/// The `ResourcePreparer` trait defines worker-side I/O and conversion for one renderer protocol.
///
/// A preparer owns lane-local services such as a resource-manager handle,
/// staging arena, decoder, or detached GHI factory. It must not borrow or
/// mutate the renderer's resident storage. Return enough metadata for the
/// render thread to make placement decisions through
/// [`super::ResourceUploadStore`].
pub trait ResourcePreparer<R: RenderResource> {
	/// Resolves and converts one request without accessing renderer-thread storage.
	///
	/// The server calls this sequentially for each lane, so the implementation
	/// may reuse mutable scratch or factory state without internal synchronization.
	fn prepare(&mut self, request: R::Request) -> impl Future<Output = Result<R::Prepared, R::Error>> + '_;
}

/// The `ResourceLoadingServer` struct provides one sequential worker lane for a renderer preparer.
///
/// The application owns task placement and shutdown. Run [`Self::run`] on an
/// async executor; add throughput by creating more servers from cloned
/// [`ResourceLoadingEndpoint`] values instead of sharing one preparer.
pub struct ResourceLoadingServer<R: RenderResource, P> {
	endpoint: ResourceLoadingEndpoint<R>,
	preparer: P,
}

impl<R: RenderResource, P: ResourcePreparer<R>> ResourceLoadingServer<R, P> {
	/// Prepares requests sequentially and applies completion backpressure only on this async lane.
	///
	/// The loop stops when the render-side loader is dropped or the endpoint is
	/// otherwise closed. In-flight preparation is allowed to finish before its
	/// completion send observes shutdown.
	pub async fn run(mut self) {
		while let Ok(request) = self.endpoint.requests.recv().await {
			let result = self.preparer.prepare(request.request).await;
			if self
				.endpoint
				.completions
				.send(ResourceCompletion {
					token: request.token,
					result,
				})
				.await
				.is_err()
			{
				break;
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use std::sync::{
		Arc,
		atomic::{AtomicUsize, Ordering as AtomicOrdering},
	};

	use super::*;

	struct TestResource;

	impl RenderResource for TestResource {
		type Key = &'static str;
		type Request = u32;
		type Prepared = u32;
		type Error = &'static str;
	}

	#[test]
	fn failed_request_can_retry_with_a_new_revision() {
		let (mut loader, endpoint) = ResourceLoader::<TestResource>::new(8, 2);
		let reference = loader.request("retry", 4).unwrap();
		assert_eq!(loader.submit_requests(1), 1);
		let executor = resource_management::r#async::Executor::new().expect("resource loader test executor");
		let request = executor.block_on(endpoint.requests.recv()).expect("initial request");
		executor
			.block_on(endpoint.completions.send(ResourceCompletion {
				token: request.token,
				result: Err("failed"),
			}))
			.expect("failure completion");

		let completion = loader.take_completion().expect("reported failure");
		assert!(completion.into_result().is_err());
		assert_eq!(loader.state(reference), ResourceState::Failed);
		let retried = loader.retry(reference).expect("failed requests may retry");
		assert!(retried.revision() > request.token.revision());
		assert_eq!(loader.submit_requests(1), 1);
		drop(loader);
		executor.block_on(endpoint.server(UnusedPreparer).run());
	}

	#[test]
	fn cancellation_rejects_an_in_flight_completion() {
		let (mut loader, endpoint) = ResourceLoader::<TestResource>::new(8, 2);
		let reference = loader.request("cancel", 5).unwrap();
		loader.submit_requests(1);
		let executor = resource_management::r#async::Executor::new().expect("resource loader test executor");
		let request = executor.block_on(endpoint.requests.recv()).expect("in-flight request");
		assert!(loader.cancel(reference));
		executor
			.block_on(endpoint.completions.send(ResourceCompletion {
				token: request.token,
				result: Ok(9),
			}))
			.expect("late completion");

		assert!(loader.take_completion().is_none());
		assert_eq!(loader.state(reference), ResourceState::Cancelled);

		let (mut second, _endpoint) = ResourceLoader::<TestResource>::new(8, 1);
		let second_reference = second.request("other loader", 2).unwrap();
		assert_eq!(second.submit_requests(1), 1);
		assert_ne!(reference, second_reference);
		assert!(!second.is_current(request.token));
		assert!(!second.mark_ready(request.token));
		assert_eq!(second.state(second_reference), ResourceState::Loading);
	}

	struct UnusedPreparer;

	impl ResourcePreparer<TestResource> for UnusedPreparer {
		fn prepare(&mut self, request: u32) -> impl Future<Output = Result<u32, &'static str>> + '_ {
			std::future::ready(Ok(request))
		}
	}

	struct CountedRequest {
		id: u32,
		clones: Arc<AtomicUsize>,
	}

	impl Clone for CountedRequest {
		fn clone(&self) -> Self {
			self.clones.fetch_add(1, AtomicOrdering::Relaxed);
			Self {
				id: self.id,
				clones: Arc::clone(&self.clones),
			}
		}
	}

	struct CountedResource;

	impl RenderResource for CountedResource {
		type Key = &'static str;
		type Request = CountedRequest;
		type Prepared = ();
		type Error = ();
	}

	#[test]
	fn duplicate_and_full_channel_requests_coalesce_without_recloning_owned_work() {
		let clones = Arc::new(AtomicUsize::new(0));
		let (mut loader, endpoint) = ResourceLoader::<CountedResource>::new(8, 1);
		let request = |id| CountedRequest {
			id,
			clones: Arc::clone(&clones),
		};
		let first = loader
			.request("first", request(1))
			.unwrap_or_else(|_| panic!("first capacity"));
		let duplicate = loader
			.request("first", request(2))
			.unwrap_or_else(|_| panic!("duplicate capacity"));
		let second = loader
			.request("second", request(3))
			.unwrap_or_else(|_| panic!("second capacity"));
		assert_eq!(first, duplicate);
		assert_ne!(first, second);
		assert_eq!(clones.load(AtomicOrdering::Relaxed), 2);
		assert_eq!(loader.submit_requests(8), 1);
		for _ in 0..8 {
			assert_eq!(loader.submit_requests(8), 0);
		}
		assert_eq!(clones.load(AtomicOrdering::Relaxed), 2);

		let executor = resource_management::r#async::Executor::new().expect("resource loader test executor");
		assert_eq!(executor.block_on(endpoint.requests.recv()).unwrap().request.id, 1);
		assert_eq!(loader.submit_requests(8), 1);
		assert_eq!(executor.block_on(endpoint.requests.recv()).unwrap().request.id, 3);
	}
}
