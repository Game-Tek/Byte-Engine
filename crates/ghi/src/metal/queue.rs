enum BatchCommitFeedbackStatus {
	Succeeded,
	Failed(String),
	HandlerFailed,
}

/// The `NativeCommand` struct owns reusable Metal 4 recording state for one queue submission.
pub(crate) struct NativeCommand {
	allocator: Retained<ProtocolObject<dyn mtl::MTL4CommandAllocator>>,
	command_buffer: Retained<ProtocolObject<dyn mtl::MTL4CommandBuffer>>,
	residency_set: Retained<ProtocolObject<dyn mtl::MTLResidencySet>>,
	retained_allocations: SmallVec<[Retained<ProtocolObject<dyn mtl::MTLAllocation>>; 32]>,
	retained_objects: SmallVec<[Retained<AnyObject>; 4]>,
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
			allocator,
			command_buffer,
			residency_set,
			retained_allocations: SmallVec::new(),
			retained_objects: SmallVec::new(),
		}
	}

	// Starts a fresh recording cycle with the command's paired allocator and residency set.
	fn begin(&mut self, label: Option<&str>, debug_labels: bool) {
		self.command_buffer.beginCommandBufferWithAllocator(self.allocator.as_ref());
		self.command_buffer.useResidencySet(self.residency_set.as_ref());

		#[cfg(debug_assertions)]
		if debug_labels {
			self.command_buffer.setLabel(label.map(NSString::from_str).as_deref());
		}
	}

	/// Returns a Metal 4 compute encoder for resource-tracked dispatch and transfer commands.
	pub(crate) fn compute_command_encoder(&self) -> Option<Retained<ProtocolObject<dyn mtl::MTL4ComputeCommandEncoder>>> {
		self.command_buffer.computeCommandEncoder()
	}

	/// Returns a Metal 4 render encoder for a resource-tracked render pass.
	pub(crate) fn render_command_encoder(
		&self,
		descriptor: &mtl::MTL4RenderPassDescriptor,
	) -> Option<Retained<ProtocolObject<dyn mtl::MTL4RenderCommandEncoder>>> {
		self.command_buffer.renderCommandEncoderWithDescriptor(descriptor)
	}

	/// Retains a Metal buffer and declares its allocation in this command's residency set.
	pub(crate) fn retain_buffer(&mut self, buffer: Retained<ProtocolObject<dyn mtl::MTLBuffer>>) {
		let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(buffer) };
		self.retain_allocation(allocation);
	}

	/// Retains a Metal texture and declares its allocation in this command's residency set.
	pub(crate) fn retain_texture(&mut self, texture: Retained<ProtocolObject<dyn mtl::MTLTexture>>) {
		let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(texture) };
		self.retain_allocation(allocation);
	}

	/// Retains a compute pipeline and declares its compiled allocation in this command's residency set.
	pub(crate) fn retain_compute_pipeline(&mut self, pipeline: Retained<ProtocolObject<dyn mtl::MTLComputePipelineState>>) {
		let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(pipeline) };
		self.retain_allocation(allocation);
	}

	/// Retains a render pipeline and declares its compiled allocation in this command's residency set.
	pub(crate) fn retain_render_pipeline(&mut self, pipeline: Retained<ProtocolObject<dyn mtl::MTLRenderPipelineState>>) {
		let allocation = unsafe { Retained::cast_unchecked::<ProtocolObject<dyn mtl::MTLAllocation>>(pipeline) };
		self.retain_allocation(allocation);
	}

	/// Retains a sampler referenced from a nested argument buffer until GPU completion.
	pub(crate) fn retain_sampler(&mut self, sampler: Retained<ProtocolObject<dyn mtl::MTLSamplerState>>) {
		let sampler = unsafe { Retained::cast_unchecked::<AnyObject>(sampler) };
		self.retained_objects.push(sampler);
	}

	/// Retains a Metal 4 argument table until every command snapshot that references it completes.
	pub(crate) fn retain_argument_table(&mut self, table: Retained<ProtocolObject<dyn mtl::MTL4ArgumentTable>>) {
		let table = unsafe { Retained::cast_unchecked::<AnyObject>(table) };
		self.retained_objects.push(table);
	}

	/// Retains a drawable and its texture until Metal completes the submitted batch.
	pub(crate) fn retain_drawable(&mut self, drawable: Retained<ProtocolObject<dyn CAMetalDrawable>>) {
		self.retain_texture(drawable.texture());
		let drawable = unsafe { Retained::cast_unchecked::<AnyObject>(drawable) };
		self.retained_objects.push(drawable);
	}

	/// Retains native allocations referenced indirectly by this command.
	pub(crate) fn retain_allocations(
		&mut self,
		allocations: impl IntoIterator<Item = Retained<ProtocolObject<dyn mtl::MTLAllocation>>>,
	) {
		for allocation in allocations {
			self.retain_allocation(allocation);
		}
	}

	fn retain_allocation(&mut self, allocation: Retained<ProtocolObject<dyn mtl::MTLAllocation>>) {
		if self
			.retained_allocations
			.iter()
			.any(|retained| std::ptr::eq::<ProtocolObject<dyn mtl::MTLAllocation>>(retained.as_ref(), allocation.as_ref()))
		{
			return;
		}
		self.residency_set.addAllocation(allocation.as_ref());
		self.retained_allocations.push(allocation);
	}

	// Ends recording and commits residency changes before queue submission.
	fn finish(&mut self) {
		self.residency_set.commit();
		self.command_buffer.endCommandBuffer();
	}

	// Resets native recording state after the owning submitted batch completes.
	fn reset(&mut self) {
		self.allocator.reset();
		self.residency_set.removeAllAllocations();
		self.residency_set.commit();
		self.retained_allocations.clear();
		self.retained_objects.clear();
	}
}

