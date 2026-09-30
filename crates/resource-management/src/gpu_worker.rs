//! Run thread-confined GHI compute processors for offline asset processing.
//!
//! The environment-map generator ([`crate::ibl::gpu::GPUIBLClient`]) and the material mip generator
//! ([`crate::resources::mips::gpu::MaterialMipGenerator`]) share this worker thread, device setup, and kernel setup.
//! Asset bakes submit requests from the shared worker pool and await the reply, so no pool thread blocks while the
//! GPU works, and the worker keeps several requests in flight at once.

use std::{
	any::Any,
	collections::VecDeque,
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
	/// The value a request resolves to. Bytes the request produced travel in its output buffer instead.
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
	/// Return `None` while its GPU work runs. Once the work completed, write its bytes into `output`, release the
	/// ticket's scratch, and return the result.
	fn poll(&mut self, ticket: Self::Ticket, output: &mut [u8]) -> Option<Self::Result>;
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
/// `Q` is the request's parameters and `R` its result. Requests own their input and output bytes while they are in
/// flight, so a caller that stops awaiting never leaves the worker with dangling memory.
pub(crate) struct GpuWorker<Q, R> {
	/// `None` once the worker was asked to shut down, or when it never started.
	sender: Option<Sender<Request<Q, R>>>,
	worker: Option<JoinHandle<()>>,
}

/// The `GpuWorkerSpawnError` enum reports why [`GpuWorker::spawn`] could not start a worker.
pub(crate) enum GpuWorkerSpawnError<E> {
	/// The processor factory failed on the worker thread.
	Initialization(E),
	/// The operating system could not create the worker thread.
	WorkerCreation(std::io::Error),
	/// The worker stopped before it reported initialization.
	WorkerUnavailable,
}

impl<Q: Send + 'static, R: Send + 'static> GpuWorker<Q, R> {
	/// Runs `initialize` on a new thread named `name` and returns once the processor it creates is ready.
	///
	/// Create every thread-affine GHI device and context inside `initialize`. The processor stays on the worker for its
	/// whole lifetime and serves every request through [`GpuProcessor`].
	pub(crate) fn spawn<P, E>(
		name: &str,
		initialize: impl FnOnce() -> Result<P, E> + Send + 'static,
	) -> Result<Self, GpuWorkerSpawnError<E>>
	where
		P: GpuProcessor<Request = Q, Result = R> + 'static,
		E: Send + 'static,
	{
		let (sender, receiver) = mpsc::channel::<Request<Q, R>>();
		let (startup, startup_receiver) = mpsc::sync_channel(1);
		let worker = std::thread::Builder::new()
			.name(name.to_string())
			.spawn(move || {
				let processor = match initialize() {
					Ok(processor) => {
						let _ = startup.send(Ok(()));
						processor
					}
					Err(error) => {
						let _ = startup.send(Err(error));
						return;
					}
				};
				serve(processor, receiver);
			})
			.map_err(GpuWorkerSpawnError::WorkerCreation)?;

		match startup_receiver.recv() {
			Ok(Ok(())) => Ok(Self {
				sender: Some(sender),
				worker: Some(worker),
			}),
			Ok(Err(error)) => {
				let _ = worker.join();
				Err(GpuWorkerSpawnError::Initialization(error))
			}
			Err(_) => {
				let _ = worker.join();
				Err(GpuWorkerSpawnError::WorkerUnavailable)
			}
		}
	}

	/// Queues one request and returns the receiver that resolves to its result and its output buffer.
	///
	/// The worker reads `input` and writes `output`, which comes back with the result. Pass an empty `output` when the
	/// result carries everything. The receiver reports cancellation when the worker stopped before serving the
	/// request. Dropping it does not cancel the GPU work; the worker finishes the request and discards the result.
	pub(crate) fn submit(&self, parameters: Q, input: Vec<u8>, output: Vec<u8>) -> oneshot::Receiver<(R, Vec<u8>)> {
		let (reply, receiver) = oneshot::channel();
		if let Some(sender) = &self.sender {
			// A failed send drops the request and its reply sender, which cancels the receiver.
			let _ = sender.send(Request {
				parameters,
				input,
				output,
				reply,
			});
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

impl<Q, R> Drop for GpuWorker<Q, R> {
	fn drop(&mut self) {
		// Closing the channel asks the worker to finish its in-flight work and stop.
		drop(self.sender.take());
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

/// The `Request` struct carries one request's parameters, owned bytes, and reply channel to the worker.
struct Request<Q, R> {
	parameters: Q,
	input: Vec<u8>,
	output: Vec<u8>,
	reply: oneshot::Sender<(R, Vec<u8>)>,
}

/// Serves requests on the worker thread until the channel closes, keeping as many in flight as the processor allows.
///
/// Queued requests are admitted in arrival order and complete in whatever order the GPU finishes them. Once the
/// channel closes, requests the GPU already runs still complete, and queued ones drop, which cancels their receivers.
fn serve<P: GpuProcessor>(mut processor: P, receiver: Receiver<Request<P::Request, P::Result>>) {
	let mut queued = VecDeque::<Request<P::Request, P::Result>>::new();
	let mut in_flight = Vec::<(P::Ticket, Request<P::Request, P::Result>)>::new();
	let mut closed = false;
	loop {
		if closed {
			if in_flight.is_empty() {
				return;
			}
			std::thread::sleep(POLL_INTERVAL);
		} else {
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
				Err(RecvTimeoutError::Disconnected) => {
					closed = true;
					queued.clear();
				}
			}
		}

		// Finish completed work first so the scratch it held can serve the queue.
		finish_completed(&mut processor, &mut in_flight);
		while in_flight.len() < P::MAX_IN_FLIGHT
			&& let Some(request) = queued.pop_front()
		{
			match processor.submit(&request.parameters, &request.input) {
				Submission::InFlight(ticket) => in_flight.push((ticket, request)),
				Submission::Complete(result) => {
					let Request { output, reply, .. } = request;
					let _ = reply.send((result, output));
				}
			}
		}
	}
}

/// Polls every in-flight request once and replies to those that completed.
fn finish_completed<P: GpuProcessor>(processor: &mut P, in_flight: &mut Vec<(P::Ticket, Request<P::Request, P::Result>)>) {
	let mut index = 0;
	while index < in_flight.len() {
		let (ticket, request) = &mut in_flight[index];
		match processor.poll(*ticket, &mut request.output) {
			Some(result) => {
				let (_, Request { output, reply, .. }) = in_flight.swap_remove(index);
				let _ = reply.send((result, output));
			}
			None => index += 1,
		}
	}
}

/// The `OwnedContext` struct keeps a GHI context and the native state it depends on together while a processor is built.
///
/// Its field order drops the context before its owner on every early-return and unwinding path. Destructure it once the
/// processor is complete and keep both values in the same order there.
pub(crate) struct OwnedContext {
	pub(crate) context: ghi::implementation::Context,
	pub(crate) owner: Box<dyn Any>,
}

/// The `ComputeContextError` enum reports which step of standalone compute-context creation failed.
pub(crate) enum ComputeContextError {
	Instance(&'static str),
	Device(&'static str),
	Context(&'static str),
}

/// Creates a self-contained compute and transfer device and context for offline asset processing.
///
/// Returns the context, its queue, and an owner that keeps the device and instance alive; put the context and owner in
/// an [`OwnedContext`] before building anything else on them.
pub(crate) fn create_compute_context()
-> Result<(ghi::implementation::Context, ghi::QueueHandle, Box<dyn Any>), ComputeContextError> {
	let features = ghi::device::Features::new().mesh_shading(false);
	let mut instance = ghi::implementation::Instance::new(features).map_err(ComputeContextError::Instance)?;
	let mut queue = None;
	let device = instance
		.create_device(
			features,
			&mut [(
				ghi::QueueSelection::new(ghi::WorkloadTypes::COMPUTE | ghi::WorkloadTypes::TRANSFER),
				&mut queue,
			)],
		)
		.map_err(ComputeContextError::Device)?;
	let context = device.create_context().map_err(ComputeContextError::Context)?;
	let queue = queue.expect("GHI device creation must populate the requested compute queue handle.");

	Ok((context, queue, Box::new((device, instance))))
}

/// The `ComputeKernelError` enum reports why a compute kernel could not become a pipeline.
pub(crate) enum ComputeKernelError {
	Compilation(String),
	Creation,
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
) -> Result<ghi::PipelineHandle, ComputeKernelError> {
	let compiled = ghi::shader::compile(label, source).map_err(ComputeKernelError::Compilation)?;
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
) -> Result<ghi::PipelineHandle, ComputeKernelError> {
	let program = besl::compile_to_besl(source, None).map_err(|error| {
		ComputeKernelError::Compilation(format!(
			"BESL kernel '{label}' failed to parse or link. The most likely cause is invalid kernel source. Error: {error:?}"
		))
	})?;
	let settings = crate::shader::ShaderGenerationSettings::compute(workgroup).name(label.to_string());
	let compiled = native_kernel_source(label, &settings, &program).map_err(ComputeKernelError::Compilation)?;
	build_compute_pipeline(context, label, &compiled, resources, push_constant_size)
}

/// Generates Metal source for a linked BESL kernel. The Metal driver compiles it when the shader is created.
#[cfg(target_os = "macos")]
fn native_kernel_source(
	label: &str,
	settings: &crate::shader::ShaderGenerationSettings,
	program: &besl::NodeReference,
) -> Result<ghi::shader::CompiledShaderSource, String> {
	use crate::shader::besl::backends::msl::{MSL_ENTRY_POINT, MSLTranspiler};

	let source = MSLTranspiler::new().generate_program(settings, program).map_err(|()| {
		format!(
			"MSL generation failed for BESL kernel '{label}'. The most likely cause is a BESL construct the MSL backend can't lower."
		)
	})?;
	Ok(ghi::shader::CompiledShaderSource::MTL {
		source,
		entry_point: MSL_ENTRY_POINT.to_string(),
	})
}

/// Generates HLSL source for a linked BESL kernel. The DX12 backend compiles it when the shader is created.
#[cfg(target_os = "windows")]
fn native_kernel_source(
	label: &str,
	settings: &crate::shader::ShaderGenerationSettings,
	program: &besl::NodeReference,
) -> Result<ghi::shader::CompiledShaderSource, String> {
	use crate::shader::besl::backends::{hlsl::HLSLTranspiler, platform::PlatformShaderLanguage};

	let main = program.get_main().ok_or_else(|| {
		format!("BESL kernel '{label}' has no `main` function. The most likely cause is a renamed entry point.")
	})?;
	let source = HLSLTranspiler::new().generate(settings, &main).map_err(|()| {
		format!(
			"HLSL generation failed for BESL kernel '{label}'. The most likely cause is a BESL construct the HLSL backend can't lower."
		)
	})?;
	Ok(ghi::shader::CompiledShaderSource::HLSL {
		source,
		entry_point: PlatformShaderLanguage::Hlsl.entry_point().to_string(),
	})
}

/// Compiles a linked BESL kernel to SPIR-V for the Vulkan backend.
#[cfg(target_os = "linux")]
fn native_kernel_source(
	label: &str,
	settings: &crate::shader::ShaderGenerationSettings,
	program: &besl::NodeReference,
) -> Result<ghi::shader::CompiledShaderSource, String> {
	use crate::shader::besl::backends::spirv::SPIRVCompiler;

	let main = program.get_main().ok_or_else(|| {
		format!("BESL kernel '{label}' has no `main` function. The most likely cause is a renamed entry point.")
	})?;
	let (binary, ..) = SPIRVCompiler::new().generate(settings, &main)?.into_parts();
	Ok(ghi::shader::CompiledShaderSource::SPIRV(binary.into_vec()))
}

/// Reports that no graphics backend on this platform can run BESL kernels.
#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn native_kernel_source(
	label: &str,
	_: &crate::shader::ShaderGenerationSettings,
	_: &besl::NodeReference,
) -> Result<ghi::shader::CompiledShaderSource, String> {
	Err(format!(
		"BESL kernel '{label}' can't be compiled on this platform. The most likely cause is an operating system without a supported graphics backend."
	))
}

/// Creates the shader and compute pipeline for compiled kernel source.
fn build_compute_pipeline(
	context: &mut ghi::implementation::Context,
	label: &'static str,
	compiled: &ghi::shader::CompiledShaderSource,
	resources: impl IntoIterator<Item = ghi::ShaderResourceDescriptor>,
	push_constant_size: usize,
) -> Result<ghi::PipelineHandle, ComputeKernelError> {
	let shader = context
		.create_shader(Some(label), compiled.as_source(), ghi::ShaderTypes::Compute, resources)
		.map_err(|_| ComputeKernelError::Creation)?;
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

		fn poll(&mut self, ticket: usize, output: &mut [u8]) -> Option<(u8, usize)> {
			let (polls, request) = self.slots[ticket].as_mut().expect("polled a free slot");
			*polls += 1;
			if *polls < self.polls_to_complete {
				return None;
			}
			let request = *request;
			let in_flight = self.slots.iter().filter(|slot| slot.is_some()).count();
			self.slots[ticket] = None;
			output.fill(request);
			Some((request, in_flight))
		}
	}

	fn worker<const CAPACITY: usize>(polls_to_complete: u32) -> (GpuWorker<u8, (u8, usize)>, Arc<AtomicUsize>) {
		let peak = Arc::new(AtomicUsize::new(0));
		let peak_for_worker = peak.clone();
		let worker = GpuWorker::spawn("Fake GPU Worker", move || {
			Ok::<_, ()>(FakeProcessor::<CAPACITY> {
				polls_to_complete,
				slots: [None; CAPACITY],
				peak_in_flight: peak_for_worker,
			})
		})
		.ok()
		.expect("the fake worker should start");
		(worker, peak)
	}

	#[crate::r#async::test]
	async fn concurrent_requests_overlap_on_the_worker() {
		let (worker, peak) = worker::<3>(4);

		let replies = (0..6_u8)
			.map(|request| worker.submit(request, vec![0; 4], vec![0; 2]))
			.collect::<Vec<_>>();
		let mut results = Vec::new();
		for reply in replies {
			results.push(reply.await.expect("the worker should serve every request"));
		}

		for (request, ((served, _), output)) in (0..6_u8).zip(&results) {
			assert_eq!(*served, request);
			assert_eq!(output, &vec![request; 2], "the output buffer comes back written");
		}
		assert!(
			peak.load(Ordering::SeqCst) >= 2,
			"several requests should be in flight at once"
		);
	}

	#[crate::r#async::test]
	async fn requests_past_capacity_wait_for_a_slot_and_keep_their_order() {
		let (worker, peak) = worker::<1>(2);

		let replies = (0..4_u8)
			.map(|request| worker.submit(request, vec![0; 1], Vec::new()))
			.collect::<Vec<_>>();
		let mut results = Vec::new();
		for reply in replies {
			results.push(reply.await.expect("the worker should serve every request").0.0);
		}

		assert_eq!(results, [0, 1, 2, 3]);
		assert_eq!(peak.load(Ordering::SeqCst), 1);
	}

	#[crate::r#async::test]
	async fn requests_without_gpu_work_complete_inline() {
		let (worker, _) = worker::<1>(2);

		let ((served, in_flight), _) = worker
			.submit(9, Vec::new(), Vec::new())
			.await
			.expect("the worker should reply");

		assert_eq!((served, in_flight), (9, 0));
	}

	#[crate::r#async::test]
	async fn a_dropped_reply_does_not_disturb_later_requests() {
		let (worker, _) = worker::<1>(3);

		drop(worker.submit(1, vec![0; 1], Vec::new()));
		let ((served, _), _) = worker
			.submit(2, vec![0; 1], Vec::new())
			.await
			.expect("the worker should reply");

		assert_eq!(served, 2);
	}

	#[crate::r#async::test]
	async fn a_stopped_worker_cancels_replies() {
		let stopped = GpuWorker::<u8, (u8, usize)>::unavailable();

		assert!(stopped.submit(1, vec![0; 1], Vec::new()).await.is_err());

		let (worker, _) = worker::<1>(50);
		let in_flight = worker.submit(1, vec![0; 1], Vec::new());
		let queued = worker.submit(2, vec![0; 1], Vec::new());
		drop(worker);

		assert!(in_flight.await.is_ok(), "work the GPU already runs still completes");
		assert!(queued.await.is_err(), "queued work is abandoned at shutdown");
	}
}
