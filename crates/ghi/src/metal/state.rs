use super::*;

pub mod buffer {
	use super::*;
	use crate::{DeviceAccesses, Uses};

	#[derive(Clone)]
	pub(crate) struct Buffer {
		pub(crate) name: Option<String>,
		pub(crate) staging: Option<BufferHandle>,
		pub(crate) buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>,
		pub(crate) size: usize,
		pub(crate) gpu_address: u64,
		pub(crate) pointer: *mut u8,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
	}
}

pub mod image {
	use super::*;
	use crate::{DeviceAccesses, Formats, Uses};

	#[derive(Clone)]
	pub(crate) struct Image {
		pub(crate) name: Option<String>,
		pub(crate) texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>,
		pub(crate) extent: Extent,
		pub(crate) format: Formats,
		pub(crate) uses: Uses,
		pub(crate) access: DeviceAccesses,
		pub(crate) array_layers: u32,
		pub(crate) cube_compatible: bool,
		pub(crate) cube_array_compatible: bool,
		pub(crate) mip_levels: u32,
		pub(crate) staging: Option<Vec<u8>>,
	}
}

pub mod sampler {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct Sampler {
		pub(crate) sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>,
	}
}

pub mod descriptor_set {
	use super::*;
	use crate::descriptors::DescriptorSetHandle;

	/// The `DescriptorSet` struct provides Metal descriptor state for one frame.
	#[derive(Clone)]
	pub(crate) struct DescriptorSet {
		pub next: Option<DescriptorSetHandle>,
		pub version: u64,
		pub descriptors: HashMap<crate::shader::ResourceSlot, HashMap<u32, Descriptor>>,
	}
}

pub mod synchronizer {
	use super::*;
	use crate::synchronizer::SynchronizerHandle;

	/// The `Synchronizer` struct owns the Metal workloads associated with one GHI synchronization point.
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		signaled: bool,
		workloads: SmallVec<[crate::metal::queue::SubmittedBatch; 4]>,
	}

	impl Synchronizer {
		pub(crate) fn new(signaled: bool) -> Self {
			Self {
				next: None,
				signaled,
				workloads: SmallVec::new(),
			}
		}

		pub(crate) fn reset(&mut self) {
			assert!(
				self.signaled,
				"Metal synchronizer reset failed. The most likely cause is that its previous workloads were not completed first.",
			);
			self.signaled = false;
		}

		pub(crate) fn signal(&mut self, workload: crate::metal::queue::SubmittedBatch) {
			self.signaled = false;
			self.workloads.push(workload);
		}

		/// Waits for every submitted batch and returns commands to their owning context for recycling.
		pub(crate) fn wait(
			&mut self,
		) -> (
			SmallVec<
				[(
					graphics_hardware_interface::QueueHandle,
					SmallVec<[crate::metal::queue::NativeCommand; 4]>,
				); 4],
			>,
			Option<String>,
		) {
			if self.signaled {
				return (SmallVec::new(), None);
			}

			let workloads = std::mem::take(&mut self.workloads);
			let mut completed = SmallVec::new();
			let mut first_error = None;
			for workload in workloads {
				let (queue, commands, error) = workload.wait();
				completed.push((queue, commands));
				if let Some(error) = error {
					first_error.get_or_insert(error);
				}
			}

			self.signaled = true;
			(completed, first_error)
		}
	}
}

pub mod swapchain {
	use super::*;
	use crate::image::ImageHandle;

	#[derive(Clone)]
	pub(crate) struct Swapchain {
		pub layer: Retained<CAMetalLayer>,
		pub view: Retained<NSView>,
		/// Proxy images exist only when the declared uses cannot be applied to a drawable texture.
		pub images: [Option<ImageHandle>; MAX_SWAPCHAIN_IMAGES],
		pub uses_proxy: bool,
		pub uses: crate::Uses,
		pub extent: Extent,
	}
}
