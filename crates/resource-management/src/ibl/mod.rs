//! Generate and store image-based lighting resources from decoded HDR images.

pub mod cpu;
#[cfg(feature = "gpu-ibl")]
pub mod gpu;
#[cfg(feature = "gpu-ibl")]
mod gpu_shaders;

/// The `IBLGenerator` struct provides reusable CPU or GPU image-based lighting generation for decoded HDR images.
///
/// Pass this generator to an environment-map asset handler after choosing the desired GPU setup. GPU generation
/// automatically falls back to the CPU implementation when an individual bake fails.
#[derive(Default)]
pub struct IBLGenerator {
	/// The worker thread that owns the GPU processor, so asset handlers on the shared pool await bakes instead of
	/// blocking a pool thread.
	#[cfg(feature = "gpu-ibl")]
	gpu_worker: Option<GpuWorker<gpu::GPUIBLProcessor>>,
}

impl IBLGenerator {
	/// Creates an IBL generator that always uses the CPU implementation.
	pub fn new() -> Self {
		Self::default()
	}

	/// Creates an IBL generator whose GPU processor is initialized on its dedicated worker thread.
	///
	/// Create thread-affine GHI state inside `initialize`; captured values must be safe to move to the worker. Setup errors
	/// are returned so you can select [`Self::new`].
	#[cfg(feature = "gpu-ibl")]
	pub fn with_gpu_processor_factory(
		initialize: impl FnOnce() -> Result<gpu::GPUIBLProcessor, gpu::GPUIBLBakeError> + Send + 'static,
	) -> Result<Self, gpu::GPUIBLBakeError> {
		Ok(Self {
			gpu_worker: Some(GpuWorker::spawn("GPU Environment Map Worker", initialize)?),
		})
	}

	/// Creates an IBL generator from a worker-local GHI context factory.
	///
	/// Build the context inside `initialize` so a non-`Send` backend context never crosses a thread boundary. Return an owner
	/// guard that keeps its device and instance alive; the queue must support compute and transfer work.
	#[cfg(feature = "gpu-ibl")]
	pub fn with_gpu_context<Owner: 'static>(
		initialize: impl FnOnce() -> Result<(ghi::implementation::Context, ghi::QueueHandle, Owner), gpu::GPUIBLBakeError>
		+ Send
		+ 'static,
	) -> Result<Self, gpu::GPUIBLBakeError> {
		Self::with_gpu_processor_factory(move || {
			let (context, queue, owner) = initialize()?;

			gpu::GPUIBLProcessor::from_context(context, queue, owner)
		})
	}

	/// Creates an IBL generator with a self-contained GHI device and context for offline baking.
	///
	/// Use [`Self::new`] when deterministic CPU-only baking is required or when this constructor reports a setup error.
	#[cfg(feature = "gpu-ibl")]
	pub fn try_with_default_gpu() -> Result<Self, gpu::GPUIBLBakeError> {
		Self::with_gpu_processor_factory(gpu::GPUIBLProcessor::try_new)
	}

	/// Generates IBL textures from one decoded RGBA16F image and stores the complete image resource.
	pub async fn generate_and_store(
		&self,
		context: BakeContext<'_>,
		url: ResourceId<'_>,
		extent: Extent,
		rgba16f: &[u8],
	) -> Result<(), LoadErrors> {
		#[cfg(feature = "gpu-ibl")]
		if let Some(worker) = &self.gpu_worker {
			// The worker owns a copy of the source while the bake is in flight. The baked maps come back in the result.
			let baked = (worker.submit(extent, rgba16f.to_vec()).await)
				.unwrap_or_else(|_| Err(crate::GpuWorkerError::Unavailable.into()));
			match baked {
				Ok(baked) => {
					context.info("Generated environment maps on the GPU.");
					return store_baked_image(context, url, baked).await;
				}
				Err(error) => context.warn(format!(
					"GPU environment-map generation failed; using the CPU fallback. The most likely cause is an unavailable or unsupported GPU path. Error: {error}"
				)),
			}
		}

		let baked = bake_image_ibl_in(extent, rgba16f, context.allocator()).map_err(|error| {
			context.error(format!(
				"Environment-map generation failed. The most likely cause is invalid HDR image dimensions or insufficient processing memory. Error: {error}"
			));
			LoadErrors::FailedToProcess
		})?;

		store_baked_image(context, url, baked).await
	}

	#[cfg(all(test, feature = "gpu-ibl"))]
	pub(crate) fn unavailable_for_test() -> Self {
		Self {
			gpu_worker: Some(GpuWorker::unavailable()),
		}
	}
}

/// Stores a decoded HDR image and its generated IBL streams without changing processor-owned data.
async fn store_baked_image(
	context: BakeContext<'_>,
	url: ResourceId<'_>,
	baked: BakedImageIBL<impl Allocator>,
) -> Result<(), LoadErrors> {
	let image = Image {
		format: Formats::RGBA16F,
		gamma: Gamma::Linear,
		extent: baked.root_extent,
		mip_count: 1,
		ibl: Some(baked.ibl),
		photometry: None,
	};

	let asset = ProcessedAsset::new(url, image).with_streams(baked.streams);

	context.store_primary(asset, &baked.data).await
}

use std::alloc::Allocator;

use utils::Extent;

#[cfg(feature = "gpu-ibl")]
use crate::gpu_worker::GpuWorker;
use crate::{
	BakeContext, ProcessedAsset,
	asset::{ResourceId, handler::LoadErrors},
	ibl::cpu::{BakedImageIBL, bake_image_ibl_in},
	resources::image::Image,
	types::{Formats, Gamma},
};
