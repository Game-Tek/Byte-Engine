use super::*;

impl Drop for Context {
	fn drop(&mut self) {
		unsafe {
			self.device.device_wait_idle().expect(
				"Failed to wait for the Vulkan device during context destruction. The most likely cause is that the device was lost.",
			);
			self.command_buffers.iter().for_each(|command_buffer| {
				command_buffer.frames.iter().for_each(|command_buffer| {
					self.device.destroy_command_pool(command_buffer.command_pool, None);
				});
			});

			self.synchronizers.iter().for_each(|synchronizer| {
				self.device.destroy_semaphore(synchronizer.semaphore, None);
				self.device.destroy_fence(synchronizer.fence, None);
			});

			self.pipelines.iter().for_each(|pipeline| {
				self.device.destroy_pipeline(pipeline.pipeline, None);
			});

			self.meshes.iter().for_each(|mesh| {
				self.device.destroy_buffer(mesh.buffer, None);
			});

			self.buffers.iter().for_each(|buffer| {
				self.device.destroy_buffer(buffer.buffer, None);
			});
			if let Some(heaps) = &self.descriptor_heaps {
				self.device.destroy_buffer(heaps.resource().buffer, None);
				self.device.destroy_buffer(heaps.sampler().buffer, None);
			}
			// Unconsumed readbacks own dedicated mapped memory outside the general allocation registry.
			for readback in self.texture_readbacks.values() {
				self.device.destroy_buffer(readback.buffer, None);
				if readback.memory != vk::DeviceMemory::null() {
					if !readback.pointer.is_null() {
						self.device.unmap_memory(readback.memory);
					}
					self.device.free_memory(readback.memory, None);
				}
			}

			self.images.iter().for_each(|image| {
				if let Some(staging_buffer) = image.staging_buffer {
					self.device.destroy_buffer(staging_buffer, None);
				}

				if !image.full_image_view.is_null() {
					self.device.destroy_image_view(image.full_image_view, None);
				}

				for &vk_image_view in &image.image_views {
					self.device.destroy_image_view(vk_image_view, None);
				}
			});

			self.swapchains.iter().for_each(|swapchain| {
				self.swapchain.destroy_swapchain(swapchain.swapchain, None);
				self.surface.destroy_surface(swapchain.surface, None);
			});

			self.images.iter().for_each(|image| {
				if image.owns_image {
					self.device.destroy_image(image.image, None);
				}
			});

			self.shaders.iter().for_each(|shader| {
				self.device.destroy_shader_module(shader.shader, None);
			});

			self.allocations
				.iter()
				.filter(|allocation| allocation.memory != vk::DeviceMemory::null())
				.for_each(|allocation| {
					self.device.free_memory(allocation.memory, None);
				});
		}
	}
}
