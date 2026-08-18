use super::*;

pub mod queue {
	use std::cell::{Cell, RefCell};
	use std::ops::Deref;
	use std::ptr::NonNull;
	use std::rc::{Rc, Weak};
	use std::sync::{Arc, Condvar, Mutex, MutexGuard};

	use block2::RcBlock;
	use objc2::runtime::AnyObject;
	use objc2_foundation::NSString;
	use objc2_metal::{
		MTL4CommandAllocator, MTL4CommandBuffer, MTL4CommandEncoder, MTL4CommandQueue, MTL4CommitFeedback, MTLDevice,
		MTLResidencySet, MTLSharedEvent,
	};

	use super::*;

	/// The `CommandPool` struct provides context-local reuse for completed Metal 4 native commands.
	struct CommandPool {
		commands: RefCell<Vec<NativeCommand>>,
	}

	type CommitFeedbackHandler = RcBlock<dyn Fn(NonNull<ProtocolObject<dyn mtl::MTL4CommitFeedback>>)>;

	/// The `BatchCommitFeedback` struct provides one thread-safe completion result for every command in a committed batch.
	struct BatchCommitFeedback {
		status: Mutex<BatchCommitFeedbackStatus>,
		completed: Condvar,
	}

	enum BatchCommitFeedbackStatus {
		Pending,
		Succeeded,
		Failed(String),
		HandlerFailed,
	}

	/// The `CommandCommitFeedback` struct keeps one batch's callback objects alive until a command observes its result.
	#[derive(Clone)]
	struct CommandCommitFeedback {
		state: Arc<BatchCommitFeedback>,
		_options: Retained<mtl::MTL4CommitOptions>,
		_handler: CommitFeedbackHandler,
	}

	impl BatchCommitFeedback {
		fn new() -> Self {
			Self {
				status: Mutex::new(BatchCommitFeedbackStatus::Pending),
				completed: Condvar::new(),
			}
		}

		// Publishes the first callback result and wakes every command waiting on the batch.
		fn complete(&self, result: BatchCommitFeedbackStatus) {
			let mut status = self.lock_status();
			if matches!(*status, BatchCommitFeedbackStatus::Pending) {
				*status = result;
				self.completed.notify_all();
			}
		}

		// Waits for commit feedback and returns any GPU failure after Metal finishes the batch.
		fn wait_error(&self) -> Option<String> {
			let mut status = self.lock_status();
			while matches!(*status, BatchCommitFeedbackStatus::Pending) {
				status = match self.completed.wait(status) {
					Ok(status) => status,
					Err(poisoned) => poisoned.into_inner(),
				};
			}

			match &*status {
				BatchCommitFeedbackStatus::Succeeded => None,
				BatchCommitFeedbackStatus::Failed(error) => Some(format!(
					"Metal 4 GPU execution failed: {error}. The most likely cause is that the submitted batch used invalid GPU commands, resources, or state."
				)),
				BatchCommitFeedbackStatus::HandlerFailed => Some(String::from(
					"Metal 4 commit feedback failed. The most likely cause is that Metal returned invalid feedback data or the feedback handler encountered an unexpected failure.",
				)),
				BatchCommitFeedbackStatus::Pending => unreachable!(),
			}
		}

