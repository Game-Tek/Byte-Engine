use super::super::*;

impl Device {
	pub fn create_command_buffer(&mut self, _name: Option<&str>, queue_handle: QueueHandle) -> CommandBufferHandle {
		assert!(
			(queue_handle.0 as usize) < self.queues.len(),
			"Invalid DX12 queue handle. The most likely cause is that the command buffer references a queue from another device."
		);
		let allocator = unsafe { self.device.CreateCommandAllocator(D3D12_COMMAND_LIST_TYPE_DIRECT) }.ok();
		let command_list: Option<ID3D12GraphicsCommandList7> = if let Some(allocator) = allocator.as_ref() {
			unsafe {
				self.device
					.CreateCommandList(0, D3D12_COMMAND_LIST_TYPE_DIRECT, allocator, None)
			}
			.ok()
		} else {
			None
		};
		if let Some(command_list) = command_list.as_ref() {
			let _ = unsafe { command_list.Close() };
		}

		self.command_buffers.push(CommandBuffer {
			queue_handle,
			allocator,
			command_list,
			pending_clear_descriptor_copies: Vec::new(),
			prepared_clear_descriptors: Vec::new(),
			retained_descriptor_heaps: Vec::new(),
			retained_resources: Vec::new(),
			retained_upload_resource_count: 0,
			recorded_texture_syncs: Vec::new(),
			original_buffer_states: SmallVec::new(),
			original_image_states: SmallVec::new(),
			cbv_srv_uav_staging_heap: None,
			sampler_staging_heap: None,
			is_open: false,
			recorded_work: false,
			sequence_index: 0,
			last_submission: None,
		});

		CommandBufferHandle((self.command_buffers.len() - 1) as u64)
	}

	pub fn create_command_buffer_recording<'a>(
		&'a mut self,
		command_buffer_handle: CommandBufferHandle,
	) -> super::super::super::CommandBufferRecording<'a> {
		self.begin_command_buffer(command_buffer_handle, 0);
		super::super::super::CommandBufferRecording::new(self, command_buffer_handle, None)
	}
}
