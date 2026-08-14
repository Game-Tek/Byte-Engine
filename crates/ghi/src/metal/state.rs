use super::*;

pub mod queue {
	use super::*;

	#[derive(Clone)]
	pub(crate) struct StoredQueue {
		pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTLCommandQueue>>,
		pub(crate) workloads: crate::WorkloadTypes,
	}

	/// The `Queue` struct owns the queue submission entry point without borrowing the device.
	pub struct Queue {
		pub(crate) device: std::ptr::NonNull<context::Context>,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	unsafe impl Send for Queue {}

	/// The `QueueReference` struct preserves the borrowed queue API while queue ownership is being split out.
	pub struct QueueReference<'a> {
		pub(crate) device: &'a mut context::Context,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	/// The `Execution` struct gathers Metal command-buffer recordings before queue submission.
	pub struct Execution<'a> {
		frame: Option<super::Frame<'a>>,
		completed_frame: Option<graphics_hardware_interface::FrameKey>,
		command_buffers: SmallVec<[super::FinishedCommandBuffer<'static>; 4]>,
	}

	impl<'a> crate::queue::QueueExecution<'a> for Execution<'a> {
		type Frame = super::Frame<'a>;

		fn frame(&mut self) -> Option<&mut Self::Frame> {
			self.frame.as_mut()
		}

		fn completed_frame(&self) -> Option<graphics_hardware_interface::FrameKey> {
			self.completed_frame
		}

		fn record<'record>(
			&'record mut self,
			command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
			record: impl FnOnce(&mut <Self::Frame as crate::frame::Frame<'a>>::CBR<'record>),
		) where
			Self::Frame: 'record,
		{
			let frame = self.frame.as_mut().expect(
				"Frame is required to record a frame command buffer. The most likely cause is that Queue::execute was called with None and the closure tried to record frame work.",
			);
			let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
			record(&mut command_buffer);
			self.command_buffers.push(command_buffer.into_finished());
		}
	}

	impl Queue {
		/// Returns mutable device access for the queue wrapper until device state is split out.
		fn device_mut(&mut self) -> &mut context::Context {
			// The owned queue is created from a live Device and must not outlive it.
			// Thread-safe ownership will require moving queue-local state out of Device.
			unsafe { self.device.as_mut() }
		}
	}

	impl crate::queue::Queue for Queue {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			let queue_handle = self.queue_handle;
			self.device_mut().create_command_buffer(name, queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device_mut().start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			let device = self.device_mut();
			for &wait_synchronizer in wait_for {
				device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: SmallVec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}

	impl crate::queue::Queue for QueueReference<'_> {
		type Frame<'a> = super::Frame<'a>;
		type Execution<'a> = Execution<'a>;

		fn create_command_buffer(&mut self, name: Option<&str>) -> graphics_hardware_interface::CommandBufferHandle {
			self.device.create_command_buffer(name, self.queue_handle)
		}

		fn start_frame<'a>(
			&'a mut self,
			index: u32,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device.start_frame(index, synchronizer_handle)
		}

		fn execute<'a, P>(
			&'a mut self,
			frame: Option<crate::queue::FrameRequest>,
			wait_for: &[graphics_hardware_interface::SynchronizerHandle],
			synchronizer: graphics_hardware_interface::SynchronizerHandle,
			execute: impl FnOnce(&mut Self::Execution<'a>) -> P,
		) where
			P: AsRef<[graphics_hardware_interface::PresentKey]>,
		{
			for &wait_synchronizer in wait_for {
				self.device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| self.device.start_frame(frame.index, frame.synchronizer));
			let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
			let frame = frame.map(|frame| frame.frame);
			let mut execution = Execution {
				frame,
				completed_frame,
				command_buffers: SmallVec::new(),
			};
			let present_keys = execute(&mut execution);

			let Some(mut frame) = execution.frame else {
				return;
			};
			let last_index = execution.command_buffers.len().saturating_sub(1);
			for (index, command_buffer) in execution.command_buffers.into_iter().enumerate() {
				let present_keys = if index == last_index { present_keys.as_ref() } else { &[] };
				frame.execute_finished(command_buffer, present_keys, synchronizer);
			}
		}
	}
}

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
		workloads: RefCell<SmallVec<[Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>; 4]>>,
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
			// Reset only after previous work is complete so diagnostics are not lost for in-flight submissions.
			self.wait();
			self.signaled.set(false);
		}

		pub(crate) fn signal_workload(&self, command_buffer: Retained<ProtocolObject<dyn mtl::MTLCommandBuffer>>) {
			self.signaled.set(false);
			self.workloads.borrow_mut().push(command_buffer);
		}

		pub(crate) fn wait(&self) {
			if self.signaled.get() {
				return;
			}

			// Retain the command buffers until completion so asynchronous Metal submissions can be diagnosed later.
			let workloads = self.workloads.take();
			for command_buffer in &workloads {
				device::wait_for_metal_command_buffer(command_buffer.as_ref());
			}

			self.signaled.set(true);
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
		pub extent: Extent,
	}
}
