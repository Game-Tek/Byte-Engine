pub(crate) enum BatchCommitFeedbackStatus {
	Succeeded,
	Failed(String),
	HandlerFailed,
}

/// The serial a native command holds while no recording uses it.
const IDLE: u64 = u64::MAX;

/// How long an allocation stays resident after the last command that used it, at the least; it leaves within twice
/// this time. Frames reuse most of their allocations, so lingering keeps a steady frame from changing the residency
/// set at all, while memory the application stopped using is still given back promptly.
const RESIDENCY_LINGER: std::time::Duration = std::time::Duration::from_millis(250);

/// The `NativeCommand` struct owns reusable Metal 4 recording state for one queue submission.
pub(crate) struct NativeCommand {
	allocator: Retained<ProtocolObject<dyn mtl::MTL4CommandAllocator>>,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTL4CommandBuffer>>,
	/// Allocations this recording uses. Submission hands them to the queue's [`QueueResidency`], which keeps them
	/// resident and alive until the GPU finishes with them.
	retained_allocations: SmallVec<[Retained<ProtocolObject<dyn mtl::MTLAllocation>>; 32]>,
	/// Addresses of every allocation and object this recording retains, so repeats cost one set probe.
	retained_addresses: ::utils::hash::HashSet<usize>,
	retained_objects: SmallVec<[Retained<ProtocolObject<dyn NSObjectProtocol>>; 4]>,
	/// The argument buffers this recording reads. A descriptor set writes over its buffer only while no command
	/// holds it, so holding one here keeps its contents as the recording bound them.
	argument_buffers: SmallVec<[Arc<ArgumentBufferSnapshot>; 16]>,
	/// The number of the recording in progress or in flight, or [`IDLE`]. The queue's [`QueueResidency`] shares it
	/// to learn which recordings are still running, including when a recording is dropped without being submitted.
	serial: Arc<AtomicU64>,
	/// The number the queue's resource tracker published this recording's accesses under, which completion hands
	/// back to it. Commands recorded without the tracker, such as an empty frame command, have none.
	tracked_recording: Option<u64>,
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
			argument_buffers: SmallVec::new(),
			serial: Arc::new(AtomicU64::new(IDLE)),
			tracked_recording: None,
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

	/// Returns the number of this recording among its queue's recordings.
	pub(crate) fn serial(&self) -> u64 {
		self.serial.load(Ordering::Relaxed)
	}

	/// Retains a buffer, texture, heap, pipeline state, or acceleration structure until GPU completion and makes it
	/// resident when this command is submitted.
	pub(crate) fn retain_allocation<T: Message + 'static>(&mut self, allocation: &T)
	where
		dyn mtl::MTLAllocation: ImplementedBy<T>,
	{
		let allocation: &ProtocolObject<dyn mtl::MTLAllocation> = ProtocolObject::from_ref(allocation);
		// Commands reach the same buffers and textures once per command; the set keeps repeats to one probe and
		// takes the reference count only once.
		if self
			.retained_addresses
			.insert(std::ptr::from_ref(allocation).cast::<()>() as usize)
		{
			self.retained_allocations.push(allocation.retain());
		}
	}

	/// Retains an object that needs no residency, such as an argument table, until GPU completion.
	pub(crate) fn retain_object<T: Message + 'static>(&mut self, object: &T)
	where
		dyn NSObjectProtocol: ImplementedBy<T>,
	{
		let object: &ProtocolObject<dyn NSObjectProtocol> = ProtocolObject::from_ref(object);
		if self
			.retained_addresses
			.insert(std::ptr::from_ref(object).cast::<()>() as usize)
		{
			self.retained_objects.push(object.retain());
		}
	}

	/// Retains a drawable and its texture until Metal completes the submitted batch.
	pub(crate) fn retain_drawable(&mut self, drawable: &ProtocolObject<dyn CAMetalDrawable>) {
		self.retain_allocation(&*drawable.texture());
		self.retain_object(drawable);
	}

	/// Keeps the argument buffer of a descriptor set as this recording bound it, and makes it resident, until GPU
	/// completion.
	pub(crate) fn retain_argument_buffer(&mut self, snapshot: &Arc<ArgumentBufferSnapshot>) {
		self.retain_allocation(&*snapshot.buffer);
		self.argument_buffers.push(snapshot.clone());
	}

	/// Records the number [`synchronization::MetalResourceTracker::finish_recording`] published this recording under.
	pub(crate) fn set_tracked_recording(&mut self, recording: u64) {
		self.tracked_recording = Some(recording);
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
		self.argument_buffers.clear();
		self.serial.store(IDLE, Ordering::Relaxed);
		self.tracked_recording = None;
	}
}

