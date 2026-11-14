//! Run thread-confined GHI compute processors for offline asset processing.
//!
//! The environment-map generator ([`crate::ibl::IBLGenerator`]) and the material mip generator
//! ([`crate::resources::mips::gpu::MaterialMipGenerator`]) share this worker thread, device setup, and kernel setup.
//! Asset bakes claim a place at the worker before they produce a request's input, then submit the request from the
//! shared worker pool and await the reply. No pool thread blocks while the GPU works: the worker records the requests
//! that arrived together as one workload and waits for it, while pool threads prepare the next inputs, and work waits
//! for the GPU before it allocates rather than after.

use std::{
	any::Any,
	collections::VecDeque,
	fmt,
	future::Future,
	pin::Pin,
	sync::{
		Arc,
		mpsc::{self, Receiver, Sender},
	},
	task::{Context, Poll, ready},
	thread::{JoinHandle, ThreadId},
};

use ghi::{context::ContextCreate as _, device::Device as _};
use smallvec::SmallVec;
use utils::{r#async::oneshot, sync::Mutex};

/// The slot every offline kernel samples its source image from.
pub(crate) const SOURCE_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);

/// The slot every offline kernel writes its output image or buffer to.
pub(crate) const OUTPUT_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);

/// The results of one batch, which holds at most [`GpuProcessor::MAX_BATCH`] requests.
pub(crate) type BatchResults<R> = SmallVec<[R; 4]>;

/// The `GpuProcessor` trait lets a thread-confined GHI processor serve batches of requests on one [`GpuWorker`].
///
/// Implement it on the type that owns a compute context. The worker calls [`Self::process`] from the thread the
/// processor was created on, so the context never crosses threads, with at most [`Self::MAX_BATCH`] requests at once.
/// Create the worker with [`GpuWorker::spawn`].
pub(crate) trait GpuProcessor {
	/// The parameters of one request, next to its input bytes.
	type Request: Send + 'static;
	/// The value a request resolves to, including any bytes it produced.
	type Result: Send + 'static;

	/// The most requests one batch holds, which bounds the scratch the processor needs.
	const MAX_BATCH: usize;

	/// Uploads every request's input, records their GPU work as one workload with
	/// [`record`](ghi::context::Context::record), and returns one result per request, in batch order, once the GPU
	/// finished it.
	///
	/// A request that needs no GPU work, or that the processor can't serve, gets its result without recording.
	fn process(&mut self, batch: &[(Self::Request, Vec<u8>)]) -> BatchResults<Self::Result>;
}

/// The `GpuWorker` struct lets asset handlers on the shared worker pool use a GHI processor that must stay on one thread.
///
/// Spawn it with the processor factory. For each image, await [`Self::claim`] before producing the input, then call
/// [`Self::submit`] with the claim and await the returned receiver. Requests own their input bytes until they are
/// uploaded, so a caller that stops awaiting never leaves the worker with dangling memory.
pub(crate) struct GpuWorker<P: GpuProcessor> {
	/// `None` once the worker was asked to shut down, or when it never started.
	sender: Option<Sender<Request<P>>>,
	worker: Option<JoinHandle<()>>,
	claims: Arc<Mutex<Claims>>,
}

/// The `GpuClaim` struct holds one of a [`GpuWorker`]'s places, from before a request's input exists until the caller
/// has used its result.
///
/// Get one with [`GpuWorker::claim`] before allocating the input, and keep it until the memory the request needs is
/// released. Work then waits for the GPU while it holds nothing, instead of queueing up with its input allocated.
/// Never await another bake or another claim while holding one: a holder waits only on its own work, such as the GPU,
/// encoding, and storing, so every claim returns in bounded time. Dropping the claim hands the place to a caller still
/// waiting; see [`GpuWorker::claim`] for which one.
pub(crate) struct GpuClaim {
	claims: Arc<Mutex<Claims>>,
	/// The thread the claim's work runs on, which is the thread that asked for it.
	thread: ThreadId,
}

/// The `Claims` struct counts a worker's free places and queues the callers that wait for one, oldest first.
struct Claims {
	free: usize,
	/// The thread of each claim taken, once per claim.
	holders: Vec<ThreadId>,
	waiting: VecDeque<(ThreadId, oneshot::Sender<()>)>,
}

