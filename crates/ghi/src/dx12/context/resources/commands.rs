use super::super::*;

impl Device {
	/// Creates one closed allocator and command-list pair for a frame sequence.
	fn create_command_buffer_frame(device: &ID3D12Device10, sequence_index: u8) -> CommandBufferFrame {
		let allocator = unsafe { device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.expect(
			"Failed to create a DX12 command allocator. The most likely cause is that the device was removed or is out of memory.",
		);
		// CreateCommandList1 returns a closed list, avoiding an error-prone create-then-close initialization path.
		let command_list = unsafe {
			device.CreateCommandList1::<ID3D12GraphicsCommandList7>(
				0,
				D3D12_COMMAND_LIST_TYPE_DIRECT,
				D3D12_COMMAND_LIST_FLAG_NONE,
			)
		}
		.expect(
			"Failed to create a DX12 command list. The most likely cause is that the device was removed or is out of memory.",
		);

		CommandBufferFrame {
			lifecycle: CommandBufferLifecycle::Idle,
			allocator,
			command_list,
			retained_descriptor_heaps: Vec::new(),
			retained_resources: Vec::new(),
			retained_resource_keys: HashSet::default(),
			descriptor_sync_scratch: SmallVec::new(),
			present_resources: SmallVec::new(),
			recorded_readbacks: SmallVec::new(),
			recorded_builds: Default::default(),
			vertex_buffer_views: SmallVec::new(),
			vertex_strides: SmallVec::new(),
			original_buffer_states: SmallVec::new(),
			original_image_states: SmallVec::new(),
			clear_descriptors: None,
			retired_clear_descriptors: SmallVec::new(),
			retired_set_descriptors: SmallVec::new(),
			generation: 0,
			is_open: false,
			recorded_work: false,
			sequence_index,
			frame_synchronizer: None,
			last_submission: None,
		}
	}

	pub fn create_command_buffer(&mut self, _name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle {
		assert!(
			(queue_handle.index() as usize) < self.queues.len(),
			"Invalid DX12 queue handle. The most likely cause is that the command buffer references a queue from another device."
		);
		let frames = (0..self.frames)
			.map(|sequence_index| Self::create_command_buffer_frame(&self.device, sequence_index))
			.collect();
		self.command_buffers.push(CommandBuffer {
			queue_handle,
			frames,
			active_sequence_index: 0,
		});

		CommandBufferHandle::from_index((self.command_buffers.len() - 1) as u64)
	}

	/// Resizes every logical command buffer's frame-local native recording storage at an idle boundary.
	pub(crate) fn resize_command_buffer_frames(&mut self, frames: u8) {
		let device = self.device.clone();
		for command_buffer in &mut self.command_buffers {
			while command_buffer.frames.len() < frames as usize {
				let sequence_index = command_buffer.frames.len() as u8;
				command_buffer
					.frames
					.push(Self::create_command_buffer_frame(&device, sequence_index));
			}
			command_buffer.frames.truncate(frames as usize);
			if command_buffer.active_sequence_index >= frames {
				command_buffer.active_sequence_index = 0;
			}
		}
	}

	/// Records a workload on `queue_handle`, submits it, and waits for it; see [`crate::context::Context::record`].
	///
	/// Each queue keeps one command buffer for its workloads, which the wait frees for the next one.
	pub(crate) fn record_workload<'a, R>(
		&'a mut self,
		queue_handle: QueueHandle,
		name: Option<&str>,
		record: impl FnOnce(&mut super::super::super::CommandBufferRecording<'a>) -> R,
	) -> R {
		let command_buffer = match self.workload_command_buffers.iter().find(|(queue, _)| *queue == queue_handle) {
			Some(&(_, command_buffer)) => command_buffer,
			None => {
				let command_buffer = self.create_command_buffer(Some("Workload"), queue_handle);
				self.workload_command_buffers.push((queue_handle, command_buffer));
				command_buffer
			}
		};
		let synchronizer = match self.workload_synchronizer {
			Some(synchronizer) => synchronizer,
			None => {
				let synchronizer = self.create_synchronizer(Some("Workload"), false);
				self.workload_synchronizer = Some(synchronizer);
				synchronizer
			}
		};
		let this: *mut Self = self;
		self.begin_command_buffer(command_buffer, 0);
		self.record_ready_compactions(command_buffer);
		let mut recording = super::super::super::CommandBufferRecording::new(self, command_buffer, None);
		if let Some(name) = name {
			crate::command_buffer::CommonCommandBufferMode::start_region(&mut recording, |label| label.write_str(name));
		}
		let result = record(&mut recording);
		if name.is_some() {
			crate::command_buffer::CommonCommandBufferMode::end_region(&mut recording);
		}
		recording.submit_workload(synchronizer);
		// SAFETY: Submitting consumed the recording, which was the only borrow of the context.
		let this = unsafe { &mut *this };
		this.wait_for_synchronizer(synchronizer);
		result
	}
}
