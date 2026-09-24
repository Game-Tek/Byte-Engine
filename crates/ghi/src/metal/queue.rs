enum BatchCommitFeedbackStatus {
	Succeeded,
	Failed(String),
	HandlerFailed,
}

/// The `NativeCommand` struct owns reusable Metal 4 recording state for one queue submission.
pub(crate) struct NativeCommand {
	allocator: Retained<ProtocolObject<dyn mtl::MTL4CommandAllocator>>,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTL4CommandBuffer>>,
	/// Allocations this recording uses. Submission hands them to the queue's [`QueueResidency`], which keeps them
	/// resident and alive until the GPU finishes with them.
	retained_allocations: SmallVec<[Retained<ProtocolObject<dyn mtl::MTLAllocation>>; 32]>,
	retained_addresses: ::utils::hash::HashSet<usize>,
	retained_objects: SmallVec<[Retained<ProtocolObject<dyn NSObjectProtocol>>; 4]>,
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

		Self {
			allocator,
			command_buffer,
			retained_allocations: SmallVec::new(),
			retained_addresses: ::utils::hash::HashSet::default(),
			retained_objects: SmallVec::new(),
		}
	}

	// Starts a fresh recording cycle with the command's paired allocator and the queue's residency set.
	fn begin(&mut self, residency_set: &ProtocolObject<dyn mtl::MTLResidencySet>, label: Option<&str>, debug_labels: bool) {
		self.command_buffer.beginCommandBufferWithAllocator(self.allocator.as_ref());
		self.command_buffer.useResidencySet(residency_set);

		#[cfg(debug_assertions)]
		if debug_labels {
			self.command_buffer.setLabel(label.map(NSString::from_str).as_deref());
		}
	}

	/// Retains a buffer, texture, pipeline state, or acceleration structure until GPU completion and makes it
	/// resident when this command is submitted.
	pub(crate) fn retain_allocation<T: Message + 'static>(&mut self, allocation: Retained<T>)
	where
		dyn mtl::MTLAllocation: ImplementedBy<T>,
	{
		let allocation = ProtocolObject::<dyn mtl::MTLAllocation>::from_retained(allocation);
		// Commands retain the same buffers and textures once per encoder; a set keeps repeats constant time.
		let address = Retained::as_ptr(&allocation) as *const () as usize;
		if !self.retained_addresses.insert(address) {
			return;
		}
		self.retained_allocations.push(allocation);
	}

	/// Retains an object that needs no residency, such as a sampler or an argument table, until GPU completion.
	pub(crate) fn retain_object<T: Message + 'static>(&mut self, object: Retained<T>)
	where
		dyn NSObjectProtocol: ImplementedBy<T>,
	{
		self.retained_objects.push(ProtocolObject::from_retained(object));
	}

	/// Retains a drawable and its texture until Metal completes the submitted batch.
	pub(crate) fn retain_drawable(&mut self, drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>) {
		self.retain_allocation(drawable.texture());
		self.retain_object(drawable);
	}

	// Ends recording before queue submission.
	fn finish(&mut self) {
		self.command_buffer.endCommandBuffer();
	}

	// Resets native recording state after the owning submitted batch completes.
	fn reset(&mut self) {
		self.allocator.reset();
		self.retained_allocations.clear();
		self.retained_addresses.clear();
		self.retained_objects.clear();
	}
}