		// Recovers poisoned state because reporting a stored GPU failure intentionally panics while the result is borrowed.
		fn lock_status(&self) -> MutexGuard<'_, BatchCommitFeedbackStatus> {
			match self.status.lock() {
				Ok(status) => status,
				Err(poisoned) => poisoned.into_inner(),
			}
		}
	}

	#[derive(Clone, Copy, Debug, Eq, PartialEq)]
	enum NativeCommandState {
		Idle,
		Recording,
		Executable,
		Submitted(u64),
	}

	/// The `NativeCommand` struct owns reusable Metal 4 recording state for one queue submission.
	#[derive(Clone)]
	pub(crate) struct NativeCommand {
		inner: Rc<NativeCommandInner>,
	}

	/// The `NativeCommandInner` struct keeps native objects and unretained command resources alive through GPU completion.
	struct NativeCommandInner {
		allocator: Retained<ProtocolObject<dyn mtl::MTL4CommandAllocator>>,
		command_buffer: Retained<ProtocolObject<dyn mtl::MTL4CommandBuffer>>,
		residency_set: Retained<ProtocolObject<dyn mtl::MTLResidencySet>>,
		queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
		completion_event: Retained<ProtocolObject<dyn mtl::MTLSharedEvent>>,
		next_completion_value: Rc<Cell<u64>>,
		command_pool: Weak<CommandPool>,
		state: Cell<NativeCommandState>,
		commit_feedback: RefCell<Option<CommandCommitFeedback>>,
		retained_allocations: RefCell<SmallVec<[Retained<ProtocolObject<dyn mtl::MTLAllocation>>; 32]>>,
		retained_objects: RefCell<SmallVec<[Retained<AnyObject>; 4]>>,
	}

	impl NativeCommand {
		// Creates the native objects that stay paired for the lifetime of this pooled command.
		fn new(queue: &StoredQueue) -> Self {
			let device = queue.queue.device();
			let allocator = device.newCommandAllocator().expect(
				"Metal 4 command allocator creation failed. The most likely cause is that the device ran out of command recording memory.",
			);
			let command_buffer = device.newCommandBuffer().expect(
				"Metal 4 command buffer creation failed. The most likely cause is that the device ran out of command buffer objects.",
			);
			let residency_descriptor = mtl::MTLResidencySetDescriptor::new();
			let residency_set = device.newResidencySetWithDescriptor_error(&residency_descriptor).expect(
				"Metal residency set creation failed. The most likely cause is that the device ran out of residency tracking resources.",
			);

			Self {
				inner: Rc::new(NativeCommandInner {
					allocator,
					command_buffer,
					residency_set,
					queue: queue.queue.clone(),
					completion_event: queue.completion_event.clone(),
					next_completion_value: queue.next_completion_value.clone(),
					command_pool: Rc::downgrade(&queue.command_pool),
					state: Cell::new(NativeCommandState::Idle),
					commit_feedback: RefCell::new(None),
					retained_allocations: RefCell::new(SmallVec::new()),
					retained_objects: RefCell::new(SmallVec::new()),
				}),
			}
		}

		// Starts a fresh recording cycle with the command's paired allocator and residency set.
		fn begin(&self, label: Option<&str>, debug_labels: bool) {
			assert_eq!(
				self.inner.state.get(),
				NativeCommandState::Idle,
				"Metal 4 native command reuse failed. The most likely cause is that an in-flight command was returned to the pool early.",
			);
			self.inner
				.command_buffer
				.beginCommandBufferWithAllocator(self.inner.allocator.as_ref());
			self.inner.command_buffer.useResidencySet(self.inner.residency_set.as_ref());

			#[cfg(debug_assertions)]
			if debug_labels {
				self.inner.command_buffer.setLabel(label.map(NSString::from_str).as_deref());
			}

			self.inner.state.set(NativeCommandState::Recording);
		}

		/// Returns a Metal 4 compute encoder that waits for prior work on the queue.
		pub(crate) fn compute_command_encoder(&self) -> Option<Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>>> {
			let encoder = self.inner.command_buffer.computeCommandEncoder()?;
			// Metal 4 queue order does not make writes visible across encoders without an explicit barrier.
			encoder.barrierAfterQueueStages_beforeStages_visibilityOptions(
				mtl::MTLStages::All,
				mtl::MTLStages::All,
				mtl::MTL4VisibilityOptions::Device,
			);
			Some(encoder)
		}

		/// Returns a Metal 4 render encoder that waits for prior work on the queue.
		pub(crate) fn render_command_encoder(
			&self,
			descriptor: &mtl::MTL4RenderPassDescriptor,
		) -> Option<Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>> {
			let encoder = self.inner.command_buffer.renderCommandEncoderWithDescriptor(descriptor)?;
			// The barrier also covers work in earlier command buffers from the same ordered queue batch.
			encoder.barrierAfterQueueStages_beforeStages_visibilityOptions(
				mtl::MTLStages::All,
				mtl::MTLStages::All,
				mtl::MTL4VisibilityOptions::Device,
			);
			Some(encoder)
		}

		/// Retains a Metal buffer and declares its allocation in this command's residency set.
		pub(crate) fn retain_buffer(&self, buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>) {
			let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(buffer) };
			self.retain_allocation(allocation);
		}

		/// Retains a Metal texture and declares its allocation in this command's residency set.
		pub(crate) fn retain_texture(&self, texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>) {
			let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(texture) };
			self.retain_allocation(allocation);
		}

		/// Retains a sampler referenced from a nested argument buffer until GPU completion.
		pub(crate) fn retain_sampler(&self, sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>) {
			let sampler = unsafe { Retained::cast_unchecked::<AnyObject>(sampler) };
			self.inner.retained_objects.borrow_mut().push(sampler);
		}

		/// Retains a Metal 4 argument table until every command snapshot that references it completes.
		pub(crate) fn retain_argument_table(&self, table: Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>) {
			let table = unsafe { Retained::cast_unchecked::<AnyObject>(table) };
			self.inner.retained_objects.borrow_mut().push(table);
		}

		/// Retains a drawable and its texture until the queue completion token is reached.
		pub(crate) fn retain_drawable(&self, drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>) {
			self.retain_texture(drawable.texture());
			let drawable = unsafe { Retained::cast_unchecked::<AnyObject>(drawable) };
			self.inner.retained_objects.borrow_mut().push(drawable);
		}

		/// Retains allocations collected by context-level upload encoding.
		pub(crate) fn retain_allocations(
			&self,
			allocations: impl IntoIterator<Item = Retained<ProtocolObject<dyn mtl::MTLAllocation>>>,
		) {
			for allocation in allocations {
				self.retain_allocation(allocation);
			}
		}

		fn retain_allocation(&self, allocation: Retained<ProtocolObject<dyn mtl::MTLAllocation>>) {
			let mut allocations = self.inner.retained_allocations.borrow_mut();
			if allocations
				.iter()
				.any(|retained| std::ptr::eq::<ProtocolObject<dyn mtl::MTLAllocation>>(retained.as_ref(), allocation.as_ref()))
			{
				return;
			}
			self.inner.residency_set.addAllocation(allocation.as_ref());
			allocations.push(allocation);
		}

		// Ends recording and commits residency changes before queue submission.
		fn finish(&self) {
			match self.inner.state.get() {
				NativeCommandState::Recording => {
					self.inner.residency_set.commit();
					self.inner.command_buffer.endCommandBuffer();
					self.inner.state.set(NativeCommandState::Executable);
				}
				NativeCommandState::Executable => {}
				state => panic!(
					"Metal 4 command buffer finish failed. The most likely cause is that the command was not recording. state={state:?}"
				),
			}
		}

		// Commits one queue-ordered batch and assigns the same shared-event completion token to every command.
		pub(crate) fn submit_batch(commands: &[NativeCommand]) {
			let Some(first) = commands.first() else {
				return;
			};
			let first_pool = first.inner.command_pool.upgrade().expect(
				"Metal 4 command pool is missing. The most likely cause is that a native command outlived its context queue.",
			);
			let mut command_buffers = SmallVec::<[NonNull<ProtocolObject<dyn mtl::MTL4CommandBuffer>>; 4]>::new();

			for command in commands {
				let command_pool = command.inner.command_pool.upgrade().expect(
					"Metal 4 command pool is missing. The most likely cause is that a native command outlived its context queue.",
				);
				assert!(
					Rc::ptr_eq(&first_pool, &command_pool),
					"Metal 4 command batch submission failed. The most likely cause is that command buffers from different queues were batched together.",
				);
				command.finish();
				command_buffers.push(NonNull::from(command.inner.command_buffer.as_ref()));
			}

			let completion_value = first.inner.next_completion_value.get();
			let next_completion_value = completion_value.checked_add(1).expect(
				"Metal shared-event completion token overflowed. The most likely cause is that this queue submitted u64::MAX command batches.",
			);
			first.inner.next_completion_value.set(next_completion_value);
			let command_buffer_pointer = NonNull::new(command_buffers.as_mut_ptr()).expect(
				"Metal 4 command batch pointer was null. The most likely cause is that an empty command batch reached submission.",
			);

			let feedback_state = Arc::new(BatchCommitFeedback::new());
			let callback_state = Arc::clone(&feedback_state);
			let feedback_handler: CommitFeedbackHandler =
				RcBlock::new(move |feedback: NonNull<ProtocolObject<dyn mtl::MTL4CommitFeedback>>| {
					// Metal can invoke this block on any thread, so no context-local or Objective-C state is captured.
					let callback_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
						let feedback = unsafe { feedback.as_ref() };
						let result = match feedback.error() {
							Some(error) => BatchCommitFeedbackStatus::Failed(error.localizedDescription().to_string()),
							None => BatchCommitFeedbackStatus::Succeeded,
						};
						callback_state.complete(result);
					}));
					if callback_result.is_err() {
						callback_state.complete(BatchCommitFeedbackStatus::HandlerFailed);
					}
				});
			let commit_options = mtl::MTL4CommitOptions::new();
			unsafe {
				commit_options.addFeedbackHandler(RcBlock::as_ptr(&feedback_handler));
			}
			let commit_feedback = CommandCommitFeedback {
				state: feedback_state,
				_options: commit_options.clone(),
				_handler: feedback_handler,
			};
			for command in commands {
				let previous_feedback = command.inner.commit_feedback.replace(Some(commit_feedback.clone()));
				assert!(
					previous_feedback.is_none(),
					"Metal 4 commit feedback registration failed. The most likely cause is that a native command was submitted more than once without being recycled.",
				);
			}

			unsafe {
				first
					.inner
					.queue
					.commit_count_options(command_buffer_pointer, command_buffers.len(), commit_options.as_ref());
			}
			let completion_event: &ProtocolObject<dyn mtl::MTLEvent> = first.inner.completion_event.as_ref();
			first.inner.queue.signalEvent_value(completion_event, completion_value);

			for command in commands {
				command.inner.state.set(NativeCommandState::Submitted(completion_value));
			}
		}

		// Waits for this command, resets its native state, and returns any GPU error after recycling it.
		pub(crate) fn wait_and_recycle(&self) -> Option<String> {
			let NativeCommandState::Submitted(completion_value) = self.inner.state.get() else {
				panic!(
					"Metal 4 command completion wait failed. The most likely cause is that a synchronizer retained a command before it was submitted."
				);
			};
			// Feedback completes even when a fatal queue error prevents the later shared-event signal.
			let error = {
				let commit_feedback = self.inner.commit_feedback.borrow();
				commit_feedback
					.as_ref()
					.expect(
						"Metal 4 commit feedback is missing. The most likely cause is that submitted command state was modified before completion.",
					)
					.state
					.wait_error()
			};
			if error.is_none() {
				let completed = self
					.inner
					.completion_event
					.waitUntilSignaledValue_timeoutMS(completion_value, u64::MAX);
				assert!(
					completed,
					"Metal shared-event wait failed. The most likely cause is that the GPU did not signal the submitted completion token. token={completion_value}",
				);
			}

			self.inner.allocator.reset();
			self.inner.residency_set.removeAllAllocations();
			self.inner.residency_set.commit();
			self.inner.retained_allocations.borrow_mut().clear();
			self.inner.retained_objects.borrow_mut().clear();
			self.inner.commit_feedback.borrow_mut().take();
			self.inner.state.set(NativeCommandState::Idle);

			let command_pool = self.inner.command_pool.upgrade().expect(
				"Metal 4 command recycle failed. The most likely cause is that the context queue was destroyed before its submitted work completed.",
			);
			command_pool.commands.borrow_mut().push(self.clone());
			error
		}
	}

	impl Deref for NativeCommand {
		type Target = ProtocolObject<dyn mtl::MTL4CommandBuffer>;

		fn deref(&self) -> &Self::Target {
			self.inner.command_buffer.as_ref()
		}
	}

	impl AsRef<NativeCommand> for NativeCommand {
		fn as_ref(&self) -> &NativeCommand {
			self
		}
	}

	/// The `StoredQueue` struct owns one Metal 4 queue and its context-local native command pool.
	pub(crate) struct StoredQueue {
		pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
		pub(crate) workloads: crate::WorkloadTypes,
		completion_event: Retained<ProtocolObject<dyn mtl::MTLSharedEvent>>,
		next_completion_value: Rc<Cell<u64>>,
		command_pool: Rc<CommandPool>,
	}

	impl StoredQueue {
		pub(crate) fn new(queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>, workloads: crate::WorkloadTypes) -> Self {
			let completion_event = queue.device().newSharedEvent().expect(
				"Metal shared event creation failed. The most likely cause is that the device ran out of synchronization resources.",
			);
			Self {
				queue,
				workloads,
				completion_event,
				next_completion_value: Rc::new(Cell::new(1)),
				command_pool: Rc::new(CommandPool {
					commands: RefCell::new(Vec::new()),
				}),
			}
		}

		/// Acquires a reset native command and begins recording with its paired allocator.
		pub(crate) fn acquire_native_command(&self, label: Option<&str>, debug_labels: bool) -> NativeCommand {
			let command = self
				.command_pool
				.commands
				.borrow_mut()
				.pop()
				.unwrap_or_else(|| NativeCommand::new(self));
			command.begin(label, debug_labels);
			command
		}
	}

	impl Clone for StoredQueue {
		fn clone(&self) -> Self {
			// Every Context gets an independent completion timeline and native command pool.
			Self::new(self.queue.clone(), self.workloads)
		}
	}

	/// The `Queue` struct owns the queue submission entry point without borrowing the device.
	pub struct Queue {
		pub(crate) device: std::ptr::NonNull<context::Context>,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	// Device::get_queue transfers one logical queue to one runtime owner; callers must not use the Context concurrently.
	unsafe impl Send for Queue {}

	/// The `QueueReference` struct preserves the borrowed queue API while queue ownership is being split out.
	pub struct QueueReference<'a> {
		pub(crate) device: &'a mut context::Context,
		pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
	}

	/// The `Execution` struct gathers Metal command-buffer recordings before one batched queue submission.
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
			index: u64,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			let queue_handle = self.queue_handle;
			self.device_mut().start_frame(index, synchronizer_handle, queue_handle)
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
			let queue_handle = self.queue_handle;
			let device = self.device_mut();
			for &wait_synchronizer in wait_for {
				device.wait_for_synchronizer(wait_synchronizer);
			}

			let frame = frame.map(|frame| device.start_frame(frame.index, frame.synchronizer, queue_handle));
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
			frame.execute_finished_batch(execution.command_buffers, present_keys.as_ref(), synchronizer);
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
			index: u64,
			synchronizer_handle: graphics_hardware_interface::SynchronizerHandle,
		) -> crate::queue::StartedFrame<Self::Frame<'a>> {
			self.device.start_frame(index, synchronizer_handle, self.queue_handle)
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

			let queue_handle = self.queue_handle;
			let frame = frame.map(|frame| self.device.start_frame(frame.index, frame.synchronizer, queue_handle));
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
			frame.execute_finished_batch(execution.command_buffers, present_keys.as_ref(), synchronizer);
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
		workloads: RefCell<SmallVec<[queue::NativeCommand; 4]>>,
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

		pub(crate) fn signal_workload(&self, command: queue::NativeCommand) {
			self.signaled.set(false);
			self.workloads.borrow_mut().push(command);
		}

		/// Retains every command in one submitted batch until the shared-event completion token is reached.
		pub(crate) fn signal_workloads(&self, commands: impl IntoIterator<Item = queue::NativeCommand>) {
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
		pub extent: Extent,
	}
}
