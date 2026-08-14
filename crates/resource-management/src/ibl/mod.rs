//! Generate and store image-based lighting resources from decoded HDR images.

pub mod cpu;
#[cfg(feature = "gpu-ibl")]
pub mod gpu;
#[cfg(feature = "gpu-ibl")]
mod gpu_shaders;

/// The `IBLGenerator` struct provides reusable CPU or GPU image-based lighting generation for decoded HDR images.
///
/// Pass this generator to an image asset handler after choosing the desired GPU setup. GPU generation automatically falls
/// back to the CPU implementation when an individual bake fails.
pub struct IBLGenerator {
	#[cfg(feature = "gpu-ibl")]
	gpu_client: Option<gpu::GPUIBLClient>,
}

impl Default for IBLGenerator {
	fn default() -> Self {
		Self::new()
	}
}

impl IBLGenerator {
	/// Creates an IBL generator that always uses the CPU implementation.
	pub fn new() -> Self {
		Self {
			#[cfg(feature = "gpu-ibl")]
			gpu_client: None,
		}
	}

	/// Creates an IBL generator whose GPU processor is initialized on its dedicated worker thread.
	///
	/// Create thread-affine GHI state inside `initialize`; captured values must be safe to move to the worker. Setup errors
	/// are returned so you can select [`Self::new`].
	#[cfg(feature = "gpu-ibl")]
	pub fn with_gpu_processor_factory(
		initialize: impl FnOnce() -> Result<gpu::GPUIBLProcessor, gpu::GPUIBLBakeError> + Send + 'static,
	) -> Result<Self, gpu::GPUIBLBakeError> {
		gpu::GPUIBLClient::from_processor_factory(initialize).map(|gpu_client| Self {
			gpu_client: Some(gpu_client),
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
	pub fn generate_and_store(
		&self,
		context: BakeContext<'_>,
		url: ResourceId<'_>,
		extent: Extent,
		rgba16f: &[u8],
	) -> Result<(), LoadErrors> {
		#[cfg(feature = "gpu-ibl")]
		if let Some(client) = &self.gpu_client {
			match client.bake_image_ibl(extent, rgba16f) {
				Ok(baked) => {
					context.info("Generated environment maps on the GPU.");
					return store_baked_image(
						context,
						url,
						baked.root_extent,
						baked.ibl,
						baked.streams,
						&baked.data,
					);
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
		store_baked_image(context, url, baked.root_extent, baked.ibl, baked.streams, &baked.data)
	}

	#[cfg(all(test, feature = "gpu-ibl"))]
	pub(crate) fn unavailable_for_test() -> Self {
		Self {
			gpu_client: Some(gpu::GPUIBLClient::unavailable_for_test()),
		}
	}
}

/// Stores a decoded HDR image and its generated IBL streams without changing processor-owned data.
fn store_baked_image(
	context: BakeContext<'_>,
	url: ResourceId<'_>,
	root_extent: [u32; 3],
	ibl: ImageIBL,
	streams: Vec<StreamDescription>,
	data: &[u8],
) -> Result<(), LoadErrors> {
	let image = Image {
		format: Formats::RGBA16F,
		gamma: Gamma::Linear,
		extent: root_extent,
		mip_count: 1,
		ibl: Some(ibl),
		photometry: None,
	};
	let asset = ProcessedAsset::new(url, image).with_streams(streams);
	context.store_primary(asset, data)
}

use utils::Extent;

use crate::{
	asset::{asset_handler::LoadErrors, ResourceId},
	ibl::cpu::bake_image_ibl_in,
	resources::image::{Image, ImageIBL},
	types::{Formats, Gamma},
	BakeContext, ProcessedAsset, StreamDescription,
};