/// The `QueueResidency` struct keeps one long-lived residency set per queue, so frames that reuse the same resources
/// neither rebuild nor recommit residency.
///
/// Submission adds only the allocations the set does not hold yet. An allocation leaves the set once every batch that
/// used it has completed. The queue holds a strong reference to each resident allocation, which keeps it alive while
/// the GPU may still use it and keeps its address unique as a key.
struct QueueResidency {
	set: Retained<ProtocolObject<dyn mtl::MTLResidencySet>>,
	/// Resident allocations by object address, with the last batch that used each one.
	resident: ::utils::hash::HashMap<usize, (Retained<ProtocolObject<dyn mtl::MTLAllocation>>, u64)>,
	/// Submitted batches in submission order, with the addresses each one was the latest user of when submitted.
	in_flight: std::collections::VecDeque<InFlightBatch>,
	/// Address lists of retired batches, reused by later submissions so steady frames do not allocate.
	spare_addresses: Vec<Vec<usize>>,
	/// Allocations removed from the set but not yet committed as removed. They stay alive until that commit.
	removed: Vec<Retained<ProtocolObject<dyn mtl::MTLAllocation>>>,
	next_batch: u64,
}

/// The `InFlightBatch` struct records which resident allocations one submitted batch used last.
struct InFlightBatch {
	batch: u64,
	completed: bool,
	addresses: Vec<usize>,
}

impl QueueResidency {
	fn new(device: &ProtocolObject<dyn MTLDevice>) -> Self {
		let descriptor = mtl::MTLResidencySetDescriptor::new();
		let set = device.newResidencySetWithDescriptor_error(&descriptor).expect(
			"Metal residency set creation failed. The most likely cause is that the device ran out of residency tracking resources.",
		);
		Self {
			set,
			resident: ::utils::hash::HashMap::default(),
			in_flight: std::collections::VecDeque::new(),
			spare_addresses: Vec::new(),
			removed: Vec::new(),
			next_batch: 0,
		}
	}

	/// Makes every allocation the batch's commands retained resident, commits the set if it changed, and returns
	/// the new batch's number.
	///
	/// Call it before committing the commands, since Metal needs the set committed before work that relies on it.
	fn admit(&mut self, commands: &mut [NativeCommand]) -> u64 {
		let batch = self.next_batch;
		self.next_batch += 1;
		let mut addresses = self.spare_addresses.pop().unwrap_or_default();
		let mut added = false;
		for command in commands {
			for allocation in command.retained_allocations.drain(..) {
				let address = Retained::as_ptr(&allocation) as *const () as usize;
				match self.resident.entry(address) {
					std::collections::hash_map::Entry::Occupied(mut entry) => {
						// Commands in one batch can share an allocation; list it once per batch.
						if entry.get().1 != batch {
							entry.get_mut().1 = batch;
							addresses.push(address);
						}
					}
					std::collections::hash_map::Entry::Vacant(entry) => {
						self.set.addAllocation(allocation.as_ref());
						entry.insert((allocation, batch));
						addresses.push(address);
						added = true;
					}
				}
			}
		}
		if added || !self.removed.is_empty() {
			self.set.commit();
			// The commit published the removals, so the GPU no longer references these allocations.
			self.removed.clear();
		}
		self.in_flight.push_back(InFlightBatch {
			batch,
			completed: false,
			addresses,
		});
		batch
	}

	/// Records that `batch` completed and removes allocations whose last user is a batch at or before the oldest
	/// batch still running.
	///
	/// Batches can complete out of order, so allocations only leave once every earlier batch has completed too.
	/// The removals are committed with the next submission.
	fn complete(&mut self, batch: u64) {
		if let Some(entry) = self.in_flight.iter_mut().find(|entry| entry.batch == batch) {
			entry.completed = true;
		}
		while self.in_flight.front().is_some_and(|entry| entry.completed) {
			let Some(mut retired) = self.in_flight.pop_front() else {
				break;
			};
			for address in retired.addresses.drain(..) {
				// A later batch that reused the allocation took over as its last user and keeps it resident.
				if self.resident.get(&address).is_some_and(|(_, last)| *last == retired.batch)
					&& let Some((allocation, _)) = self.resident.remove(&address)
				{
					self.set.removeAllocation(allocation.as_ref());
					self.removed.push(allocation);
				}
			}
			self.spare_addresses.push(retired.addresses);
		}
	}
}

impl Deref for NativeCommand {
	type Target = ProtocolObject<dyn mtl::MTL4CommandBuffer>;

