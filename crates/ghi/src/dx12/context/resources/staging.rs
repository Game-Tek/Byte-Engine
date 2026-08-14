use super::super::*;

impl Device {
	pub(crate) fn upload_resource_count(&self) -> usize {
		self.command_buffers
			.iter()
			.map(|command_buffer| command_buffer.retained_upload_resource_count)
			.sum()
	}

	pub(crate) fn readback_resource_count(&self) -> usize {
		self.texture_readbacks.len()
	}

	pub(crate) fn debug_region_begin_count(&self) -> usize {
		self.debug_region_begin_count.get()
	}

	pub(crate) fn debug_region_end_count(&self) -> usize {
		self.debug_region_end_count.get()
	}

	pub(crate) fn texture_copy_count(&self) -> usize {
		self.texture_copy_count
	}

	pub(crate) fn buffer_copy_count(&self) -> usize {
		self.buffer_copy_count
	}

	pub(crate) fn buffer_clear_count(&self) -> usize {
		self.buffer_clear_count
	}

	pub(crate) fn native_command_list_execute_count(&self) -> usize {
		self.native_command_list_execute_count
	}

	pub(crate) fn empty_command_list_skip_count(&self) -> usize {
		self.empty_command_list_skip_count
	}
}
