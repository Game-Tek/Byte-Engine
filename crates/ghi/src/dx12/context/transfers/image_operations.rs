use super::*;

impl Device {
	pub(crate) fn record_image_copy(
		&mut self,
		command_buffer_handle: CommandBufferHandle,
		source_image: crate::BaseImageHandle,
		destination_image: crate::BaseImageHandle,
		sequence_index: u8,
	) {
		let command_list = self.command_list(command_buffer_handle);
		let Some(source) = self.images.get(source_image.index() as usize) else {
			return;
		};
		let Some(destination) = self.images.get(destination_image.index() as usize) else {
			return;
		};
		if source.extent != destination.extent || source.is_3d != destination.is_3d || source.format != destination.format {
			return;
		}
		// Per-frame images keep separate native resources per frame, so copies must use the active frame resource.
		let Some(source_resource) = self.ensure_image_resource_for_sequence(source_image, sequence_index) else {
			return;
		};
		let Some(destination_resource) = self.ensure_image_resource_for_sequence(destination_image, sequence_index) else {
			return;
		};

		self.transition_tracked_image(
			&command_list,
			source_image,
			&source_resource,
			TextureBarrierState::COPY_SOURCE,
		);
		self.transition_tracked_image(
			&command_list,
			destination_image,
			&destination_resource,
			TextureBarrierState::COPY_DESTINATION,
		);
		unsafe {
			command_list.CopyResource(&destination_resource, &source_resource);
		}
		self.transition_tracked_image(
			&command_list,
			destination_image,
			&destination_resource,
			TextureBarrierState::COMMON,
		);
		self.transition_tracked_image(&command_list, source_image, &source_resource, TextureBarrierState::COMMON);
		self.mark_command_buffer_work(command_buffer_handle);
	}
}