/// The `QueueResidency` struct keeps one long-lived residency set per queue, so frames that reuse the same resources
/// neither rebuild nor recommit residency.
///
/// An allocation enters the set when a descriptor set starts to hold it, see [`Self::pin`], or when a submitted
/// command uses it. It leaves once no descriptor set holds it, every recording that could reach it has finished, and
/// no recording used it for [`RESIDENCY_LINGER`]. The queue holds a strong reference to each resident allocation,
/// which keeps it alive while the GPU may still use it and keeps its address unique as a key.
pub(crate) struct QueueResidency {
	set: Retained<ProtocolObject<dyn mtl::MTLResidencySet>>,
	/// Resident allocations by object address.
	resident: ::utils::hash::HashMap<usize, Resident>,
	/// The serial of every native command of the queue, see [`NativeCommand::serial`].
	commands: Vec<Arc<AtomicU64>>,
	/// The number the next recording takes.
	next_command: u64,
	/// What `next_command` was at the last sweep. An allocation no recording from then on used has lingered.
	swept_command: u64,
	swept_at: std::time::Instant,
	/// Allocations removed from the set but not yet committed as removed. They stay alive until that commit.
	removed: Vec<Retained<ProtocolObject<dyn mtl::MTLAllocation>>>,
	/// Whether the set holds additions or removals it has not committed.
	dirty: bool,
}

/// The `Resident` struct is one allocation the queue keeps resident, with what still needs it.
struct Resident {
	allocation: Retained<ProtocolObject<dyn mtl::MTLAllocation>>,
	/// Recordings numbered below this may use the allocation.
	users_below: u64,
	/// How many descriptor set entries hold the allocation.
	pins: u32,
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
			commands: Vec::new(),
			next_command: 0,
			swept_command: 0,
			swept_at: std::time::Instant::now(),
			removed: Vec::new(),
			dirty: false,
		}
	}

	/// Makes `allocation` resident for as long as a descriptor set entry holds it, so commands that bind the set
	/// do not have to name its resources. Every call needs one [`Self::unpin`] of the same allocation.
	pub(crate) fn pin(&mut self, allocation: &ProtocolObject<dyn mtl::MTLAllocation>) {
		match self.resident.entry(std::ptr::from_ref(allocation).cast::<()>() as usize) {
			std::collections::hash_map::Entry::Occupied(mut entry) => entry.get_mut().pins += 1,
			std::collections::hash_map::Entry::Vacant(entry) => {
				self.set.addAllocation(allocation);
				entry.insert(Resident {
					allocation: allocation.retain(),
					users_below: 0,
					pins: 1,
				});
				self.dirty = true;
			}
		}
	}

	/// Releases one [`Self::pin`] of the allocation at `address`.
	///
	/// Recordings started before the call may still read the descriptor set as it was, so the allocation stays
	/// resident until they have all finished.
	pub(crate) fn unpin(&mut self, address: usize) {
		let resident = self.resident.get_mut(&address).expect(
			"Unpinned Metal allocation is not resident. The most likely cause is that a descriptor set released an allocation it never held.",
		);
		resident.pins -= 1;
		resident.users_below = resident.users_below.max(self.next_command);
	}

	/// Numbers the recording `command` starts.
	fn begin_command(&mut self, command: &NativeCommand) {
		command.serial.store(self.next_command, Ordering::Relaxed);
		self.next_command += 1;
	}

	/// Makes every allocation the batch's commands retained resident and commits the set if it changed.
	///
	/// Call it before committing the commands, since Metal needs the set committed before work that relies on it.
	fn admit(&mut self, commands: &mut [NativeCommand]) {
		for command in commands {
			// A command is numbered when its recording starts, so it always has a serial here.
			let users_below = command.serial().saturating_add(1);
			for allocation in command.retained_allocations.drain(..) {
				match self.resident.entry(Retained::as_ptr(&allocation) as *const () as usize) {
					std::collections::hash_map::Entry::Occupied(mut entry) => {
						let resident = entry.get_mut();
						resident.users_below = resident.users_below.max(users_below);
					}
					std::collections::hash_map::Entry::Vacant(entry) => {
						self.set.addAllocation(allocation.as_ref());
						entry.insert(Resident {
							allocation,
							users_below,
							pins: 0,
						});
						self.dirty = true;
					}
				}
			}
		}
		if self.swept_at.elapsed() >= RESIDENCY_LINGER {
			self.sweep();
		}
		if std::mem::take(&mut self.dirty) {
			self.set.commit();
			// The commit published the removals, so the GPU no longer references these allocations.
			self.removed.clear();
		}
	}

	/// Removes the allocations nothing needs any more: no descriptor set holds them, every recording that could
	/// use them has finished, and no recording used them since the previous sweep.
	///
	/// The removals are committed with the next submission.
	fn sweep(&mut self) {
		// A command that was dropped, as an abandoned recording's is, runs nothing.
		self.commands.retain(|serial| Arc::strong_count(serial) > 1);
		// Recordings finish out of order, so only those before the oldest running one are known to be done.
		let finished_below = self
			.commands
			.iter()
			.map(|serial| serial.load(Ordering::Relaxed))
			.min()
			.map_or(self.next_command, |oldest| oldest.min(self.next_command));
		let unused_below = finished_below.min(self.swept_command);
		let (set, removed) = (&self.set, &mut self.removed);
		self.resident.retain(|_, resident| {
			if resident.pins != 0 || resident.users_below > unused_below {
				return true;
			}
			set.removeAllocation(resident.allocation.as_ref());
			removed.push(resident.allocation.clone());
			false
		});
		self.dirty |= !self.removed.is_empty();
		self.swept_command = self.next_command;
		self.swept_at = std::time::Instant::now();
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
	commands: SmallVec<[NativeCommand; 4]>,
	feedback: std::sync::mpsc::Receiver<BatchCommitFeedbackStatus>,
	_commit_options: Retained<mtl::MTL4CommitOptions>,
}

