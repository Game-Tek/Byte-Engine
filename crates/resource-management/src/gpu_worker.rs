//! Run thread-confined GHI compute processors for offline asset processing.
//!
//! The environment-map generator ([`crate::ibl::IBLGenerator`]) and the material mip generator
//! ([`crate::resources::mips::gpu::MaterialMipGenerator`]) share this worker thread, device setup, and kernel setup.
//! Asset bakes submit requests from the shared worker pool and await the reply, so no pool thread blocks while the
//! GPU works, and the worker keeps several requests in flight at once.

use std::{
	any::Any,
	collections::VecDeque,
	fmt,
	sync::mpsc::{self, Receiver, RecvTimeoutError, Sender},
	thread::JoinHandle,
	time::Duration,
};

use ghi::{context::ContextCreate as _, device::Device as _};
use utils::r#async::oneshot;

/// The slot every offline kernel samples its source image from.
pub(crate) const SOURCE_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);

/// The slot every offline kernel writes its output image or buffer to.
pub(crate) const OUTPUT_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);

/// How long the worker sleeps between completion polls while requests are in flight.
///
/// Bakes last milliseconds to seconds, so one millisecond of completion latency is invisible next to the GPU time it
/// lets the worker overlap. A backend wake-up primitive can replace the interval if profiling ever shows it matters.
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/// The `GpuProcessor` trait lets a thread-confined GHI processor keep several requests in flight on one [`GpuWorker`].
///
/// Implement it on the type that owns a compute context. [`Self::submit`] uploads and records one request without
/// waiting for the GPU, and [`Self::poll`] finishes it once its synchronizer has signaled. The worker calls both from
/// the thread the processor was created on, so the context never crosses threads, and it never has more than
/// [`Self::MAX_IN_FLIGHT`] requests submitted at once. Create the worker with [`GpuWorker::spawn`].
pub(crate) trait GpuProcessor {
	/// The parameters of one request, next to its input bytes.
	type Request: Send + 'static;
	/// The value a request resolves to, including any bytes it produced.
	type Result: Send + 'static;
	/// Identifies one in-flight request to [`Self::poll`], such as the scratch it occupies.
	type Ticket: Copy;

	/// The most requests the worker submits before one completes, which bounds the scratch the processor needs.
	const MAX_IN_FLIGHT: usize;

	/// Uploads `input`, records the GPU work for `request`, and submits it.
	///
	/// Return [`Submission::InFlight`] with the ticket [`Self::poll`] finishes it with, or [`Submission::Complete`]
	/// when the request needed no GPU work.
	fn submit(&mut self, request: &Self::Request, input: &[u8]) -> Submission<Self::Ticket, Self::Result>;

	/// Checks one in-flight request without blocking.
	///
	/// Return `None` while its GPU work runs. Once the work completed, release the ticket's scratch and return the
	/// result.
	fn poll(&mut self, ticket: Self::Ticket) -> Option<Self::Result>;
}

/// The `Submission` enum reports how a [`GpuProcessor`] took one request.
pub(crate) enum Submission<T, R> {
	/// The GPU runs the request; finish it by polling `T`.
	InFlight(T),
	/// The request needed no GPU work and its result is final.
	Complete(R),
}

/// The `GpuWorker` struct lets asset handlers on the shared worker pool use a GHI processor that must stay on one thread.
///
/// Spawn it with the processor factory, then call [`Self::submit`] once per image and await the returned receiver.
/// Requests own their input bytes until they are uploaded, so a caller that stops awaiting never leaves the worker
/// with dangling memory.
pub(crate) struct GpuWorker<P: GpuProcessor> {
	/// `None` once the worker was asked to shut down, or when it never started.
	sender: Option<Sender<Request<P>>>,
	worker: Option<JoinHandle<()>>,
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
			}),
			Err(error) => {
				let _ = worker.join();
				Err(error)
			}
		}
	}

	/// Queues one request and returns the receiver that resolves to its result.
	///
	/// The worker uploads `input`. The receiver reports cancellation when the worker stopped before serving the
	/// request. Dropping it does not cancel the GPU work; the worker finishes the request and discards the result.
	pub(crate) fn submit(&self, parameters: P::Request, input: Vec<u8>) -> oneshot::Receiver<P::Result> {
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
		}
	}
}

