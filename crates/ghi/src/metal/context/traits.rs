use super::*;
use crate::device::Device as _;

impl crate::context::Context for Context {
	type Queue<'a> = crate::metal::queue::Queue<'a>;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		false
	}

	fn supports_bc_texture_compression(&self) -> bool {
		// self.device.supportsBCTextureCompression()
		true
	}

	fn supports_ray_tracing(&self) -> bool {
		self.settings.ray_tracing && self.device.supportsRaytracing()
	}

	fn bottom_level_acceleration_structure_size(
		&self,
		handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) -> usize {
		self.acceleration_structures.bottom_level(handle).structure.size()
	}

	/// Creates a borrowed queue wrapper for queue-local submission.
	fn queue<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> queue::Queue<'a> {
		queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn create_command_buffer_recording(
		&mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> impl crate::command_buffer::CommandBufferRecording + crate::command_buffer::CommonCommandBufferMode {
		Context::create_command_buffer_recording(self, command_buffer_handle)
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().gpu_address
	}

	fn get_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &T {
		// SAFETY: Typed handles preserve the allocation's type and the buffer remains mapped while the context lives.
		unsafe { &*self.typed_buffer_pointer(buffer_handle, 0) }
	}

	fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		// SAFETY: Typed handles preserve the allocation's type and `&mut self` guarantees exclusive CPU access.
		unsafe { &mut *self.typed_buffer_pointer(buffer_handle, 0) }
	}

	/// Transfers the mapped range to a higher-level owner without manufacturing an unbounded reference.
	///
	/// # Safety
	///
	/// The caller must keep the context and buffer alive and must not create another CPU mapping until the returned mapping is discarded.
	unsafe fn transfer_buffer_mapping<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> crate::buffer::Mapping {
		let pointer = self.typed_buffer_pointer::<T>(buffer_handle, 0);
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), T::byte_count(pointer)) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.sync_buffer_copy(buffer_handle.into(), 0);
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
		let handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		self.images.resource_mut(handle).staging.as_deref_mut().unwrap_or_default()
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		let handle = self.images.nth_handle(image_handle.0, 0).unwrap();
		self.pending_image_syncs.push_back((handle, None));
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image_handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		let Some(staging) = self.images.resource_mut(image_handle).staging.as_mut() else {
			return;
		};

		f(staging);
		self.pending_image_syncs.push_back((image_handle, None));
	}

	/// Applies retained descriptor writes to every frame-local logical set they target.
	///
	/// Each sequence's set resolves the write to that sequence's copy of a per-frame resource. Every copy exists
	/// from creation, so the write resolves completely here.
	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		for write in descriptor_set_writes {
			let frame_offset = write.frame_offset.unwrap_or(0);
			// A public handle names the first set of its frame-local chain.
			let set_handles = self
				.descriptor_sets
				.chain(write.descriptor_set)
				.collect::<SmallVec<[_; MAX_FRAMES_IN_FLIGHT]>>();
			for (sequence_index, set_handle) in set_handles.into_iter().enumerate() {
				let sequence_index = sequence_index as u8;
				if let Some(descriptor) = self.resolve_descriptor_for_frame(write.descriptor, sequence_index, frame_offset) {
					self.update_descriptor_slot(set_handle, write.slot, descriptor, write.array_element);
				}
			}
		}
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		desired_image_count: Option<u8>,
		_fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();

		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(utils::to_pixel_format(SWAPCHAIN_FORMAT));

		let (display_sync_enabled, drawable_count) = match presentation_mode {
			graphics_hardware_interface::PresentationModes::Inmediate => (false, 2),
			graphics_hardware_interface::PresentationModes::FIFO => (true, 2),
			graphics_hardware_interface::PresentationModes::Mailbox => (true, 3),
		};
		let drawable_count = desired_image_count.map_or(drawable_count, |count| count.clamp(2, 3) as usize);
		layer.setDisplaySyncEnabled(display_sync_enabled);
		// A value other than 2 or 3 causes an exception
		layer.setMaximumDrawableCount(drawable_count);

		// Frames render into per-sequence images and presentation blits them into the drawable, so the drawable is
		// never a render target and cannot be framebuffer-only.
		layer.setFramebufferOnly(false);

		window_os_handles.view.setWantsLayer(true);
		window_os_handles.view.setLayer(Some(layer.as_super()));

		let extent = update_layer_extent(&layer, &window_os_handles.view);

		let handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			layer,
			view: window_os_handles.view.clone(),
			images: [None; MAX_FRAMES_IN_FLIGHT],
			uses,
			extent,
			last_presented_time: std::sync::Arc::new(AtomicU64::new(0)),
			presented_instant: None,
			present_interval: None,
		});

		handle
	}

	/// Prepares the swapchain image that `frame` will render into before the frame is started.
	///
	/// Waiting on the frame sequence's synchronizer first gives the same reuse guarantee that
	/// `start_frame` provides for in-frame acquisition. The synchronizer stays signaled, so the
	/// later `start_frame` wait returns immediately. The drawable is taken at submission instead,
	/// so this never waits for the display.
	fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = graphics_hardware_interface::FrameKey::new(frame.index, self.frames).sequence_index;
		let synchronizer_handle = synchronizer_for_sequence(&self.synchronizers, frame.synchronizer, sequence_index);
		self.wait_for_private_synchronizer(synchronizer_handle);
		self.acquire_swapchain_image_for_sequence(sequence_index, swapchain_handle)
	}

	fn set_present_interval(
		&mut self,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
		interval: Option<std::time::Duration>,
	) {
		self.swapchains[swapchain_handle.0 as usize].present_interval = interval;
	}

	/// Waits for Metal work, copies one transfer result, and releases its native staging buffer.
	fn get_image_data(
		&mut self,
		texture_copy_handle: graphics_hardware_interface::TextureCopyHandle,
	) -> Result<crate::TextureReadback, crate::TextureTransferError> {
		// Only the transfer's own submission has to finish; other work in flight on this context keeps running.
		let (_, synchronizer) = self.texture_readbacks.submitted(texture_copy_handle)?;
		match synchronizer {
			Some(synchronizer) => self.wait_for_private_synchronizer(synchronizer),
			None => self.wait(),
		}
		let mut readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		let pointer = readback.buffer.contents().as_ptr().cast::<u8>();
		let layout = &readback.layout;
		let native_bytes_per_image = readback.native_bytes_per_row * layout.row_count;
		debug_assert_eq!(
			layout.bytes_per_image,
			layout.bytes_per_row * layout.row_count,
			"Metal readback rows do not tile the compact image. The most likely cause is a copy layout with row padding.",
		);
		// Metal requires aligned native rows. Repack once mapping is synchronized so callers receive the compact
		// authoritative layout, writing into the bytes the transfer reserved.
		let destination = readback.bytes.spare_capacity_mut().as_mut_ptr().cast::<u8>();
		for image in 0..layout.depth_slices {
			// SAFETY: The transfer sized the mapped buffer for every padded row and reserved the owned vector for every
			// compact row of each image, and the two allocations are distinct.
			unsafe {
				utils::copy_rows(
					pointer.add(image * native_bytes_per_image),
					readback.native_bytes_per_row,
					destination.add(image * layout.bytes_per_image),
					layout.bytes_per_row,
					layout.bytes_per_row,
					layout.row_count,
				);
			}
		}
		// SAFETY: Each image's rows tile its compact bytes, so the copies initialized every byte of the reserved size.
		unsafe { readback.bytes.set_len(layout.bytes_per_image * layout.depth_slices) };

		Ok(crate::TextureReadback {
			bytes: readback.bytes,
			extent: readback.extent,
			format: readback.format,
			bytes_per_row: layout.bytes_per_row,
			bytes_per_image: layout.bytes_per_image,
		})
	}

	fn resize_buffer<T: crate::Pod>(
		&mut self,
		buffer_handle: graphics_hardware_interface::DynamicBufferHandle<T>,
		size: usize,
	) {
		let buffer_handle = buffer_handle.into();
		let buffer = self.buffers.get_single(buffer_handle).unwrap();

		if buffer.size >= size {
			return;
		}

		let (uses, access, name) = (buffer.uses, buffer.access, buffer.name.clone());

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

	fn start_frame_capture(&mut self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	fn end_frame_capture(&mut self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	fn wait_for_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = synchronizer_for_sequence(&self.synchronizers, synchronizer_handle, frame_index as u8);
			self.wait_for_private_synchronizer(synchronizer_handle);
		}
	}

	fn poll_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> bool {
		// Pollers never start frames, so this is where their finished uploads get released.
		self.retire_completed_internal_uploads();
		let mut complete = true;
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = synchronizer_for_sequence(&self.synchronizers, synchronizer_handle, frame_index as u8);
			complete &= self.synchronizers.resource_mut(synchronizer_handle).poll(&mut self.queues);
		}
		complete
	}

	fn wait(&mut self) {
		let mut first_error = None;
		for synchronizer in self.synchronizers.iter_mut() {
			first_error = first_error.or(synchronizer.wait(&mut self.queues));
		}
		if let Some(error) = first_error {
			panic!("{error}");
		}
	}

	fn counter_duration(&self, counter: graphics_hardware_interface::CounterHandle) -> Option<std::time::Duration> {
		self.counters.duration(counter)
	}

	fn set_frames_in_flight(&mut self, frames: u8) {
		assert!(
			frames as usize <= MAX_FRAMES_IN_FLIGHT,
			"Too many Metal frames in flight. The most likely cause is that set_frames_in_flight was called with more than {MAX_FRAMES_IN_FLIGHT} frames. frames={frames}",
		);
		let frames = frames.max(1);
		// Retire upload slots before truncation so their queue ownership cannot be lost.
		for sequence_index in frames as usize..self.internal_upload_queues.len() {
			self.retire_internal_uploads(sequence_index as u8);
		}
		self.frames = frames;
		self.internal_upload_queues.resize(frames as usize, None);
		// TODO: Rebuild dynamic resources for new frame count.
	}
}