impl SubmittedBatch {
	/// Waits for Metal's completion message, returns the commands to their queue's pool, and reports any GPU error.
	pub(crate) fn wait(self, queues: &mut [StoredQueue]) -> Option<String> {
		let feedback = self.feedback.recv().unwrap_or(BatchCommitFeedbackStatus::HandlerFailed);
		self.finish(feedback, queues)
	}

	/// Returns Metal's completion message when it already arrived, without blocking.
	pub(crate) fn try_feedback(&self) -> Option<BatchCommitFeedbackStatus> {
		match self.feedback.try_recv() {
			Ok(feedback) => Some(feedback),
			Err(std::sync::mpsc::TryRecvError::Empty) => None,
			Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(BatchCommitFeedbackStatus::HandlerFailed),
		}
	}

	/// Returns a completed batch's commands to their queue's pool and reports any GPU error.
	pub(crate) fn finish(mut self, feedback: BatchCommitFeedbackStatus, queues: &mut [StoredQueue]) -> Option<String> {
		let error = match feedback {
			BatchCommitFeedbackStatus::Succeeded => None,
			BatchCommitFeedbackStatus::Failed(error) => Some(format!(
				"Metal 4 GPU execution failed: {error}. The most likely cause is that the submitted batch used invalid GPU commands, resources, or state."
			)),
			BatchCommitFeedbackStatus::HandlerFailed => Some(String::from(
				"Metal 4 commit feedback failed. The most likely cause is that Metal returned invalid feedback data or the feedback handler encountered an unexpected failure.",
			)),
		};
		let queue = &mut queues[self.queue_handle.0 as usize];
		for command in &mut self.commands {
			// Completed accesses cannot race later commands, so the queue's hazard history drops them. Frames overlap,
			// so the queue is rarely idle, and without this the history would keep every access ever made.
			if let Some(recording) = command.tracked_recording {
				queue.resource_tracker.complete_recording(recording);
			}
			// Resetting a command also tells the queue's residency that its recording finished.
			command.reset();
		}
		queue.command_pool.extend(self.commands);
		error
	}
}

/// The `StoredQueue` struct owns one Metal 4 queue and its context-local native command pool.
pub(crate) struct StoredQueue {
	pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
	pub(crate) resource_tracker: synchronization::MetalResourceTracker,
	command_pool: Vec<NativeCommand>,
	/// What the queue keeps resident. Descriptor sets pin the allocations they hold here.
	pub(crate) residency: QueueResidency,
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
		let mut command = match self.command_pool.pop() {
			Some(command) => command,
			None => {
				let command = NativeCommand::new(self);
				self.residency.commands.push(command.serial.clone());
				command
			}
		};
		self.residency.begin_command(&command);
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
		self.residency.admit(&mut commands);
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
	command_buffers: SmallVec<[super::FinishedCommandBuffer; 4]>,
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

	fn record_with_present_keys<'record>(
		&'record mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
		_present_keys: &[graphics_hardware_interface::PresentKey],
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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use block2::StackBlock;
use objc2::Message;
use objc2::runtime::{ImplementedBy, NSObjectProtocol};
use objc2_foundation::NSString;
use objc2_metal::{MTL4CommandAllocator, MTL4CommandBuffer, MTL4CommandQueue, MTL4CommitFeedback, MTLDevice, MTLResidencySet};

use super::*;