	fn deref(&self) -> &Self::Target {
		self.command_buffer.as_ref()
	}
}

/// The `SubmittedBatch` struct owns one queue submission until Metal reports completion.
pub(crate) struct SubmittedBatch {
	queue_handle: graphics_hardware_interface::QueueHandle,
	/// The queue residency number of this batch, which releases its allocations once it completes.
	batch: u64,
	commands: SmallVec<[NativeCommand; 4]>,
	feedback: std::sync::mpsc::Receiver<BatchCommitFeedbackStatus>,
	_commit_options: Retained<mtl::MTL4CommitOptions>,
}

impl SubmittedBatch {
	/// Waits for Metal's completion message, returns the commands to their queue's pool, and reports any GPU error.
	pub(crate) fn wait(mut self, queues: &mut [StoredQueue]) -> Option<String> {
		let feedback = self.feedback.recv().unwrap_or(BatchCommitFeedbackStatus::HandlerFailed);
		let error = match feedback {
			BatchCommitFeedbackStatus::Succeeded => None,
			BatchCommitFeedbackStatus::Failed(error) => Some(format!(
				"Metal 4 GPU execution failed: {error}. The most likely cause is that the submitted batch used invalid GPU commands, resources, or state."
			)),
			BatchCommitFeedbackStatus::HandlerFailed => Some(String::from(
				"Metal 4 commit feedback failed. The most likely cause is that Metal returned invalid feedback data or the feedback handler encountered an unexpected failure.",
			)),
		};
		for command in &mut self.commands {
			command.reset();
		}
		let queue = &mut queues[self.queue_handle.0 as usize];
		queue.residency.complete(self.batch);
		queue.command_pool.extend(self.commands);
		error
	}
}

/// The `StoredQueue` struct owns one Metal 4 queue and its context-local native command pool.
pub(crate) struct StoredQueue {
	pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
	pub(crate) resource_tracker: synchronization::MetalResourceTracker,
	command_pool: Vec<NativeCommand>,
	residency: QueueResidency,
}

impl StoredQueue {
	pub(crate) fn new(queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>) -> Self {
		let residency = QueueResidency::new(&queue.device());
		Self {
			queue,
			resource_tracker: synchronization::MetalResourceTracker::default(),
			command_pool: Vec::new(),
			residency,
		}
	}

	/// Acquires a reset native command and begins recording with its paired allocator.
	pub(crate) fn acquire_native_command(&mut self, label: Option<&str>, debug_labels: bool) -> NativeCommand {
		let mut command = self.command_pool.pop().unwrap_or_else(|| NativeCommand::new(self));
		command.begin(&self.residency.set, label, debug_labels);
		command
	}

	/// Submits uniquely owned commands and returns one batch that owns them through completion.
	pub(crate) fn submit_batch(
		&mut self,
		queue_handle: graphics_hardware_interface::QueueHandle,
		mut commands: SmallVec<[NativeCommand; 4]>,
	) -> SubmittedBatch {
		assert!(
			!commands.is_empty(),
			"Metal 4 command batch submission failed. The most likely cause is that an empty command batch reached submission.",
		);
		let batch = self.residency.admit(&mut commands);
		let mut command_buffers = SmallVec::<[NonNull<ProtocolObject<dyn mtl::MTL4CommandBuffer>>; 4]>::new();
		for command in &mut commands {
			command.finish();
			command_buffers.push(NonNull::from(command.command_buffer.as_ref()));
		}

		let command_buffer_pointer = NonNull::new(command_buffers.as_mut_ptr()).expect(
			"Metal 4 command batch pointer was null. The most likely cause is that an empty command batch reached submission.",
		);

		let (feedback_sender, feedback) = std::sync::mpsc::sync_channel(1);
		let feedback_handler = StackBlock::new(move |feedback: NonNull<ProtocolObject<dyn mtl::MTL4CommitFeedback>>| {
			// Metal may invoke this block on any thread, so it sends an owned result without accessing GHI state.
			let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
				// SAFETY: Metal owns the feedback object for the full feedback-handler invocation.
				let feedback = unsafe { feedback.as_ref() };
				match feedback.error() {
					Some(error) => BatchCommitFeedbackStatus::Failed(error.localizedDescription().to_string()),
					None => BatchCommitFeedbackStatus::Succeeded,
				}
			}))
			.unwrap_or(BatchCommitFeedbackStatus::HandlerFailed);
			let _ = feedback_sender.try_send(result);
		});
		let commit_options = mtl::MTL4CommitOptions::new();
		// SAFETY: The copied block is retained below until Metal invokes it for this submission.
		unsafe { commit_options.addFeedbackHandler(NonNull::from(&*feedback_handler).as_ptr()) };
		// SAFETY: The pointer addresses `command_buffers.len()` contiguous retained command-buffer references.
		unsafe {
			self.queue
				.commit_count_options(command_buffer_pointer, command_buffers.len(), commit_options.as_ref())
		};
		SubmittedBatch {
			queue_handle,
			batch,
			commands,
			feedback,
			_commit_options: commit_options,
		}
	}
}

