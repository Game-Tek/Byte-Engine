use super::*;

impl Drop for Context {
	fn drop(&mut self) {
		unsafe {
			self.device.device_wait_idle().expect(
				"Failed to wait for the Vulkan device during context destruction. The most likely cause is that the device was lost.",
			);
			// Retired storage is no longer referenced by any live resource, so the loops below would leak it.
			self.destroy_retired_resources();
			for frame in self.command_buffers.iter().flat_map(|command_buffer| &command_buffer.frames) {
				self.device.destroy_command_pool(frame.command_pool, None);
			}

			for synchronizer in &self.synchronizers {
				self.device.destroy_semaphore(synchronizer.semaphore, None);
				self.device.destroy_fence(synchronizer.fence, None);
			}

			for pipeline in &self.pipelines {
				self.device.destroy_pipeline(pipeline.pipeline, None);
			}

			let meshes = self.meshes.iter().map(|mesh| mesh.buffer);
			for buffer in meshes.chain(self.buffers.iter().map(|buffer| buffer.buffer)) {
				self.device.destroy_buffer(buffer, None);
			}
			if let Some(heaps) = &self.descriptor_heaps {
				self.device.destroy_buffer(heaps.resource().buffer, None);
				self.device.destroy_buffer(heaps.sampler().buffer, None);
			}
			// Unconsumed readbacks own dedicated mapped memory outside the general allocation registry.
			for readback in self.texture_readbacks.values() {
				self.release_texture_readback(readback);
			}

			for image in &self.images {
				if let Some(staging_buffer) = image.staging_buffer {
					self.device.destroy_buffer(staging_buffer, None);
				}
				if !image.full_image_view.is_null() {
					self.device.destroy_image_view(image.full_image_view, None);
				}
				for &vk_image_view in &image.image_views {
					self.device.destroy_image_view(vk_image_view, None);
				}
			}

			for swapchain in &self.swapchains {
				self.swapchain.destroy_swapchain(swapchain.swapchain, None);
				self.surface.destroy_surface(swapchain.surface, None);
			}

			for image in self.images.iter().filter(|image| image.owns_image) {
				self.device.destroy_image(image.image, None);
			}

			for shader in &self.shaders {
				self.device.destroy_shader_module(shader.shader, None);
			}

			for allocation in self
				.allocations
				.iter()
				.filter(|allocation| allocation.memory != vk::DeviceMemory::null())
			{
				self.device.free_memory(allocation.memory, None);
			}
		}
	}
}
