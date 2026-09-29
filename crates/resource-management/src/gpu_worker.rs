//! Run thread-confined GHI compute processors for offline asset processing.
//!
//! The environment-map generator ([`crate::ibl::gpu::GPUIBLClient`]) and the material mip generator
//! ([`crate::resources::mips::gpu::MaterialMipGenerator`]) share this worker thread, device setup, and kernel setup.

use std::{
	any::Any,
	sync::{
		Mutex,
		mpsc::{self, Receiver, SyncSender},
	},
	thread::JoinHandle,
};

use ghi::{context::ContextCreate as _, device::Device as _};

/// The slot every offline kernel samples its source image from.
pub(crate) const SOURCE_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(0);

/// The slot every offline kernel writes its output image to.
pub(crate) const OUTPUT_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);

/// The `GpuWorker` struct lets asset handlers on the shared worker pool use a GHI processor that must stay on one thread.
///
/// Spawn it with the processor factory and the function that serves one request, then call [`Self::call`] once per
/// image. `Q` is the request's parameters and `R` its result.
pub(crate) struct GpuWorker<Q, R> {
	/// `None` asks the worker to shut down.
	sender: SyncSender<Option<BorrowedRequest<Q>>>,
	responses: Mutex<Receiver<R>>,
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
	/// whole lifetime, and each request runs `serve` with the caller's parameters and borrowed bytes.
	pub(crate) fn spawn<P: 'static, E: Send + 'static>(
		name: &str,
		initialize: impl FnOnce() -> Result<P, E> + Send + 'static,
		serve: fn(&mut P, Q, &[u8]) -> R,
	) -> Result<Self, GpuWorkerSpawnError<E>> {
		let (sender, receiver) = mpsc::sync_channel::<Option<BorrowedRequest<Q>>>(1);
		let (response_sender, responses) = mpsc::sync_channel(1);
		let (startup, startup_receiver) = mpsc::sync_channel(1);
		let worker = std::thread::Builder::new()
			.name(name.to_string())
			.spawn(move || {
				let mut processor = match initialize() {
					Ok(processor) => {
						let _ = startup.send(Ok(()));
						processor
					}
					Err(error) => {
						let _ = startup.send(Err(error));
						return;
					}
				};

				while let Ok(Some(request)) = receiver.recv() {
					// SAFETY: `GpuWorker::call` blocks until this response arrives or the worker disconnects, so the
					// borrowed bytes stay live and immutable for the reconstructed slice's full use.
					let bytes = unsafe { std::slice::from_raw_parts(request.bytes, request.len) };
					if response_sender
						.send(serve(&mut processor, request.parameters, bytes))
						.is_err()
					{
						return;
					}
				}
			})
			.map_err(GpuWorkerSpawnError::WorkerCreation)?;

		match startup_receiver.recv() {
			Ok(Ok(())) => Ok(Self {
				sender,
				responses: Mutex::new(responses),
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

	/// Serves one request on the worker and waits for its result. Returns `None` when the worker has stopped.
	pub(crate) fn call(&self, parameters: Q, bytes: &[u8]) -> Option<R> {
		// One outstanding round trip matches the single worker and avoids allocating a response channel per request.
		let responses = self.responses.lock().ok()?;
		self.sender
			.send(Some(BorrowedRequest {
				parameters,
				bytes: bytes.as_ptr(),
				len: bytes.len(),
			}))
			.ok()?;
		responses.recv().ok()
	}

	/// Creates a worker whose thread already stopped, so every call reports it as unavailable.
	#[cfg(all(test, feature = "gpu-ibl"))]
	pub(crate) fn unavailable() -> Self {
		let (sender, _) = mpsc::sync_channel(1);
		let (_, responses) = mpsc::sync_channel(1);
		Self {
			sender,
			responses: Mutex::new(responses),
			worker: None,
		}
	}
}

impl<Q, R> Drop for GpuWorker<Q, R> {
	fn drop(&mut self) {
		let _ = self.sender.send(None);
		if let Some(worker) = self.worker.take() {
			let _ = worker.join();
		}
	}
}

/// The `BorrowedRequest` struct carries borrowed bytes to the worker while the caller waits for the response.
struct BorrowedRequest<Q> {
	parameters: Q,
	bytes: *const u8,
	len: usize,
}

// SAFETY: `GpuWorker::call` does not return until the worker responds or disconnects. The immutable bytes therefore
// outlive every worker access, and no mutable reference can coexist with the caller's shared borrow.
unsafe impl<Q: Send> Send for BorrowedRequest<Q> {}

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

/// The `ComputeKernelError` enum reports why a native compute kernel could not become a pipeline.
pub(crate) enum ComputeKernelError {
	Compilation(String),
	Creation,
}

/// Compiles one native compute kernel that samples [`SOURCE_SLOT`] and writes [`OUTPUT_SLOT`], and builds its pipeline.
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