impl<P: GpuProcessor> Drop for GpuWorker<P> {
	fn drop(&mut self) {
		// Closing the channel asks the worker to finish its in-flight work and stop.
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

/// Serves requests on the worker thread until the channel closes, keeping as many in flight as the processor allows.
///
/// Queued requests are admitted in arrival order and complete in whatever order the GPU finishes them. Once the
/// channel closes, requests the GPU already runs still complete, and queued ones drop, which cancels their receivers.
fn serve<P: GpuProcessor>(mut processor: P, receiver: Receiver<Request<P>>) {
	let mut queued = VecDeque::new();
	// The replies of the requests the GPU runs, next to the tickets that finish them.
	let mut in_flight = Vec::new();
	loop {
		// Park while idle; otherwise take what arrived and return to polling after one interval.
		let received = if queued.is_empty() && in_flight.is_empty() {
			receiver.recv().map_err(|_| RecvTimeoutError::Disconnected)
		} else {
			receiver.recv_timeout(POLL_INTERVAL)
		};
		match received {
			Ok(request) => {
				queued.push_back(request);
				queued.extend(receiver.try_iter());
			}
			Err(RecvTimeoutError::Timeout) => {}
			// Dropping the queue cancels its receivers; the worker stops once the GPU finished what it runs.
			Err(RecvTimeoutError::Disconnected) => {
				queued.clear();
				loop {
					finish_completed(&mut processor, &mut in_flight);
					if in_flight.is_empty() {
						return;
					}
					std::thread::sleep(POLL_INTERVAL);
				}
			}
		}

		// Finish completed work first so the scratch it held can serve the queue.
		finish_completed(&mut processor, &mut in_flight);
		while in_flight.len() < P::MAX_IN_FLIGHT
			&& let Some((parameters, input, reply)) = queued.pop_front()
		{
			// The input is uploaded once submitted, so only the reply waits for the GPU.
			match processor.submit(&parameters, &input) {
				Submission::InFlight(ticket) => in_flight.push((ticket, reply)),
				Submission::Complete(result) => {
					let _ = reply.send(result);
				}
			}
		}
	}
}

/// Polls every in-flight request once and replies to those that completed.
fn finish_completed<P: GpuProcessor>(processor: &mut P, in_flight: &mut Vec<(P::Ticket, oneshot::Sender<P::Result>)>) {
	let mut index = 0;
	while index < in_flight.len() {
		match processor.poll(in_flight[index].0) {
			Some(result) => {
				let _ = in_flight.swap_remove(index).1.send(result);
			}
			None => index += 1,
		}
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

/// Compiles one native compute kernel that samples [`SOURCE_SLOT`] and writes the storage image at [`OUTPUT_SLOT`], and
/// builds its pipeline.
///
/// `label` names the shader and pipeline in tools, and `push_constant_size` is the size of the kernel's push constants.
pub(crate) fn create_compute_kernel(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	source: ghi::shader::ShaderSource<'_>,
	push_constant_size: usize,
) -> Result<ghi::PipelineHandle, GpuWorkerError> {
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
) -> Result<ghi::PipelineHandle, GpuWorkerError> {
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

/// Creates the shader and compute pipeline for compiled kernel source.
fn build_compute_pipeline(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	compiled: &ghi::shader::CompiledShaderSource,
	resources: impl IntoIterator<Item = ghi::ShaderResourceDescriptor>,
	push_constant_size: usize,
) -> Result<ghi::PipelineHandle, GpuWorkerError> {
	let shader = context
		.create_shader(Some(label), compiled.as_source(), ghi::ShaderTypes::Compute, resources)
		.map_err(|_| GpuWorkerError::ShaderCreation)?;
	let push_constant_ranges = [ghi::pipelines::PushConstantRange::new(0, push_constant_size as u32)];

	Ok(context.create_compute_pipeline(
		ghi::pipelines::compute::Builder::new(
			&push_constant_ranges,
			ghi::ShaderParameter::new(&shader, ghi::ShaderTypes::Compute),
		)
		.name(label),
	))
}

#[cfg(test)]
mod tests {
	use std::sync::{
		Arc,
		atomic::{AtomicUsize, Ordering},
	};

	use super::*;

	/// A processor whose requests complete after `polls_to_complete` polls.
	///
	/// Each result reports how many requests were in flight when it completed, which is the observable that proves
	/// the worker overlaps requests.
	struct FakeProcessor<const CAPACITY: usize> {
		polls_to_complete: u32,
		slots: [Option<(u32, u8)>; CAPACITY],
		peak_in_flight: Arc<AtomicUsize>,
	}

	impl<const CAPACITY: usize> GpuProcessor for FakeProcessor<CAPACITY> {
		type Request = u8;
		type Result = (u8, usize);
		type Ticket = usize;

		const MAX_IN_FLIGHT: usize = CAPACITY;

		fn submit(&mut self, request: &u8, input: &[u8]) -> Submission<usize, (u8, usize)> {
			if input.is_empty() {
				return Submission::Complete((*request, 0));
			}
			let slot = self
				.slots
				.iter()
				.position(Option::is_none)
				.expect("the worker never submits past the processor's capacity");
			self.slots[slot] = Some((0, *request));
			let in_flight = self.slots.iter().filter(|slot| slot.is_some()).count();
			self.peak_in_flight.fetch_max(in_flight, Ordering::SeqCst);
			Submission::InFlight(slot)
		}

		fn poll(&mut self, ticket: usize) -> Option<(u8, usize)> {
			let (polls, request) = self.slots[ticket].as_mut().expect("polled a free slot");
			*polls += 1;
			if *polls < self.polls_to_complete {
				return None;
			}
			let request = *request;
			let in_flight = self.slots.iter().filter(|slot| slot.is_some()).count();
			self.slots[ticket] = None;
			Some((request, in_flight))
		}
	}

	fn worker<const CAPACITY: usize>(polls_to_complete: u32) -> (GpuWorker<FakeProcessor<CAPACITY>>, Arc<AtomicUsize>) {
		let peak = Arc::new(AtomicUsize::new(0));
		let peak_for_worker = peak.clone();
		let worker = GpuWorker::spawn("Fake GPU Worker", move || {
			Ok::<_, GpuWorkerError>(FakeProcessor::<CAPACITY> {
				polls_to_complete,
				slots: [None; CAPACITY],
				peak_in_flight: peak_for_worker,
			})
		})
		.expect("the fake worker should start");
		(worker, peak)
	}

	#[crate::r#async::test]
	async fn concurrent_requests_overlap_on_the_worker() {
		let (worker, peak) = worker::<3>(4);

		let results = utils::r#async::join_all((0..6_u8).map(|request| worker.submit(request, vec![0; 4]))).await;

		for (request, result) in (0..6_u8).zip(results) {
			let (served, _) = result.expect("the worker should serve every request");
			assert_eq!(served, request);
		}
		assert!(
			peak.load(Ordering::SeqCst) >= 2,
			"several requests should be in flight at once"
		);
	}

	#[crate::r#async::test]
	async fn requests_past_capacity_wait_for_a_slot_and_keep_their_order() {
		let (worker, peak) = worker::<1>(2);

		let results = utils::r#async::join_all((0..4_u8).map(|request| worker.submit(request, vec![0; 1])))
			.await
			.into_iter()
			.map(|reply| reply.expect("the worker should serve every request").0)
			.collect::<Vec<_>>();

		assert_eq!(results, [0, 1, 2, 3]);
		assert_eq!(peak.load(Ordering::SeqCst), 1);
	}

	#[crate::r#async::test]
	async fn requests_without_gpu_work_complete_inline() {
		let (worker, _) = worker::<1>(2);

		let (served, in_flight) = worker.submit(9, Vec::new()).await.expect("the worker should reply");

		assert_eq!((served, in_flight), (9, 0));
	}

	#[crate::r#async::test]
	async fn a_dropped_reply_does_not_disturb_later_requests() {
		let (worker, _) = worker::<1>(3);

		drop(worker.submit(1, vec![0; 1]));
		let (served, _) = worker.submit(2, vec![0; 1]).await.expect("the worker should reply");

		assert_eq!(served, 2);
	}

	#[crate::r#async::test]
	async fn a_stopped_worker_cancels_replies() {
		let stopped = GpuWorker::<FakeProcessor<1>>::unavailable();

		assert!(stopped.submit(1, vec![0; 1]).await.is_err());

		let (worker, _) = worker::<1>(50);
		let in_flight = worker.submit(1, vec![0; 1]);
		let queued = worker.submit(2, vec![0; 1]);
		drop(worker);

		assert!(in_flight.await.is_ok(), "work the GPU already runs still completes");
		assert!(queued.await.is_err(), "queued work is abandoned at shutdown");
	}
}
