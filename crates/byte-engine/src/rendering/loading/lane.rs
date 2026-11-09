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

/// The `Priority` enum ranks a request, so lanes start the loads that keep something from drawing before the loads
/// that only refine it.
///
/// [`LoadPipeline::priority`] ranks every request, including the dependencies a lane requests. See [`spawn`] for how
/// lanes split between the two ranks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Priority {
	/// Something can't draw until this request loads.
	Needed,
	/// This request improves something that already draws.
	Background,
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
///
/// Next, pass the implementation to [`spawn`] with the lane split its [`Priority`] ranks need.
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

	/// Ranks one request, so a request that something can't draw without never waits behind one that only refines.
	///
	/// Every request is [`Priority::Needed`] unless the pipeline overrides this. Return [`Priority::Background`] only
	/// for a request that improves something that already draws, such as a finer detail level or a full mip chain.
	fn priority(_request: &Self::Request) -> Priority {
		Priority::Needed
	}

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

/// The `Submission` struct carries one request through the queues that the client and every lane send to.
pub(super) struct Submission<P: LoadPipeline + ?Sized> {
	pub(super) key: P::Key,
	pub(super) request: P::Request,
	/// Loads the resource again even when its key is already loading or resident.
	pub(super) reload: bool,
}

/// The `Queues` struct is the sending side of one pipeline's request queues, so the client and every lane submit
/// through the same path.
///
/// [`spawn`] creates it with its [`Inbox`]. The [`LoaderClient`] submits the render thread's requests through it, and
/// [`LoaderLane::request`] submits dependencies.
pub(super) struct Queues<P: LoadPipeline + ?Sized> {
	needed: kanal::AsyncSender<Submission<P>>,
	background: kanal::AsyncSender<Submission<P>>,
	/// Gets one token for each submission of either priority, so a general lane that sleeps on it checks both queues
	/// again.
	doorbell: kanal::AsyncSender<()>,
}

/// The `Inbox` struct is the receiving side of one pipeline's request queues, which [`spawn`] hands out to lanes.
pub(super) struct Inbox<P: LoadPipeline + ?Sized> {
	pub(super) needed: kanal::AsyncReceiver<Submission<P>>,
	pub(super) background: kanal::AsyncReceiver<Submission<P>>,
	pub(super) doorbell: kanal::AsyncReceiver<()>,
}

impl<P: LoadPipeline> Queues<P> {
	/// Creates one pipeline's empty, unbounded request queues.
	pub(super) fn new() -> (Self, Inbox<P>) {
		let (needed, needed_receiver) = kanal::unbounded_async();
		let (background, background_receiver) = kanal::unbounded_async();
		let (doorbell, doorbell_receiver) = kanal::unbounded_async();
		(
			Self {
				needed,
				background,
				doorbell,
			},
			Inbox {
				needed: needed_receiver,
				background: background_receiver,
				doorbell: doorbell_receiver,
			},
		)
	}

	/// Sends `submission` to the queue of its [`Priority`], then rings the doorbell.
	///
	/// The token goes in after the submission, so a general lane that found both queues empty before the submission
	/// arrived always wakes after it can see it. A closed queue means the client is gone, so nothing would adopt the
	/// result.
	pub(super) fn submit(&self, submission: Submission<P>) {
		let queue = match P::priority(&submission.request) {
			Priority::Needed => &self.needed,
			Priority::Background => &self.background,
		};
		if matches!(queue.as_sync().try_send(submission), Ok(true)) {
			let _ = self.doorbell.as_sync().try_send(());
		}
	}

	/// Closes every queue, so idle lanes stop even though each lane still holds senders for its dependencies.
	///
	/// Requests still queued are dropped, because nothing would adopt their results.
	pub(super) fn close(&self) {
		let _ = self.needed.close();
		let _ = self.background.close();
		let _ = self.doorbell.close();
	}
}

impl<P: LoadPipeline + ?Sized> Clone for Queues<P> {
	fn clone(&self) -> Self {
		Self {
			needed: self.needed.clone(),
			background: self.background.clone(),
			doorbell: self.doorbell.clone(),
		}
	}
}