impl crate::context::ContextCreate for Context {
	fn create_allocation(
		&mut self,
		size: usize,
		_resource_uses: crate::Uses,
		device_accesses: crate::DeviceAccesses,
	) -> graphics_hardware_interface::AllocationHandle {
		let options = utils::resource_options_from_access(device_accesses);
		let buffer = self.allocator.buffer(size, options);
		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			buffer.setLabel(Some(&NSString::from_str(&format!("Allocation {}", self.allocations.len()))));
		}
		self.allocations.push(buffer);
		graphics_hardware_interface::AllocationHandle((self.allocations.len() - 1) as u64)
	}

	fn add_mesh_from_vertices_and_indices(
		&mut self,
		vertex_count: u32,
		index_count: u32,
		vertices: &[u8],
		indices: &[u8],
		vertex_layout: &[crate::pipelines::VertexElement],
	) -> graphics_hardware_interface::MeshHandle {
		// Split interleaved vertices into one packed stream per Metal vertex binding.
		let index_buffer = self.create_buffer_with_bytes(indices);
		// Meshes carry no name, so labels identify them by handle.
		#[cfg(debug_assertions)]
		let mesh_index = self.meshes.len();
		#[cfg(debug_assertions)]
		if self.settings.debug_labels {
			index_buffer.setLabel(Some(&NSString::from_str(&format!("Mesh {mesh_index} Indices"))));
		}
		let vertex_size: usize = vertex_layout.iter().map(|element| element.format.size()).sum();
		let max_binding = vertex_layout
			.iter()
			.map(|element| element.binding as usize + 1)
			.max()
			.unwrap_or(0);
		let mut binding_spans = vec![Vec::<(usize, usize, usize)>::new(); max_binding];
		let mut source_offset = 0usize;

		for element in vertex_layout {
			let element_size = element.format.size();
			let binding = element.binding as usize;
			let destination_offset = binding_spans[binding].last().map_or(0, |&(_, offset, size)| offset + size);
			binding_spans[binding].push((source_offset, destination_offset, element_size));
			source_offset += element_size;
		}

		let vertex_buffers = binding_spans
			.iter()
			.enumerate()
			.map(|(_binding, spans)| {
				(!spans.is_empty()).then(|| {
					let binding_stride = spans.last().map_or(0, |&(_, offset, size)| offset + size);
					let mut binding_vertices = vec![0u8; binding_stride * vertex_count as usize];

					for vertex_index in 0..vertex_count as usize {
						let source_vertex_offset = vertex_index * vertex_size;
						let destination_vertex_offset = vertex_index * binding_stride;
						for &(span_source_offset, span_destination_offset, span_size) in spans {
							let source = source_vertex_offset + span_source_offset;
							let destination = destination_vertex_offset + span_destination_offset;
							binding_vertices[destination..destination + span_size]
								.copy_from_slice(&vertices[source..source + span_size]);
						}
					}

					let buffer = self.create_buffer_with_bytes(&binding_vertices);
					#[cfg(debug_assertions)]
					if self.settings.debug_labels {
						buffer.setLabel(Some(&NSString::from_str(&format!(
							"Mesh {mesh_index} Vertices (binding {_binding})"
						))));
					}
					buffer
				})
			})
			.collect::<Vec<_>>();

		self.meshes.push(Mesh {
			vertex_buffers,
			index_buffer,
			index_count,
		});

		graphics_hardware_interface::MeshHandle((self.meshes.len() - 1) as u64)
	}

	fn create_shader(
		&mut self,
		name: Option<&str>,
		shader_source_type: crate::shader::Sources,
		stage: crate::ShaderTypes,
		descriptor_sets: impl IntoIterator<Item = crate::shader::ShaderDescriptorSet>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		self.factory.create_shader(name, shader_source_type, stage, descriptor_sets)
	}

	/// Creates one copy of the set per frame in flight, each sized for `layout`. A copy allocates its argument
	/// buffer when a command first binds it.
	fn create_descriptor_set(
		&mut self,
		_name: Option<&str>,
		layout: &crate::shader::DescriptorSetLayout,
	) -> graphics_hardware_interface::DescriptorSetHandle {
		self.descriptor_sets
			.add_chain((0..self.frames).map(|sequence| DescriptorSet::new(layout, sequence)))
	}

	fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = self.factory.create_raster_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = self.factory.create_compute_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		self.assert_ray_tracing("create a ray tracing pipeline");
		let pipeline = self.factory.create_ray_tracing_pipeline(builder);
		self.intern_pipeline(pipeline)
	}

	fn create_shader_binding_table(&mut self, name: Option<&str>) -> graphics_hardware_interface::ShaderBindingTableHandle {
		self.assert_ray_tracing("create a shader binding table");
		self.shader_binding_tables.push(ShaderBindingTable {
			name: crate::debug_name(name),
			written: None,
		});
		graphics_hardware_interface::ShaderBindingTableHandle((self.shader_binding_tables.len() - 1) as u64)
	}

	/// Fills a new visible function table instead of the one the table holds, because recordings that already bound
	/// the old one may not have executed yet.
	fn write_shader_binding_table(
		&mut self,
		table: graphics_hardware_interface::ShaderBindingTableHandle,
		pipeline: graphics_hardware_interface::PipelineHandle,
		hit_groups: &[u32],
	) {
		let shader_binding_table = self.shader_binding_tables.get_mut(table.0 as usize).expect(
			"Metal shader binding table write failed because the handle names no table of this context. The most likely cause is that the handle came from another context. Write tables through the context that created them.",
		);
		let label = shader_binding_table.name.as_deref().filter(|_| self.settings.debug_labels);
		let functions = build_shader_binding_table(&self.pipelines[pipeline.0 as usize], hit_groups, label);
		shader_binding_table.written = Some((pipeline, functions));
	}

	fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::BufferHandle<T> {
		let size = T::layout(builder.length).size();
		let buffer = self.create_buffer_resource(builder.name, size, builder.resource_uses, builder.device_accesses);
		let (handle, _) = self.buffers.add(buffer);

		graphics_hardware_interface::BufferHandle::<T>(handle, std::marker::PhantomData)
	}

	fn build_dynamic_buffer<T: crate::Pod>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		let size = <T as crate::buffer::BufferContents>::layout(builder.length).size();

		// Create every frame sequence's copy up front, so a buffer created mid-frame never shares a copy with another
		// frame in flight, which the CPU could rewrite while that frame still reads it.
		let buffers = (0..self.frames)
			.map(|_| self.create_buffer_resource(builder.name, size, builder.resource_uses, builder.device_accesses))
			.collect::<SmallVec<[_; MAX_FRAMES_IN_FLIGHT]>>();
		let master = self.buffers.add_chain(buffers);

		graphics_hardware_interface::DynamicBufferHandle::<T>(master, std::marker::PhantomData)
	}

	fn build_dynamic_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		crate::image_group::ImageGroups::reject_dynamic_member(&builder);
		let description = ImageDescription::new(&builder);
		// Create every frame sequence's image up front, like dynamic buffers, so frames in flight never share one.
		let images = (0..self.frames)
			.map(|_| self.create_image_resource(builder.get_name(), description))
			.collect::<SmallVec<[_; MAX_FRAMES_IN_FLIGHT]>>();
		let master = self.images.add_chain(images);

		graphics_hardware_interface::DynamicImageHandle(master)
	}

	fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
		if builder.group.is_some() {
			crate::image_group::ImageGroups::validate_member(&builder);
		}
		// A member starts with its own texture at the builder's extent, so descriptors can reference it before the
		// group is placed. Placement replaces the texture with one in the group heap.
		let image = self.create_image_resource(builder.get_name(), ImageDescription::new(&builder));
		let (handle, _) = self.images.add(image);
		if let Some(group) = builder.group {
			self.image_groups.add_member(group, handle);
		}

		graphics_hardware_interface::ImageHandle(handle)
	}

	fn create_image_group(&mut self, name: Option<&str>) -> graphics_hardware_interface::ImageGroupHandle {
		self.image_groups.create(name)
	}

	fn build_sampler(&mut self, builder: sampler_builder::Builder) -> graphics_hardware_interface::SamplerHandle {
		self.samplers
			.push(build_sampler(&self.device, &builder, self.settings.debug_labels));
		graphics_hardware_interface::SamplerHandle((self.samplers.len() - 1) as u64)
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		geometries: &[crate::rt::TriangleGeometry],
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		self.create_bottom_level(name, geometries)
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> crate::TopLevelAccelerationStructureHandle {
		self.create_top_level(name, max_instance_count)
	}

	fn destroy_bottom_level_acceleration_structure(
		&mut self,
		handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		self.destroy_bottom_level(handle);
	}

	fn destroy_top_level_acceleration_structure(&mut self, handle: crate::TopLevelAccelerationStructureHandle) {
		self.destroy_top_level(handle);
	}

	/// Creates one synchronizer per frame sequence, so each frame waits only for the frame that last used its sequence.
	///
	/// Metal synchronizers are signaled whenever they have no pending work, so the initial state needs no storage.
	fn create_synchronizer(&mut self, _name: Option<&str>, _signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		self.synchronizers
			.add_chain((0..self.frames).map(|_| Synchronizer::default()))
	}

	/// Metal counters share one timestamp heap, so a counter is only a slot owner and needs no native object.
	fn create_counter(&mut self, _name: Option<&str>) -> graphics_hardware_interface::CounterHandle {
		self.counters.create()
	}
}
