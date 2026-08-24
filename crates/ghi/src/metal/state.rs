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
	use std::cell::{Cell, RefCell};

	use super::*;
	use crate::synchronizer::SynchronizerHandle;

	/// The `Synchronizer` struct owns the Metal workloads associated with one GHI synchronization point.
	pub(crate) struct Synchronizer {
		pub next: Option<SynchronizerHandle>,
		signaled: Cell<bool>,
		workloads: RefCell<SmallVec<[crate::metal::queue::NativeCommand; 4]>>,
	}

	impl Synchronizer {
		pub(crate) fn new(signaled: bool) -> Self {
			Self {
				next: None,
				signaled: Cell::new(signaled),
				workloads: RefCell::new(SmallVec::new()),
			}
		}

		pub(crate) fn reset(&self) {
			// Reset only after prior tokens complete so native allocators and residency sets are safe to reuse.
			self.wait();
			self.signaled.set(false);
		}

		pub(crate) fn signal_workload(&self, command: crate::metal::queue::NativeCommand) {
			self.signaled.set(false);
			self.workloads.borrow_mut().push(command);
		}

		/// Retains every command in one submitted batch until the shared-event completion token is reached.
		pub(crate) fn signal_workloads(&self, commands: impl IntoIterator<Item = crate::metal::queue::NativeCommand>) {
			self.signaled.set(false);
			self.workloads.borrow_mut().extend(commands);
		}

		pub(crate) fn wait(&self) {
			if self.signaled.get() {
				return;
			}

			let workloads = self.workloads.take();
			let mut first_error = None;
			for command in &workloads {
				if let Some(error) = command.wait_and_recycle() {
					first_error.get_or_insert(error);
				}
			}

			self.signaled.set(true);
			if let Some(error) = first_error {
				panic!("{error}");
			}
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