/// The `LoaderLane` struct is one sequential worker that prepares uploads for the [`Loader`].
///
/// Run each lane on its own async task on the loading thread. Lanes compete for the same request queues, so lane
/// count is how many resources read and decode at once. A needed lane takes only [`Priority::Needed`] requests. A
/// general lane takes those first and [`Priority::Background`] requests when no needed one waits; see [`spawn`]. The
/// GPU work of every lane goes through the one loader.
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
	/// Feeds dependencies into the queues the client sends to, so one registry coalesces both.
	dependencies: Queues<P>,
	needed: kanal::AsyncReceiver<Submission<P>>,
	/// The background queue and the doorbell, which only general lanes hold. A lane without them is a needed lane.
	///
	/// Only general lanes can take background requests, so their count bounds how many background loads run at once.
	general: Option<(kanal::AsyncReceiver<Submission<P>>, kanal::AsyncReceiver<()>)>,
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
	/// it like any other result. It queues by its own [`LoadPipeline::priority`], not by the priority of the load
	/// that requests it. Requests for a key already loading or resident are ignored.
	pub fn request(&self, request: P::Request) {
		self.dependencies.submit(Submission {
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
	///
	/// A needed lane waits on the needed queue only. A general lane takes a needed request first and a background
	/// one otherwise, and sleeps on the doorbell only after it found both queues empty.
	pub async fn run(mut self) {
		let Some((background, doorbell)) = self.general.take() else {
			while let Ok(submission) = self.needed.recv().await {
				if !self.serve(submission).await {
					break;
				}
			}
			return;
		};
		loop {
			let next = self
				.needed
				.as_sync()
				.try_recv()
				.ok()
				.flatten()
				.or_else(|| background.as_sync().try_recv().ok().flatten());
			match next {
				Some(submission) => {
					if !self.serve(submission).await {
						break;
					}
				}
				// A submission that arrived after both checks rang the doorbell after it was queued, so this wait
				// can't miss it. A token can also belong to a request another lane took, which only costs a recheck.
				None => {
					if doorbell.recv().await.is_err() {
						break;
					}
				}
			}
		}
	}

	/// Loads one submission, unless its key is already loading or resident, and sends the result to the client.
	///
	/// Returns `false` once the client is gone, so the lane stops.
	async fn serve(&mut self, Submission { key, request, reload }: Submission<P>) -> bool {
		// The client and every lane send to the same queues, so coalesce here. A reload still records its key.
		if !self
			.registry
			.lock()
			.unwrap_or_else(|error| error.into_inner())
			.insert(key.clone())
			&& !reload
		{
			return true;
		}
		let pipeline = self.pipeline.clone();
		self.busy_lanes.fetch_add(1, Ordering::AcqRel);
		let result = pipeline.load(request, self).await;
		self.busy_lanes.fetch_sub(1, Ordering::AcqRel);
		if result.is_err() {
			self.registry.lock().unwrap_or_else(|error| error.into_inner()).remove(&key);
		}
		self.results.send((key, result)).await.is_ok()
	}
}

/// Creates the single client and lane pool for one rendering pipeline.
///
/// `lane_count` lanes load at once. The last `background_lane_count` of them are general lanes, which take
/// [`Priority::Needed`] requests first and [`Priority::Background`] requests when no needed one waits. The others are
/// needed lanes, which take needed requests only. So a needed request never queues behind a background one and can
/// use every lane that isn't already loading background work, while at most `background_lane_count` background loads
/// run at once. With one lane, that lane is a general lane. A pipeline that ranks every request needed can pass 1,
/// because every lane then takes every request. `queue_capacity` bounds the results that wait for [`LoaderClient::poll`]; while it is full, lanes
/// wait to send theirs.
///
/// Keep the client on the render thread and move every lane to a task on the loading thread, where
/// [`Loader::run`] runs too.
///
/// # Panics
///
/// Panics when `lane_count` is zero, or when `background_lane_count` leaves no general lane or, with more than one
/// lane, no needed lane.
pub fn spawn<P: LoadPipeline>(
	loader: &Loader,
	pipeline: P,
	lane_count: usize,
	background_lane_count: usize,
	queue_capacity: usize,
) -> (LoaderClient<P>, Vec<LoaderLane<P>>) {
	spawn_lanes(
		&loader.batcher.staging,
		&loader.uploads,
		&loader.batcher.busy_lanes,
		pipeline,
		lane_count,
		background_lane_count,
		queue_capacity,
	)
}

/// Creates the client and lanes of [`spawn`] around the loader's staging arena, upload stream, and busy-lane count.
///
/// Checks the lane split, creates the request queues and the result queue, and gives the last
/// `background_lane_count` lanes the background queue and the doorbell.
fn spawn_lanes<P: LoadPipeline>(
	staging: &Arc<UploadStagingArena>,
	uploads: &kanal::AsyncSender<UploadRequest>,
	busy_lanes: &Arc<AtomicUsize>,
	pipeline: P,
	lane_count: usize,
	background_lane_count: usize,
	queue_capacity: usize,
) -> (LoaderClient<P>, Vec<LoaderLane<P>>) {
	assert!(
		lane_count > 0,
		"A loader lane pool was created without lanes. The most likely cause is a lane count of zero. Pass a `lane_count` of at least 1 to `spawn`."
	);
	// One lane must be general so background requests load. A single lane is general, otherwise one must be a needed lane.
	let most_background_lanes = (lane_count - 1).max(1);
	assert!(
		(1..=most_background_lanes).contains(&background_lane_count),
		"A loader lane pool of {lane_count} lanes was given {background_lane_count} background lanes. The most likely cause is a split that leaves no lane for background requests or none for needed ones only. Pass a `background_lane_count` from 1 to {most_background_lanes} to `spawn`."
	);
	let needed_lane_count = lane_count - background_lane_count;

	let pipeline = Arc::new(pipeline);
	let registry = Arc::new(Mutex::new(HashSet::new()));
	let (queues, inbox) = Queues::<P>::new();
	let (result_sender, result_receiver) = kanal::bounded_async(queue_capacity);

	let lanes = (0..lane_count)
		.map(|index| LoaderLane {
			pipeline: pipeline.clone(),
			staging: staging.clone(),
			uploads: uploads.clone(),
			busy_lanes: busy_lanes.clone(),
			registry: registry.clone(),
			dependencies: queues.clone(),
			needed: inbox.needed.clone(),
			general: (index >= needed_lane_count).then(|| (inbox.background.clone(), inbox.doorbell.clone())),
			results: result_sender.clone(),
		})
		.collect();

	(LoaderClient::new(queues, result_receiver), lanes)
}

#[cfg(test)]
mod tests {
	use std::{cell::Cell, collections::HashMap, rc::Rc};

	use super::*;
	use crate::{core::async_runtime::yield_now, rendering::loading::Event};

	/// How many runtime turns the helpers give the lanes. Every test step needs only a few.
	const TURNS: usize = 256;

	/// The `Step` enum is one thing a [`GatedPipeline`] load did, in the order the lanes did it.
	#[derive(Clone, Copy, Debug, PartialEq, Eq)]
	enum Step {
		Start(u32),
		Finish(u32),
	}

	/// The `Job` struct is one test request, which keeps loading until the test opens its gate.
	struct Job {
		id: u32,
		priority: Priority,
		gate: kanal::AsyncReceiver<()>,
		/// Requested through the lane as soon as the load starts.
		dependency: Option<Box<Job>>,
	}

	/// The `GatedPipeline` struct lets a test hold loads open, so it can see which requests run at once.
	#[derive(Default)]
	struct GatedPipeline {
		log: Arc<Mutex<Vec<Step>>>,
	}

	impl LoadPipeline for GatedPipeline {
		type Key = u32;
		type Request = Job;
		type Resident = u32;

		fn key(job: &Job) -> u32 {
			job.id
		}

		fn priority(job: &Job) -> Priority {
			job.priority
		}

		async fn load(&self, job: Job, lane: &mut LoaderLane<Self>) -> Result<u32, LoadError> {
			self.log.lock().unwrap().push(Step::Start(job.id));
			if let Some(dependency) = job.dependency {
				lane.request(*dependency);
			}
			// A closed gate releases the load too, so a finished test leaves no lane waiting.
			let _ = job.gate.recv().await;
			self.log.lock().unwrap().push(Step::Finish(job.id));
			Ok(job.id)
		}
	}

	/// Creates a client and lanes around a test arena, without a GPU loader.
	fn spawn_test_lanes(
		pipeline: GatedPipeline,
		lane_count: usize,
		background_lane_count: usize,
	) -> (LoaderClient<GatedPipeline>, Vec<LoaderLane<GatedPipeline>>) {
		// The lanes never stage bytes. Leaking the buffer keeps it alive for as long as any lane holds the arena.
		let (staging, _worker) = UploadStagingArena::new_for_test(Box::leak(Box::new([0u8; 64])));
		let (uploads, _) = kanal::unbounded_async();
		spawn_lanes(
			&staging,
			&uploads,
			&Arc::new(AtomicUsize::new(0)),
			pipeline,
			lane_count,
			background_lane_count,
			64,
		)
	}

	/// Runs one test on a single-threaded runtime, the way the loading thread runs lanes.
	fn run(test: impl Future<Output = ()>) {
		compio::runtime::Runtime::new()
			.expect("The lane test couldn't start its runtime. The most likely cause is that the system denied the I/O driver. Run the test again.")
			.block_on(test);
	}

	/// Gives the lanes turns until `condition` holds, and fails the test if it never does.
	async fn until(state: &str, mut condition: impl FnMut() -> bool) {
		for _ in 0..TURNS {
			if condition() {
				return;
			}
			yield_now().await;
		}
		panic!("The lane pool never reached the expected state: {state}.");
	}

	/// Gives the lanes enough turns to start every load they can, so a test can check what didn't start.
	async fn settle() {
		for _ in 0..TURNS {
			yield_now().await;
		}
	}

	/// The `Pool` struct is a running lane pool under test, with the gates of the jobs it was sent.
	struct Pool {
		client: LoaderClient<GatedPipeline>,
		log: Arc<Mutex<Vec<Step>>>,
		gates: HashMap<u32, kanal::AsyncSender<()>>,
		/// How many lanes returned from [`LoaderLane::run`].
		stopped: Rc<Cell<usize>>,
	}

	impl Pool {
		/// Runs a pool on the current runtime and lets every lane reach its first wait.
		async fn start(lane_count: usize, background_lane_count: usize) -> Self {
			let pipeline = GatedPipeline::default();
			let log = pipeline.log.clone();
			let (client, lanes) = spawn_test_lanes(pipeline, lane_count, background_lane_count);
			let stopped = Rc::new(Cell::new(0));
			for lane in lanes {
				let stopped = stopped.clone();
				compio::runtime::spawn(async move {
					lane.run().await;
					stopped.set(stopped.get() + 1);
				})
				.detach();
			}
			settle().await;
			Self {
				client,
				log,
				gates: HashMap::new(),
				stopped,
			}
		}

		/// Creates job `id`, which loads until [`Self::release`] opens its gate.
		fn job(&mut self, id: u32, priority: Priority) -> Job {
			let (gate, opened) = kanal::unbounded_async();
			self.gates.insert(id, gate);
			Job {
				id,
				priority,
				gate: opened,
				dependency: None,
			}
		}

		/// Requests job `id` through the client.
		fn request(&mut self, id: u32, priority: Priority) {
			let job = self.job(id, priority);
			self.client.request(job);
		}

		/// Requests job `id`, which requests `dependency` through its lane as soon as it starts.
		fn request_with(&mut self, id: u32, priority: Priority, dependency: Job) {
			let mut job = self.job(id, priority);
			job.dependency = Some(Box::new(dependency));
			self.client.request(job);
		}

		/// Lets job `id` finish, now or as soon as it starts.
		fn release(&mut self, id: u32) {
			self.gates
				.remove(&id)
				.expect("The test released a job it never created.")
				.as_sync()
				.send(())
				.unwrap();
		}

		/// Returns the jobs that started and didn't finish, in id order.
		fn running(&self) -> Vec<u32> {
			let mut running = Vec::new();
			for step in self.log.lock().unwrap().iter() {
				match *step {
					Step::Start(id) => running.push(id),
					Step::Finish(id) => running.retain(|running| *running != id),
				}
			}
			running.sort_unstable();
			running
		}

		/// Returns the most jobs that ran at once so far.
		fn peak(&self) -> usize {
			let (mut running, mut peak) = (0, 0);
			for step in self.log.lock().unwrap().iter() {
				match step {
					Step::Start(_) => {
						running += 1;
						peak = usize::max(peak, running);
					}
					Step::Finish(_) => running -= 1,
				}
			}
			peak
		}

		/// Returns the jobs the client reported ready since the previous call, in id order.
		fn ready(&mut self) -> Vec<u32> {
			let mut ready = Vec::new();
			while let Some(event) = self.client.poll() {
				match event {
					Event::Ready { key, .. } => ready.push(key),
					Event::Failed { key, error } => panic!("Job {key} failed: {error}"),
				}
			}
			ready.sort_unstable();
			ready
		}
	}

	#[test]
	fn needed_requests_start_before_queued_background_requests() {
		run(async {
			// One needed lane and one general lane.
			let mut pool = Pool::start(2, 1).await;
			for id in 1..=3 {
				pool.request(id, Priority::Background);
			}
			settle().await;
			assert_eq!(pool.running(), [1], "Only the general lane takes background requests.");

			pool.request(10, Priority::Needed);
			pool.request(11, Priority::Needed);
			settle().await;
			assert_eq!(
				pool.running(),
				[1, 10],
				"A needed request must not wait behind queued background ones."
			);

			pool.release(1);
			settle().await;
			assert_eq!(
				pool.running(),
				[10, 11],
				"A free general lane must take a needed request before a background one."
			);

			pool.release(10);
			pool.release(11);
			settle().await;
			assert_eq!(pool.running(), [2], "Background requests load once no needed request waits.");
		});
	}

	#[test]
	fn background_loads_never_exceed_the_background_lane_count() {
		run(async {
			let mut pool = Pool::start(4, 2).await;
			for id in 1..=6 {
				pool.request(id, Priority::Background);
			}
			for id in 1..=6 {
				settle().await;
				pool.release(id);
			}
			settle().await;

			assert_eq!(pool.ready(), [1, 2, 3, 4, 5, 6]);
			assert_eq!(
				pool.peak(),
				2,
				"Two general lanes must load at most two background requests at once."
			);
		});
	}

	#[test]
	fn needed_bursts_use_every_lane() {
		run(async {
			let mut pool = Pool::start(4, 2).await;
			for id in 1..=6 {
				pool.request(id, Priority::Needed);
			}
			settle().await;
			assert_eq!(
				pool.running(),
				[1, 2, 3, 4],
				"Needed requests must use the general lanes too."
			);

			for id in 1..=6 {
				pool.release(id);
			}
			settle().await;
			assert_eq!(pool.ready(), [1, 2, 3, 4, 5, 6]);
			assert_eq!(pool.peak(), 4);
		});
	}

	#[test]
	fn background_requests_drain_without_needed_work() {
		run(async {
			// A single lane is a general lane.
			let mut pool = Pool::start(1, 1).await;
			for id in 1..=3 {
				pool.request(id, Priority::Background);
				pool.release(id);
			}
			settle().await;

			assert_eq!(pool.ready(), [1, 2, 3]);
		});
	}

	#[test]
	fn dependencies_queue_by_their_own_priority() {
		run(async {
			// Two needed lanes and one general lane.
			let mut pool = Pool::start(3, 1).await;

			let dependency = pool.job(2, Priority::Needed);
			pool.request_with(1, Priority::Background, dependency);
			settle().await;
			assert_eq!(
				pool.running(),
				[1, 2],
				"A needed dependency of a background load must not wait for the general lane that load holds."
			);

			let dependency = pool.job(4, Priority::Background);
			pool.request_with(3, Priority::Needed, dependency);
			settle().await;
			pool.release(2);
			settle().await;
			assert_eq!(
				pool.running(),
				[1, 3],
				"A background dependency of a needed load must wait for a general lane, even while a needed lane is idle."
			);

			pool.release(1);
			settle().await;
			assert_eq!(pool.running(), [3, 4]);
		});
	}

	#[test]
	fn dropping_the_client_stops_needed_and_general_lanes() {
		run(async {
			let Pool { client, stopped, .. } = Pool::start(3, 1).await;
			drop(client);
			until("every lane stopped", || stopped.get() == 3).await;
		});
	}

	#[test]
	#[should_panic(expected = "was given 0 background lanes")]
	fn a_split_without_a_general_lane_panics() {
		spawn_test_lanes(GatedPipeline::default(), 2, 0);
	}

	#[test]
	#[should_panic(expected = "was given 2 background lanes")]
	fn a_split_without_a_needed_lane_panics() {
		spawn_test_lanes(GatedPipeline::default(), 2, 2);
	}
}