impl Deref for NativeCommand {
	type Target = ProtocolObject<dyn mtl::MTL4CommandBuffer>;

	fn deref(&self) -> &Self::Target {
		self.command_buffer.as_ref()
	}
}

impl AsRef<NativeCommand> for NativeCommand {
	fn as_ref(&self) -> &NativeCommand {
		self
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
	// Waits for Metal's completion message and returns the commands for queue-local recycling.
	pub(crate) fn wait(
		mut self,
	) -> (
		graphics_hardware_interface::QueueHandle,
		SmallVec<[NativeCommand; 4]>,
		Option<String>,
	) {
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
		(self.queue_handle, self.commands, error)
	}
}

/// The `StoredQueue` struct owns one Metal 4 queue and its context-local native command pool.
pub(crate) struct StoredQueue {
	pub(crate) queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>,
	pub(crate) workloads: crate::WorkloadTypes,
	pub(crate) resource_tracker: synchronization::MetalResourceTracker,
	command_pool: Vec<NativeCommand>,
}

impl StoredQueue {
	pub(crate) fn new(queue: Retained<ProtocolObject<dyn mtl::MTL4CommandQueue>>, workloads: crate::WorkloadTypes) -> Self {
		Self {
			queue,
			workloads,
			resource_tracker: synchronization::MetalResourceTracker::default(),
			command_pool: Vec::new(),
		}
	}

	/// Acquires a reset native command and begins recording with its paired allocator.
	pub(crate) fn acquire_native_command(&mut self, label: Option<&str>, debug_labels: bool) -> NativeCommand {
		let mut command = self.command_pool.pop().unwrap_or_else(|| NativeCommand::new(self));
		command.begin(label, debug_labels);
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
		unsafe {
			commit_options.addFeedbackHandler(NonNull::from(&*feedback_handler).as_ptr());
			self.queue
				.commit_count_options(command_buffer_pointer, command_buffers.len(), commit_options.as_ref());
		}
		SubmittedBatch {
			queue_handle,
			commands,
			feedback,
			_commit_options: commit_options,
		}
	}

	/// Returns completed commands to this queue's exclusive reuse pool.
	pub(crate) fn recycle(&mut self, commands: impl IntoIterator<Item = NativeCommand>) {
		self.command_pool.extend(commands);
	}
}

impl Clone for StoredQueue {
	fn clone(&self) -> Self {
		// Every Context gets an independent completion timeline and native command pool.
		Self::new(self.queue.clone(), self.workloads)
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
		let mut command_buffer = frame.create_command_buffer_recording(command_buffer_handle);
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
use objc2::runtime::AnyObject;
use objc2_foundation::NSString;
use objc2_metal::{
	MTL4CommandAllocator, MTL4CommandBuffer, MTL4CommandQueue, MTL4CommitFeedback, MTLDevice, MTLResidencySet,
};

use super::*;