impl Drop for GpuClaim {
	fn drop(&mut self) {
		let mut guard = self.claims.lock();
		let claims = &mut *guard;
		let holder = claims.holders.iter().position(|thread| *thread == self.thread);
		claims
			.holders
			.swap_remove(holder.expect("A claim's thread is recorded until the claim is dropped"));

		// An async task stays on the thread that first polled it, so the CPU work of two claims on one thread takes
		// turns, and every wait of one, for the GPU or for storage, ends only once the other yields. The place therefore
		// goes to the oldest caller on a thread that holds no claim, or to the oldest caller when every waiting thread
		// holds one, so a place never idles while someone waits. A send fails only for a caller that stopped waiting,
		// so the place goes on to the next one.
		//
		// Each waiter is matched by scanning the queue and, per waiter, the holders. `holders` is capped at CLAIMS,
		// two per request a batch holds, and `waiting` holds only the callers blocked on a place, so both lists stay small
		// and a claim returns in bounded time by contract.
		loop {
			let next = claims
				.waiting
				.iter()
				.position(|(thread, _)| !claims.holders.contains(thread))
				.unwrap_or(0);
			let Some((thread, waiter)) = claims.waiting.remove(next) else {
				break;
			};
			if waiter.send(()).is_ok() {
				claims.holders.push(thread);
				return;
			}
		}
		claims.free += 1;
	}
}

/// The `ClaimWait` struct is the future of a claim that found every place taken; it resolves once a dropped claim hands
/// it one.
struct ClaimWait {
	granted: oneshot::Receiver<()>,
	claims: Arc<Mutex<Claims>>,
	thread: ThreadId,
}

impl Future for ClaimWait {
	type Output = GpuClaim;