impl Clone for StoredQueue {
	fn clone(&self) -> Self {
		// Every Context gets an independent completion timeline and native command pool.
		Self::new(self.queue.clone())
	}
}

/// The `Queue` struct provides borrowed Metal queue submission without transferring context ownership.
pub struct Queue<'a> {
	pub(crate) device: &'a mut context::Context,
	pub(crate) queue_handle: graphics_hardware_interface::QueueHandle,
}

/// The `Execution` struct gathers Metal command-buffer recordings before one batched queue submission.
pub struct Execution<'a> {
	frame: Option<super::Frame<'a>>,
	completed_frame: Option<graphics_hardware_interface::FrameKey>,
	command_buffers: SmallVec<[super::FinishedCommandBuffer<'static>; 4]>,
}

impl Drop for Execution<'_> {
	fn drop(&mut self) {
		let Some(frame) = self.frame.as_mut() else {
			return;
		};
		for command_buffer in &self.command_buffers {
			for &handle in &command_buffer.texture_readbacks {
				frame.device().texture_readbacks.abandon_recorded(handle);
			}
		}
	}
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
		let mut command_buffer = crate::frame::Frame::create_command_buffer_recording(frame, command_buffer_handle);
		record(&mut command_buffer);
		self.command_buffers.push(command_buffer.into_finished());
	}
}

impl crate::queue::Queue for Queue<'_> {
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
		self.device
			.start_frame(index, synchronizer_handle, self.queue_handle, &std::alloc::Global)
	}

	fn execute<'a, P>(
		&'a mut self,
		frame: Option<crate::queue::FrameRequest<'a>>,
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
		let frame = frame.map(|frame| {
			self.device
				.start_frame(frame.index, frame.synchronizer, queue_handle, frame.allocator)
		});
		let completed_frame = frame.as_ref().and_then(|frame| frame.completed_frame);
		let frame = frame.map(|frame| frame.frame);
		let mut execution = Execution {
			frame,
			completed_frame,
			command_buffers: SmallVec::new(),
		};
		let present_keys = execute(&mut execution);

		let Some(mut frame) = execution.frame.take() else {
			return;
		};
		let command_buffers = std::mem::take(&mut execution.command_buffers);
		frame.execute_finished_batch(command_buffers, present_keys.as_ref(), synchronizer);
	}
}

use std::ops::Deref;
use std::ptr::NonNull;

use block2::StackBlock;
use objc2::Message;
use objc2::runtime::{ImplementedBy, NSObjectProtocol};
use objc2_foundation::NSString;
use objc2_metal::{MTL4CommandAllocator, MTL4CommandBuffer, MTL4CommandQueue, MTL4CommitFeedback, MTLDevice, MTLResidencySet};

use super::*;
