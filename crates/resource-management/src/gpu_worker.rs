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

/// The slot every offline kernel writes its output image or buffer to.
pub(crate) const OUTPUT_SLOT: ghi::ResourceSlot = ghi::ResourceSlot::new(1);

/// The `GpuWorker` struct lets asset handlers on the shared worker pool use a GHI processor that must stay on one thread.
///
/// Spawn it with the processor factory and the function that serves one request, then call [`Self::call`] once per
/// image. `Q` is the request's parameters and `R` its result. A request can also lend the worker an output slice, so
/// a processor can write its result straight into the caller's storage.
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
	/// whole lifetime, and each request runs `serve` with the caller's parameters, borrowed input bytes, and borrowed
	/// output bytes.
	pub(crate) fn spawn<P: 'static, E: Send + 'static>(
		name: &str,
		initialize: impl FnOnce() -> Result<P, E> + Send + 'static,
		serve: fn(&mut P, Q, &[u8], &mut [u8]) -> R,
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
					// borrowed input stays live and immutable for the reconstructed slice's full use.
					let input = unsafe { std::slice::from_raw_parts(request.input, request.input_len) };
					// SAFETY: the same wait keeps the caller's exclusive output borrow live, and the caller can't touch
					// it until `call` returns, so this is the only reference to those bytes.
					let output = unsafe { std::slice::from_raw_parts_mut(request.output, request.output_len) };
					if response_sender
						.send(serve(&mut processor, request.parameters, input, output))
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
	///
	/// The worker reads `input` and may write any part of `output` before it responds. Pass an empty `output` when the
	/// result carries everything.
	pub(crate) fn call(&self, parameters: Q, input: &[u8], output: &mut [u8]) -> Option<R> {
		// One outstanding round trip matches the single worker and avoids allocating a response channel per request.
		let responses = self.responses.lock().ok()?;
		self.sender
			.send(Some(BorrowedRequest {
				parameters,
				input: input.as_ptr(),
				input_len: input.len(),
				output: output.as_mut_ptr(),
				output_len: output.len(),
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

/// The `BorrowedRequest` struct carries borrowed input and output bytes to the worker while the caller waits for the
/// response.
struct BorrowedRequest<Q> {
	parameters: Q,
	input: *const u8,
	input_len: usize,
	output: *mut u8,
	output_len: usize,
}

// SAFETY: `GpuWorker::call` does not return until the worker responds or disconnects. Both slices therefore outlive
// every worker access. The caller holds a shared borrow of the input, so nothing mutates it, and an exclusive borrow of
// the output, so the worker's slice is the only way to reach those bytes.
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
