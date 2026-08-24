use super::super::*;

impl Context {
	#[cfg(any(debug_assertions, test))]
	pub fn has_errors(&self) -> bool {
		false
	}

	pub fn set_frames_in_flight(&mut self, frames: u8) {
		let frames = frames.max(1);
		// Retire upload slots before truncation so their queue ownership cannot be lost.
		for sequence_index in frames as usize..self.internal_upload_queues.len() {
			self.retire_internal_uploads(sequence_index as u8);
		}
		self.frames = frames;
		self.internal_upload_queues.resize(frames as usize, None);
		// TODO: Rebuild dynamic resources for new frame count.
	}

	pub fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		let options = utils::resource_options_from_access(device_accesses);
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal allocation failed. The most likely cause is that the device is out of memory.");
		let pointer = buffer.contents().as_ptr() as *mut u8;

		self.allocations.push(Allocation { buffer, pointer, size });
		graphics_hardware_interface::AllocationHandle((self.allocations.len() - 1) as u64)
	}

	pub fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		// Split interleaved vertices into one packed stream per Metal vertex binding.
		let options = mtl::MTLResourceOptions::StorageModeShared;
		let index_ptr = NonNull::new(indices.as_ptr() as *mut std::ffi::c_void)
			.expect("Index data pointer was null. The most likely cause is an empty index slice.");
		let index_buffer = unsafe {
			self.device
				.newBufferWithBytes_length_options(index_ptr, indices.len() as _, options)
		}
		.expect("Metal index buffer creation failed. The most likely cause is that the device is out of memory.");
		let vertex_size = vertex_layout.iter().map(|element| element.format.size()).sum();
		let max_binding = vertex_layout
			.iter()
			.map(|element| element.binding)
			.max()
			.map(|binding| binding as usize + 1)
			.unwrap_or(0);
		let mut binding_spans = vec![Vec::<(usize, usize, usize)>::new(); max_binding];
		let mut source_offset = 0usize;

		for element in vertex_layout {
			let element_size = element.format.size();
			let binding = element.binding as usize;
			let destination_offset = binding_spans[binding]
				.last()
				.map(|(_, destination_offset, size)| destination_offset + size)
				.unwrap_or(0);
			binding_spans[binding].push((source_offset, destination_offset, element_size));
			source_offset += element_size;
		}

		let vertex_buffers = binding_spans
			.iter()
			.map(|spans| {
				if spans.is_empty() {
					return None;
				}

				let binding_stride = spans
					.last()
					.map(|(_, destination_offset, size)| destination_offset + size)
					.unwrap_or(0);
				let mut binding_vertices = vec![0u8; binding_stride * vertex_count as usize];

				for vertex_index in 0..vertex_count as usize {
					let source_vertex_offset = vertex_index * vertex_size;
					let destination_vertex_offset = vertex_index * binding_stride;

					for &(span_source_offset, span_destination_offset, span_size) in spans {
						let source_range =
							source_vertex_offset + span_source_offset..source_vertex_offset + span_source_offset + span_size;
						let destination_range = destination_vertex_offset + span_destination_offset
							..destination_vertex_offset + span_destination_offset + span_size;
						binding_vertices[destination_range].copy_from_slice(&vertices[source_range]);
					}
				}

				let vertex_ptr = NonNull::new(binding_vertices.as_ptr() as *mut std::ffi::c_void)
					.expect("Vertex data pointer was null. The most likely cause is an empty vertex slice.");
				Some(
					unsafe {
						self.device
							.newBufferWithBytes_length_options(vertex_ptr, binding_vertices.len() as _, options)
					}
					.expect("Metal vertex buffer creation failed. The most likely cause is that the device is out of memory."),
				)
			})
			.collect::<Vec<_>>();

		self.meshes.push(Mesh {
			vertex_buffers,
			index_buffer,
			vertex_count,
			index_count,
			vertex_size,
		});

		graphics_hardware_interface::MeshHandle((self.meshes.len() - 1) as u64)
	}

	pub fn build_buffer<T: Copy>(&mut self, builder: buffer_builder::Builder) -> graphics_hardware_interface::BufferHandle<T> {
		let size = std::mem::size_of::<T>();
		let handle = self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);

		graphics_hardware_interface::BufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(handle.0),
			std::marker::PhantomData,
		)
	}

	pub fn build_dynamic_buffer<T: Copy>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		let size = std::mem::size_of::<T>();

		let root = self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
		let master = graphics_hardware_interface::BaseBufferHandle::new(root.0);

		if self.frames > 1 {
			// Defer frame-local resources until the frame is first processed so startup only pays for frame 0.
			self.tasks
				.push(Task::new(Tasks::BuildBuffer(BuildBuffer { previous: root, master }), Some(1)));
		}

		graphics_hardware_interface::DynamicBufferHandle::<T>(master, std::marker::PhantomData)
	}

	/// Creates a borrowed queue wrapper for queue-local submission.
	pub fn queue<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> queue::Queue<'a> {
		queue::Queue {
			device: self,
			queue_handle,
		}
	}

	pub fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::super::CommandBuffer<'a> {
		super::super::CommandBuffer {
			device: self,
			command_buffer_handle,
		}
	}

	pub fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().gpu_address
	}

	pub fn get_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { &*(buffer.pointer as *const T) }
	}

	pub fn get_mut_buffer_slice<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::BufferHandle<T>) -> &mut T {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { &mut *(buffer.pointer as *mut T) }
	}

	/// Transfers the mapped range to a higher-level owner without manufacturing an unbounded reference.
	pub unsafe fn transfer_buffer_mapping<T: Copy>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let buffer = self.buffers.get_single(buffer_handle.into()).unwrap();
		let buffer = buffer
			.staging
			.map(|staging_handle| self.buffers.resource(staging_handle))
			.unwrap_or(buffer);
		unsafe { crate::buffer::Mapping::from_raw_parts(buffer.pointer, std::mem::size_of::<T>()) }
	}

	pub fn resize_buffer<T: Copy>(&mut self, buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>, size: usize) {
		let buffer_handle = buffer_handle.into();
		let buffer = self.buffers.get_single(buffer_handle).unwrap();

		if buffer.size >= size {
			return;
		}

		let uses = buffer.uses;
		let access = buffer.access;
		let name = buffer.name.clone();

		// Dynamic buffers have one materialized resource per in-flight frame. Resize every existing resource so command recording cannot resolve an older allocation for a nonzero sequence.
		for frame_index in 0..self.frames as usize {
			let Some(handle) = self.buffers.nth_handle(buffer_handle, frame_index) else {
				continue;
			};
			let replacement = self.create_buffer_resource(name.as_deref(), size, uses, access);
			*self.buffers.resource_mut(handle) = replacement;
			self.rewrite_descriptors_for_handle(PrivateHandles::Buffer(handle));
		}
	}
}