	fn poll(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<GpuClaim> {
		// The sender sits in the queue that this future's `claims` keeps alive, and it's only taken out to send.
		ready!(Pin::new(&mut self.granted).poll(context)).expect("A waiting claim's sender is only dropped after sending");
		Poll::Ready(GpuClaim {
			claims: Arc::clone(&self.claims),
			thread: self.thread,
		})
	}
}

impl Drop for ClaimWait {
	fn drop(&mut self) {
		// A caller that stops waiting may have been handed a place meanwhile. Closing first makes every later hand-off
		// skip this caller, and a place that already arrived goes on to the next waiter instead of being lost. A place
		// this future already returned was taken out of the channel, so it isn't released twice.
		self.granted.close();
		if let Ok(Some(())) = self.granted.try_recv() {
			drop(GpuClaim {
				claims: Arc::clone(&self.claims),
				thread: self.thread,
			});
		}
	}
}

/// The `GpuWorkerError` enum reports why the shared GPU worker could not set up a processor or serve a request.
///
/// GPU processor errors, such as the environment-map and mip errors, wrap it. Return it from a processor factory to
/// report your own instance, device, or context failure.
#[derive(Debug)]
pub enum GpuWorkerError {
	Instance(&'static str),
	Device(&'static str),
	Context(&'static str),
	ShaderCompilation(String),
	ShaderCreation,
	ThreadCreation(String),
	/// The worker stopped before it started or answered a request.
	Unavailable,
}

impl GpuWorkerError {
	/// Writes the error for the GPU `subsystem`, such as "mip" or "environment-map".
	pub(crate) fn describe(&self, subsystem: &str, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
		match self {
			Self::Instance(error) => write!(
				formatter,
				"GPU {subsystem} instance creation failed. The most likely cause is that no supported graphics backend is available. Error: {error}"
			),
			Self::Device(error) => write!(
				formatter,
				"GPU {subsystem} device creation failed. The most likely cause is that no device supports compute and transfer work. Error: {error}"
			),
			Self::Context(error) => write!(
				formatter,
				"GPU {subsystem} context creation failed. The most likely cause is that the selected device could not create an auxiliary context. Error: {error}"
			),
			Self::ShaderCompilation(error) => write!(
				formatter,
				"GPU {subsystem} shader compilation failed. The most likely cause is unsupported native shader syntax. Error: {error}"
			),
			Self::ShaderCreation => write!(
				formatter,
				"GPU {subsystem} shader creation failed. The most likely cause is that the selected backend rejected the compute shader."
			),
			Self::ThreadCreation(error) => write!(
				formatter,
				"GPU {subsystem} worker creation failed. The most likely cause is that the process cannot create another thread. Error: {error}"
			),
			Self::Unavailable => write!(
				formatter,
				"GPU {subsystem} worker is unavailable. The most likely cause is that GPU initialization or command execution terminated the worker."
			),
		}
	}
}

impl<P: GpuProcessor + 'static> GpuWorker<P> {
	/// How many claims the worker hands out: one batch the GPU runs, and as many requests again preparing their
	/// input, so the next batch is ready when the GPU finishes.
	pub(crate) const CLAIMS: usize = 2 * P::MAX_BATCH;

	/// Runs `initialize` on a new thread named `name` and returns once the processor it creates is ready.
	///
	/// Create every thread-affine GHI device and context inside `initialize`. The processor stays on the worker for its
	/// whole lifetime and serves every request through [`GpuProcessor`].
	pub(crate) fn spawn<E: From<GpuWorkerError> + Send + 'static>(
		name: &str,
		initialize: impl FnOnce() -> Result<P, E> + Send + 'static,
	) -> Result<Self, E> {
		let (sender, receiver) = mpsc::channel();
		let (startup, startup_receiver) = mpsc::sync_channel(1);
		let worker = std::thread::Builder::new()
			.name(name.to_string())
			.spawn(move || match initialize() {
				Ok(processor) => {
					let _ = startup.send(Ok(()));
					serve(processor, receiver);
				}
				Err(error) => {
					let _ = startup.send(Err(error));
				}
			})
			.map_err(|error| GpuWorkerError::ThreadCreation(error.to_string()))?;

		// A worker that stops before it reports is as unavailable as one whose factory failed.
		match startup_receiver
			.recv()
			.unwrap_or_else(|_| Err(GpuWorkerError::Unavailable.into()))
		{
			Ok(()) => Ok(Self {
				sender: Some(sender),
				worker: Some(worker),
				claims: Self::fresh_claims(),
			}),
			Err(error) => {
				let _ = worker.join();
				Err(error)
			}
		}
	}

	fn fresh_claims() -> Arc<Mutex<Claims>> {
		Arc::new(Mutex::new(Claims {
			free: Self::CLAIMS,
			holders: Vec::with_capacity(Self::CLAIMS),
			waiting: VecDeque::new(),
		}))
	}

	/// Waits for one of the worker's [`Self::CLAIMS`] places and returns it.
	///
	/// Call this before producing a request's input, then pass the claim to [`Self::submit`]. The wait suspends the
	/// caller, so pool threads keep running other bakes while every place is taken. A freed place goes to the oldest
	/// caller whose thread holds no claim, and to the oldest caller when every waiting thread holds one; callers on
	/// different threads then run their claims' work side by side.
	pub(crate) async fn claim(&self) -> GpuClaim {
		let thread = std::thread::current().id();
		let granted = {
			let mut claims = self.claims.lock();
			if claims.free > 0 {
				claims.free -= 1;
				claims.holders.push(thread);
				None
			} else {
				let (grant, granted) = oneshot::channel();
				claims.waiting.push_back((thread, grant));
				Some(granted)
			}
		};
		let claims = Arc::clone(&self.claims);
		match granted {
			Some(granted) => ClaimWait { granted, claims, thread }.await,
			None => GpuClaim { claims, thread },
		}
	}

	/// Queues one request under `claim` and returns the receiver that resolves to its result.
	///
	/// The worker batches it with the requests queued next to it and uploads `input`. The receiver reports
	/// cancellation when the worker stopped before serving the request. Dropping it does not cancel the GPU work; the
	/// worker finishes the request and discards the result.
	pub(crate) fn submit(&self, claim: &GpuClaim, parameters: P::Request, input: Vec<u8>) -> oneshot::Receiver<P::Result> {
		// Each claim covers this worker's requests only, which is what bounds its queue.
		debug_assert!(
			Arc::ptr_eq(&claim.claims, &self.claims),
			"A claim must come from the worker it submits to"
		);
		let (reply, receiver) = oneshot::channel();
		if let Some(sender) = &self.sender {
			// A failed send drops the request and its reply sender, which cancels the receiver.
			let _ = sender.send((parameters, input, reply));
		}
		receiver
	}

	/// Creates a worker whose thread already stopped, so every request reports it as unavailable.
	#[cfg(test)]
	pub(crate) fn unavailable() -> Self {
		Self {
			sender: None,
			worker: None,
			claims: Self::fresh_claims(),
		}
	}
}

impl<P: GpuProcessor> Drop for GpuWorker<P> {
	fn drop(&mut self) {
		// Closing the channel asks the worker to finish the requests already queued and stop.
		drop(self.sender.take());
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

/// Carries one request's parameters, input bytes, and reply channel to the worker.
type Request<P> = (
	<P as GpuProcessor>::Request,
	Vec<u8>,
	oneshot::Sender<<P as GpuProcessor>::Result>,
);

/// Serves requests on the worker thread in batches until the channel closes.
///
/// Each batch takes the oldest request and those queued behind it, up to [`GpuProcessor::MAX_BATCH`], so requests are
/// served in arrival order. Requests queued before the channel closed are still served.
fn serve<P: GpuProcessor>(mut processor: P, receiver: Receiver<Request<P>>) {
	let mut batch = Vec::with_capacity(P::MAX_BATCH);
	let mut replies = Vec::with_capacity(P::MAX_BATCH);
	// Parks while idle, and stops once the channel closed and every queued request was taken.
	while let Ok(first) = receiver.recv() {
		for (parameters, input, reply) in std::iter::once(first).chain(receiver.try_iter().take(P::MAX_BATCH - 1)) {
			batch.push((parameters, input));
			replies.push(reply);
		}
		let results = processor.process(&batch);
		debug_assert_eq!(results.len(), batch.len(), "A processor returns one result per request");
		for (reply, result) in replies.drain(..).zip(results) {
			let _ = reply.send(result);
		}
		// The inputs were uploaded, so they're freed before the worker waits for the next batch.
		batch.clear();
	}
}

/// The `OwnedContext` struct keeps a GHI context and the native state it depends on together for a processor's lifetime.
///
/// Processors store it whole. Its field order drops the context before its owner on every path, and `dyn Any` keeps
/// the thread-confined processor that holds it non-`Send`.
pub(crate) struct OwnedContext {
	pub(crate) context: ghi::implementation::Context,
	pub(crate) owner: Box<dyn Any>,
}

/// Creates a self-contained compute and transfer device and context for offline asset processing.
///
/// Returns the context, with the device and instance that keep it alive, and its queue.
pub(crate) fn create_compute_context() -> Result<(OwnedContext, ghi::QueueHandle), GpuWorkerError> {
	let features = ghi::device::Features::new().mesh_shading(false);
	let mut instance = ghi::implementation::Instance::new(features).map_err(GpuWorkerError::Instance)?;
	let mut queue = None;
	let device = instance
		.create_device(
			features,
			&mut [(
				ghi::QueueSelection::new(ghi::WorkloadTypes::COMPUTE | ghi::WorkloadTypes::TRANSFER),
				&mut queue,
			)],
		)
		.map_err(GpuWorkerError::Device)?;
	let context = device.create_context().map_err(GpuWorkerError::Context)?;
	let queue = queue.expect("GHI device creation must populate the requested compute queue handle.");

	let owner = Box::new((device, instance));
	Ok((OwnedContext { context, owner }, queue))
}

/// The `ComputeKernel` struct holds a bake kernel's pipeline with the layout of the one descriptor set it reads, so
/// a processor can create that set for each texture it processes.
///
/// Next, create a set with `create_descriptor_set(.., &kernel.layout)` and bind it after `kernel.pipeline`.
#[derive(Clone)]
pub(crate) struct ComputeKernel {
	pub(crate) pipeline: ghi::PipelineHandle,
	pub(crate) layout: ghi::DescriptorSetLayout,
}

/// Compiles one native compute kernel that samples [`SOURCE_SLOT`] and writes the storage image at [`OUTPUT_SLOT`], and
/// builds its pipeline.
///
/// `label` names the shader and pipeline in tools, and `push_constant_size` is the size of the kernel's push constants.
pub(crate) fn create_compute_kernel(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	source: ghi::shader::ShaderSource<'_>,
	push_constant_size: usize,
) -> Result<ComputeKernel, GpuWorkerError> {
	let compiled = ghi::shader::compile(label, source).map_err(GpuWorkerError::ShaderCompilation)?;
	let resources = [
		ghi::ShaderResourceDescriptor::single(
			SOURCE_SLOT,
			ghi::ResourceKind::CombinedImageSampler,
			ghi::AccessPolicies::READ,
		),
		ghi::ShaderResourceDescriptor::single(OUTPUT_SLOT, ghi::ResourceKind::StorageImage, ghi::AccessPolicies::WRITE),
	];
	build_compute_pipeline(context, label, &compiled, resources, push_constant_size)
}

/// Compiles a BESL compute kernel for the active graphics backend and builds its pipeline.
///
/// `resources` must describe every binding the kernel declares, and `workgroup` is the local size the kernel is
/// compiled with, which dispatches must use too.
pub(crate) fn create_besl_compute_kernel(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	source: &str,
	workgroup: utils::Extent,
	resources: impl IntoIterator<Item = ghi::ShaderResourceDescriptor>,
	push_constant_size: usize,
) -> Result<ComputeKernel, GpuWorkerError> {
	let program = besl::compile_to_besl(source, None).map_err(|error| {
		GpuWorkerError::ShaderCompilation(format!(
			"BESL kernel '{label}' failed to parse or link. The most likely cause is invalid kernel source. Error: {error:?}"
		))
	})?;
	let settings = crate::shader::ShaderGenerationSettings::compute(workgroup).name(label.to_string());
	let compiled = native_kernel_source(label, &settings, &program).map_err(GpuWorkerError::ShaderCompilation)?;
	build_compute_pipeline(context, label, &compiled, resources, push_constant_size)
}

/// Lowers a linked BESL kernel with the platform shader generator into the source the active GHI backend takes.
///
/// Linux compiles the GLSL to SPIR-V here. The Metal and DX12 backends compile their source text when the shader is
/// created.
fn native_kernel_source(
	label: &str,
	settings: &crate::shader::ShaderGenerationSettings,
	program: &besl::NodeReference,
) -> Result<ghi::shader::CompiledShaderSource, String> {
	use crate::shader::besl::backends::platform::{PlatformShaderCompiler, PlatformShaderLanguage};

	let mut compiler = PlatformShaderCompiler::new();
	let language = PlatformShaderLanguage::current_platform();
	// SPIR-V kernels have always reflected their bindings, which rejects overlapping slots. Metal and DX12 kernels
	// never did, so they take their source text alone.
	let source = if language.is_glsl() {
		compiler.lower(settings, program).map(|lowered| lowered.source)
	} else {
		compiler.lower_source(settings, program)
	}
	.map_err(|error| format!("BESL kernel '{label}' could not be lowered for this platform. {error}"))?;
	let entry_point = language.entry_point().to_string();
	Ok(match language {
		#[cfg(target_os = "linux")]
		PlatformShaderLanguage::Glsl => ghi::shader::CompiledShaderSource::SPIRV(
			crate::shader::besl::backends::spirv::compile_glsl_to_spirv(&source, &settings.name)?.into_vec(),
		),
		// Lowering already reported platforms without a GLSL compiler.
		#[cfg(not(target_os = "linux"))]
		PlatformShaderLanguage::Glsl => unreachable!("GLSL kernels are only lowered on Linux"),
		PlatformShaderLanguage::Hlsl => ghi::shader::CompiledShaderSource::HLSL { source, entry_point },
		PlatformShaderLanguage::Msl => ghi::shader::CompiledShaderSource::MTL { source, entry_point },
	})
}

/// Creates the shader and compute pipeline for compiled kernel source, whose `resources` form the one descriptor set
/// the kernel reads.
fn build_compute_pipeline(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	compiled: &ghi::shader::CompiledShaderSource,
	resources: impl IntoIterator<Item = ghi::ShaderResourceDescriptor>,
	push_constant_size: usize,
) -> Result<ComputeKernel, GpuWorkerError> {
	let layout = ghi::DescriptorSetLayout::new(label, resources);
	let shader = context
		.create_shader(
			Some(label),
			compiled.as_source(),
			ghi::ShaderTypes::Compute,
			[ghi::ShaderDescriptorSet::new(layout.clone())],
		)
		.map_err(|_| GpuWorkerError::ShaderCreation)?;
	let push_constant_ranges = [ghi::pipelines::PushConstantRange::new(0, push_constant_size as u32)];

	let pipeline = context.create_compute_pipeline(
		ghi::pipelines::compute::Builder::new(
			&push_constant_ranges,
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		)
		.name(label),
	);
	Ok(ComputeKernel { pipeline, layout })
}

#[cfg(test)]
mod tests {
	use std::sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	};

	use utils::r#async::FutureExt as _;

	use super::*;

	/// A processor whose batches take `batch_time` on the "GPU".
	///
	/// Each result reports the size of the batch that carried it, which is the observable that proves the worker
	/// batches requests.
	struct FakeProcessor<const CAPACITY: usize> {
		batch_time: std::time::Duration,
		peak_batch: Arc<AtomicUsize>,
	}

	impl<const CAPACITY: usize> GpuProcessor for FakeProcessor<CAPACITY> {
		type Request = u8;
		type Result = (u8, usize);

		const MAX_BATCH: usize = CAPACITY;

		fn process(&mut self, batch: &[(u8, Vec<u8>)]) -> BatchResults<(u8, usize)> {
			assert!(
				batch.len() <= CAPACITY,
				"the worker never batches past the processor's capacity"
			);
			self.peak_batch.fetch_max(batch.len(), Ordering::SeqCst);
			std::thread::sleep(self.batch_time);
			batch.iter().map(|(request, _)| (*request, batch.len())).collect()
		}
	}

	fn worker<const CAPACITY: usize>(batch_milliseconds: u64) -> (GpuWorker<FakeProcessor<CAPACITY>>, Arc<AtomicUsize>) {
		let peak = Arc::new(AtomicUsize::new(0));
		let peak_for_worker = peak.clone();
		let worker = GpuWorker::spawn("Fake GPU Worker", move || {
			Ok::<_, GpuWorkerError>(FakeProcessor::<CAPACITY> {
				batch_time: std::time::Duration::from_millis(batch_milliseconds),
				peak_batch: peak_for_worker,
			})
		})
		.expect("the fake worker should start");
		(worker, peak)
	}

	/// Claims a place, serves one request under it, and releases the place with the reply.
	async fn serve<const CAPACITY: usize>(
		worker: &GpuWorker<FakeProcessor<CAPACITY>>,
		request: u8,
		input: Vec<u8>,
	) -> Result<(u8, usize), oneshot::Canceled> {
		let claim = worker.claim().await;
		worker.submit(&claim, request, input).await
	}

	#[crate::r#async::test]
	async fn concurrent_requests_share_a_batch() {
		let (worker, peak) = worker::<3>(20);

		let results = utils::r#async::join_all((0..6_u8).map(|request| serve(&worker, request, vec![0; 4]))).await;

		for (request, result) in (0..6_u8).zip(results) {
			let (served, _) = result.expect("the worker should serve every request");
			assert_eq!(served, request);
		}
		assert!(
			peak.load(Ordering::SeqCst) >= 2,
			"requests queued together should run as one batch"
		);
	}

	#[crate::r#async::test]
	async fn requests_past_capacity_wait_for_the_next_batch_and_keep_their_order() {
		let (worker, peak) = worker::<1>(2);

		let results = utils::r#async::join_all((0..6_u8).map(|request| serve(&worker, request, vec![0; 1])))
			.await
			.into_iter()
			.map(|reply| reply.expect("the worker should serve every request").0)
			.collect::<Vec<_>>();

		assert_eq!(results, [0, 1, 2, 3, 4, 5]);
		assert_eq!(peak.load(Ordering::SeqCst), 1);
	}

	#[crate::r#async::test]
	async fn claims_past_capacity_wait_and_are_granted_in_arrival_order() {
		let (worker, _) = worker::<1>(2);
		let [released, kept] = [worker.claim().await, worker.claim().await];
		let mut first = std::pin::pin!(worker.claim());
		let mut second = std::pin::pin!(worker.claim());

		assert!(first.as_mut().now_or_never().is_none(), "every place is taken");
		assert!(second.as_mut().now_or_never().is_none(), "every place is taken");

		drop(released);

		assert!(
			second.as_mut().now_or_never().is_none(),
			"a released place goes to the oldest waiter"
		);
		let _first = first
			.now_or_never()
			.expect("the oldest waiter should hold the released place");

		drop(kept);

		assert!(
			second.now_or_never().is_some(),
			"the next release should reach the next waiter"
		);
	}

	#[crate::r#async::test]
	async fn a_freed_place_goes_first_to_a_waiter_whose_thread_holds_no_claim() {
		let (worker, _) = worker::<1>(2);
		let worker = &worker;
		// This thread holds one place and another thread holds the other.
		let kept = worker.claim().await;
		let released = std::thread::scope(|scope| {
			let other = scope.spawn(|| worker.claim().now_or_never().expect("one place is still free"));
			other.join().expect("the claiming thread should finish")
		});
		// This thread starts waiting while it holds a claim, before a thread that holds none.
		let mut older = std::pin::pin!(worker.claim());
		assert!(older.as_mut().now_or_never().is_none(), "every place is taken");

		let taken = std::thread::scope(|scope| {
			let (registered, wait_until_registered) = mpsc::channel();
			let (freed, wait_until_freed) = mpsc::channel::<()>();
			let younger = scope.spawn(move || {
				let mut younger = std::pin::pin!(worker.claim());
				assert!(younger.as_mut().now_or_never().is_none(), "every place is taken");
				registered.send(()).expect("the test thread should wait for the registration");
				wait_until_freed.recv().expect("the test thread should free a place");
				younger.now_or_never()
			});
			wait_until_registered.recv().expect("the waiting thread should register");
			drop(released);
			freed.send(()).expect("the waiting thread should wait for the freed place");
			younger.join().expect("the waiting thread should finish")
		});

		assert!(
			taken.is_some(),
			"the waiter on a thread without a claim should get the freed place"
		);
		assert!(
			older.as_mut().now_or_never().is_none(),
			"the older waiter's thread still holds a claim"
		);
		drop(kept);
		assert!(
			older.now_or_never().is_some(),
			"the older waiter should get the next place once its thread holds no claim"
		);
	}

	#[crate::r#async::test]
	async fn a_waiter_that_stops_waiting_passes_its_place_on() {
		let (worker, _) = worker::<1>(2);
		let [released, kept] = [worker.claim().await, worker.claim().await];
		let mut abandoned = Box::pin(worker.claim());
		assert!(abandoned.as_mut().now_or_never().is_none(), "every place is taken");

		// The released place reaches the waiter, which stops waiting before it takes it.
		drop(released);
		drop(abandoned);

		assert!(
			worker.claim().now_or_never().is_some(),
			"the abandoned place should be free again"
		);
		drop(kept);
	}

	#[crate::r#async::test]
	async fn a_dropped_reply_does_not_disturb_later_requests() {
		let (worker, _) = worker::<1>(3);

		let claim = worker.claim().await;
		drop(worker.submit(&claim, 1, vec![0; 1]));
		drop(claim);
		let (served, _) = serve(&worker, 2, vec![0; 1]).await.expect("the worker should reply");

		assert_eq!(served, 2);
	}

	#[crate::r#async::test]
	async fn a_stopped_worker_cancels_replies_but_finishes_queued_work() {
		let stopped = GpuWorker::<FakeProcessor<1>>::unavailable();

		assert!(serve(&stopped, 1, vec![0; 1]).await.is_err());

		let (worker, _) = worker::<1>(20);
		let [first, second] = [worker.claim().await, worker.claim().await];
		let running = worker.submit(&first, 1, vec![0; 1]);
		let queued = worker.submit(&second, 2, vec![0; 1]);
		drop(worker);

		assert!(running.await.is_ok(), "work the GPU already runs still completes");
		assert!(queued.await.is_ok(), "work queued before shutdown still completes");
	}
}
