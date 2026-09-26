use super::resources::acceleration_structures::{INSTANCE_DESCRIPTOR_SIZE, to_index_type, to_vertex_format};
use super::*;

impl crate::context::Context for Context {
	type Queue<'a> = crate::metal::queue::Queue<'a>;
	type CommandBuffer<'a> = crate::metal::CommandBuffer<'a>;

	#[cfg(any(debug_assertions, test))]
	fn has_errors(&self) -> bool {
		false
	}

	fn supports_bc_texture_compression(&self) -> bool {
		// self.device.supportsBCTextureCompression()
		true
	}

	/// Creates a borrowed queue wrapper for queue-local submission.
	fn queue<'a>(&'a mut self, queue_handle: graphics_hardware_interface::QueueHandle) -> queue::Queue<'a> {
		queue::Queue {
			device: self,
			queue_handle,
		}
	}

	fn command_buffer<'a>(
		&'a mut self,
		command_buffer_handle: graphics_hardware_interface::CommandBufferHandle,
	) -> super::CommandBuffer<'a> {
		super::CommandBuffer {
			device: self,
			command_buffer_handle,
		}
	}

	fn get_buffer_address(&self, buffer_handle: graphics_hardware_interface::BaseBufferHandle) -> u64 {
		self.buffers.get_single(buffer_handle).unwrap().gpu_address
	}

	fn get_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &T {
		// SAFETY: Typed handles preserve the allocation's type and the buffer remains mapped while the context lives.
		unsafe { &*self.typed_buffer_pointer(buffer_handle) }
	}

	fn get_mut_buffer_slice<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<T>,
	) -> &mut T {
		// SAFETY: Typed handles preserve the allocation's type and `&mut self` guarantees exclusive CPU access.
		unsafe { &mut *self.typed_buffer_pointer(buffer_handle) }
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
		let pointer = self.typed_buffer_pointer(buffer_handle);
		// SAFETY: The caller accepts the lifetime and exclusivity requirements documented by this method.
		unsafe { crate::buffer::Mapping::from_raw_parts(pointer.cast::<u8>(), T::byte_count(pointer)) }
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		let handle = self.buffers.nth_handle(buffer_handle.into(), 0).unwrap();
		let buffer = self.buffers.resource(handle);
		if buffer.staging.is_some() {
			self.pending_buffer_syncs.push_back(handle);
		}
	}

	fn get_texture_slice_mut(&mut self, texture_handle: graphics_hardware_interface::ImageHandle) -> &mut [u8] {
		let handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		let image = self.images.resource_mut(handle);

		let Some(staging) = image.staging.as_mut() else {
			return &mut [];
		};

		staging.as_mut_slice()
	}

	fn sync_texture(&mut self, image_handle: graphics_hardware_interface::ImageHandle) {
		let handle = self.images.nth_handle(image_handle.0, 0).unwrap();
		self.pending_image_syncs.push_back((handle, None));
	}

	fn write_texture(&mut self, texture_handle: graphics_hardware_interface::ImageHandle, f: impl FnOnce(&mut [u8])) {
		let image_handle = self.images.nth_handle(texture_handle.0, 0).unwrap();
		let image = self.images.resource_mut(image_handle);

		let Some(staging) = image.staging.as_mut() else {
			return;
		};

		f(staging);
		self.pending_image_syncs.push_back((image_handle, None));
	}

	/// Applies retained descriptor writes to every frame-local logical set they target.
	///
	/// Each write is remembered per frame sequence, so a frame resource created later can still be bound.
	fn write(&mut self, descriptor_set_writes: &[crate::descriptors::DescriptorWrite]) {
		for write in descriptor_set_writes {
			let frame_offset = write.frame_offset.unwrap_or(0);
			// A public handle names the first set of its frame-local chain.
			let set_handles = DescriptorSetHandle(write.descriptor_set.0).get_all(&self.descriptor_sets);
			for (sequence_index, set_handle) in set_handles.into_iter().enumerate() {
				let sequence_index = sequence_index as u8;
				self.descriptor_sources.insert(
					(set_handle, write.slot, write.array_element, sequence_index),
					(write.descriptor, frame_offset),
				);
				if let Some(descriptor) = self.resolve_descriptor_for_frame(write.descriptor, sequence_index, frame_offset) {
					self.update_descriptor_slot(set_handle, write.slot, descriptor, sequence_index, write.array_element);
				}
			}
		}
	}

	fn write_instance(
		&mut self,
		instances_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		instance_index: usize,
		transform: [[f32; 4]; 3],
		custom_index: u16,
		mask: u8,
		sbt_record_offset: usize,
		acceleration_structure: graphics_hardware_interface::BottomLevelAccelerationStructureHandle,
	) {
		let structure = &self.acceleration_structures[acceleration_structure.0 as usize];
		// A Metal instance record names its bottom-level structure by GPU resource handle, so the structure must be
		// resident while the build reads the record; the build command marks it.
		let acceleration_structure_id = structure.structure.gpuResourceID();
		let buffer = self.buffers.get_single(instances_buffer_handle).expect(
			"Metal acceleration structure instance buffer is missing. The most likely cause is that its handle came from another context.",
		);
		let offset = instance_index * INSTANCE_DESCRIPTOR_SIZE;

		assert!(
			offset + INSTANCE_DESCRIPTOR_SIZE <= buffer.size,
			"Metal acceleration structure instance is out of bounds. The most likely cause is that the instance buffer was created for fewer instances. instance_index={instance_index}, instance_capacity={}",
			buffer.size / INSTANCE_DESCRIPTOR_SIZE,
		);

		// GHI transforms are row-major 3x4 matrices; Metal reads them as four packed float3 columns.
		let columns = std::array::from_fn(|column| mtl::MTLPackedFloat3 {
			x: transform[0][column],
			y: transform[1][column],
			z: transform[2][column],
		});
		let descriptor = mtl::MTLIndirectAccelerationStructureInstanceDescriptor {
			transformationMatrix: mtl::MTLPackedFloat4x3 { columns },
			options: mtl::MTLAccelerationStructureInstanceOptions::empty(),
			mask: mask as u32,
			intersectionFunctionTableOffset: sbt_record_offset as u32,
			userID: custom_index as u32,
			accelerationStructureID: acceleration_structure_id,
		};

		// SAFETY: The write stays inside the shared instance buffer, whose bounds were checked above.
		unsafe {
			buffer
				.pointer
				.add(offset)
				.cast::<mtl::MTLIndirectAccelerationStructureInstanceDescriptor>()
				.write_unaligned(descriptor);
		}
	}

	fn write_sbt_entry(
		&mut self,
		_sbt_buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		_sbt_record_offset: usize,
		_pipeline_handle: graphics_hardware_interface::PipelineHandle,
		_shader_handle: graphics_hardware_interface::ShaderHandle,
	) {
		// Metal has no shader binding table: a ray-tracing pipeline dispatches its ray-generation function directly
		// and resolves hits through the acceleration structure, so binding-table records carry no backend state.
	}

	fn bind_to_window(
		&mut self,
		window_os_handles: &window::Handles,
		presentation_mode: graphics_hardware_interface::PresentationModes,
		_fallback_extent: Extent,
		uses: crate::Uses,
	) -> graphics_hardware_interface::SwapchainHandle {
		let layer = CAMetalLayer::new();
		let format = crate::Formats::BGRAu8;

		layer.setDevice(Some(&self.device));
		layer.setPixelFormat(utils::to_pixel_format(format));

		let display_sync_enabled = match presentation_mode {
			graphics_hardware_interface::PresentationModes::Inmediate => false,
			graphics_hardware_interface::PresentationModes::FIFO | graphics_hardware_interface::PresentationModes::Mailbox => {
				true
			}
		};

		layer.setDisplaySyncEnabled(display_sync_enabled);

		let desired_drawable_count = match presentation_mode {
			graphics_hardware_interface::PresentationModes::Inmediate
			| graphics_hardware_interface::PresentationModes::FIFO => 2,
			graphics_hardware_interface::PresentationModes::Mailbox => 3,
		};

		// A value other than 2 or 3 causes an exception
		layer.setMaximumDrawableCount(desired_drawable_count);

		let uses_proxy = !drawable_supports_uses(uses);

		// framebufferOnly permits Metal's optimized display path when raster output is the drawable's only use.
		let framebuffer_only_uses = Uses::RenderTarget | Uses::Clear;
		layer.setFramebufferOnly(!uses_proxy && framebuffer_only_uses.contains(uses));

		window_os_handles.view.setWantsLayer(true);
		window_os_handles.view.setLayer(Some(layer.as_super()));

		let extent = update_layer_extent(&layer, &window_os_handles.view);

		// A proxy swapchain renders into one intermediate image per frame sequence and copies it to the drawable.
		let images = std::array::from_fn(|_| {
			uses_proxy.then(|| {
				let description = image::ImageDescription {
					extent,
					format,
					uses: uses | Uses::BlitSource,
					access: DeviceAccesses::DeviceOnly,
					array_layers: 1,
					cube_compatible: false,
					cube_array_compatible: false,
					mip_levels: 1,
				};
				let proxy = build_image(
					&self.device,
					Some("Swapchain Proxy Image"),
					description,
					self.settings.debug_labels,
				);
				self.images.add(proxy).1
			})
		});

		let handle = graphics_hardware_interface::SwapchainHandle(self.swapchains.len() as u64);

		self.swapchains.push(Swapchain {
			layer,
			view: window_os_handles.view.clone(),
			images,
			uses_proxy,
			uses,
			extent,
			pending_drawable: None,
			last_presented_time: std::sync::Arc::new(AtomicU64::new(0)),
			present_interval: None,
		});

		handle
	}

	/// Acquires the drawable that `frame` will present before the frame is started.
	///
	/// Waiting on the frame sequence's synchronizer first gives the same reuse guarantee that
	/// `start_frame` provides for in-frame acquisition. The synchronizer stays signaled, so the
	/// later `start_frame` wait returns immediately.
	fn acquire_swapchain_image(
		&mut self,
		frame: crate::queue::FrameRequest<'_>,
		swapchain_handle: graphics_hardware_interface::SwapchainHandle,
	) -> Option<crate::frame::SwapchainAcquisition> {
		let sequence_index = (frame.index % u64::from(self.frames)) as u8;
		let synchronizer_handle = self.synchronizer_for_sequence(frame.synchronizer, sequence_index);
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
		self.texture_readbacks.submitted(texture_copy_handle)?;
		self.wait();
		let mut readback = self.texture_readbacks.take_submitted(texture_copy_handle)?;
		let pointer = readback.buffer.contents().as_ptr().cast::<u8>();
		// Metal requires aligned native rows. Repack once mapping is synchronized so callers receive the compact authoritative layout.
		for image in 0..readback.image_count {
			for row in 0..readback.row_count {
				let source_offset = image * readback.native_bytes_per_image + row * readback.native_bytes_per_row;
				let destination_offset = image * readback.bytes_per_image + row * readback.bytes_per_row;
				// SAFETY: Native readback layout calculations bound this row within the mapped Metal buffer.
				let source = unsafe { pointer.add(source_offset) };
				// SAFETY: Compact layout calculations bound this row within the owned destination vector.
				let destination = unsafe { readback.bytes.as_mut_ptr().add(destination_offset) };
				// SAFETY: The mapped native buffer and owned destination vector are distinct and cover one compact row.
				unsafe { std::ptr::copy_nonoverlapping(source, destination, readback.bytes_per_row) };
			}
		}

		Ok(crate::TextureReadback {
			bytes: readback.bytes,
			extent: readback.extent,
			format: readback.format,
			bytes_per_row: readback.bytes_per_row,
			bytes_per_image: readback.bytes_per_image,
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

	fn start_frame_capture(&mut self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	fn end_frame_capture(&mut self) {
		// TODO: Hook into MTLCaptureManager when needed.
	}

	fn wait_for_synchronizer(&mut self, synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		for frame_index in 0..self.frames as usize {
			let synchronizer_handle = self.synchronizer_for_sequence(synchronizer_handle, frame_index as u8);
			self.wait_for_private_synchronizer(synchronizer_handle);
		}
	}

	fn wait(&mut self) {
		let mut first_error = None;
		for synchronizer in self.synchronizers.iter_mut() {
			if let Some(error) = synchronizer.wait(&mut self.queues) {
				first_error.get_or_insert(error);
			}
		}
		if let Some(error) = first_error {
			panic!("{error}");
		}
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
		let buffer = self
			.device
			.newBufferWithLength_options(size as _, options)
			.expect("Metal allocation failed. The most likely cause is that the device is out of memory.");
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
		let options = mtl::MTLResourceOptions::StorageModeShared;
		let index_ptr = NonNull::new(indices.as_ptr() as *mut std::ffi::c_void)
			.expect("Index data pointer was null. The most likely cause is an empty index slice.");
		// SAFETY: `index_ptr` references `indices.len()` initialized bytes for the duration of buffer creation.
		let index_buffer = unsafe {
			self.device
				.newBufferWithBytes_length_options(index_ptr, indices.len() as _, options)
		}
		.expect("Metal index buffer creation failed. The most likely cause is that the device is out of memory.");
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
			.enumerate()
			.map(|(_binding, spans)| {
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
				// SAFETY: `vertex_ptr` references the initialized packed binding bytes for the duration of buffer creation.
				let buffer = unsafe {
					self.device
						.newBufferWithBytes_length_options(vertex_ptr, binding_vertices.len() as _, options)
				}
				.expect("Metal vertex buffer creation failed. The most likely cause is that the device is out of memory.");
				#[cfg(debug_assertions)]
				if self.settings.debug_labels {
					buffer.setLabel(Some(&NSString::from_str(&format!(
						"Mesh {mesh_index} Vertices (binding {_binding})"
					))));
				}
				Some(buffer)
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
		shader_resource_descriptors: impl IntoIterator<Item = crate::shader::ShaderResourceDescriptor>,
	) -> Result<graphics_hardware_interface::ShaderHandle, ()> {
		let shader = build_shader(
			&self.device,
			name,
			shader_source_type,
			stage,
			shader_resource_descriptors,
			self.settings.debug_labels,
		)?;
		self.shaders.push(shader);
		Ok(graphics_hardware_interface::ShaderHandle((self.shaders.len() - 1) as u64))
	}

	/// Creates one retained logical descriptor set per in-flight frame without allocating a native layout.
	fn create_descriptor_set(&mut self, _name: Option<&str>) -> graphics_hardware_interface::DescriptorSetHandle {
		let handle = graphics_hardware_interface::DescriptorSetHandle(self.descriptor_sets.len() as u64);
		let mut previous_handle: Option<DescriptorSetHandle> = None;

		for _ in 0..self.frames {
			let descriptor_set_handle = DescriptorSetHandle(self.descriptor_sets.len() as u64);
			self.descriptor_sets.push(descriptor_set::DescriptorSet {
				next: None,
				version: 0,
				descriptors: HashMap::default(),
				argument_buffers: Vec::new(),
			});

			if let Some(previous_handle) = previous_handle {
				self.descriptor_sets[previous_handle.0 as usize].next = Some(descriptor_set_handle);
			}

			previous_handle = Some(descriptor_set_handle);
		}

		handle
	}

	fn create_raster_pipeline(&mut self, builder: raster_pipeline::Builder) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = build_raster_pipeline(
			&self.device,
			&self.compiler,
			&self.shaders,
			self.settings.debug_labels,
			builder,
		);
		self.intern_raster_pipeline(pipeline)
	}

	fn create_compute_pipeline(
		&mut self,
		builder: crate::pipelines::compute::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = build_compute_pipeline(
			&self.device,
			&self.compiler,
			&self.shaders,
			self.settings.debug_labels,
			builder,
		);
		self.intern_raster_pipeline(pipeline)
	}

	fn create_ray_tracing_pipeline(
		&mut self,
		builder: crate::pipelines::ray_tracing::Builder,
	) -> graphics_hardware_interface::PipelineHandle {
		let pipeline = build_ray_tracing_pipeline(
			&self.device,
			&self.compiler,
			&self.shaders,
			self.settings.debug_labels,
			builder,
		);
		self.intern_raster_pipeline(pipeline)
	}

	fn build_buffer<T: ?Sized + crate::buffer::BufferContents>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::BufferHandle<T> {
		let size = T::layout(builder.length).size();
		let handle = self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);

		graphics_hardware_interface::BufferHandle::<T>(
			graphics_hardware_interface::BaseBufferHandle::new(handle.0),
			std::marker::PhantomData,
		)
	}

	fn build_dynamic_buffer<T: crate::Pod>(
		&mut self,
		builder: buffer_builder::Builder,
	) -> graphics_hardware_interface::DynamicBufferHandle<T> {
		let size = <T as crate::buffer::BufferContents>::layout(builder.length).size();

		let root = self.create_buffer_internal(None, builder.name, size, builder.resource_uses, builder.device_accesses);
		let master = graphics_hardware_interface::BaseBufferHandle::new(root.0);

		if self.frames > 1 {
			// Defer frame-local resources until the frame is first processed so startup only pays for frame 0.
			self.tasks.push(Task {
				task: Tasks::BuildBuffer { previous: root, master },
				frame: 1,
			});
		}

		graphics_hardware_interface::DynamicBufferHandle::<T>(master, std::marker::PhantomData)
	}

	fn build_dynamic_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::DynamicImageHandle {
		crate::image_group::ImageGroups::reject_dynamic_member(&builder);
		let root = self.create_image_internal(None, builder.get_name(), image::ImageDescription::new(&builder));
		let master = graphics_hardware_interface::BaseImageHandle::new(root.0);

		if self.frames > 1 {
			// Defer frame-local resources until the frame is first processed so startup only pays for frame 0.
			self.tasks.push(Task {
				task: Tasks::BuildImage { previous: root, master },
				frame: 1,
			});
		}

		graphics_hardware_interface::DynamicImageHandle(master)
	}

	fn build_image(&mut self, builder: image_builder::Builder) -> graphics_hardware_interface::ImageHandle {
		if builder.group.is_some() {
			crate::image_group::ImageGroups::validate_member(&builder);
		}
		// A member starts with its own texture at the builder's extent, so descriptors can reference it before the
		// group is placed. Placement replaces the texture with one in the group heap.
		let image_handle = self.create_image_internal(None, builder.get_name(), image::ImageDescription::new(&builder));
		let handle = graphics_hardware_interface::BaseImageHandle::new(image_handle.0);
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

	fn create_acceleration_structure_instance_buffer(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::BaseBufferHandle {
		// Instance records are written by the host and read by the acceleration-structure build, so they live in
		// shared storage rather than behind a staging copy the caller would have to synchronize.
		let buffer = self.create_buffer_resource(
			name,
			max_instance_count as usize * INSTANCE_DESCRIPTOR_SIZE,
			crate::Uses::AccelerationStructureBuild,
			crate::DeviceAccesses::HostToDevice,
		);
		let mut creator = self.buffers.creator();

		creator.add(buffer);

		creator.into()
	}

	fn create_top_level_acceleration_structure(
		&mut self,
		name: Option<&str>,
		max_instance_count: u32,
	) -> graphics_hardware_interface::TopLevelAccelerationStructureHandle {
		let sizing = mtl::MTLInstanceAccelerationStructureDescriptor::descriptor();
		sizing.setInstanceCount(max_instance_count as usize);
		sizing.setInstanceDescriptorType(mtl::MTLAccelerationStructureInstanceDescriptorType::Indirect);

		graphics_hardware_interface::TopLevelAccelerationStructureHandle(self.create_acceleration_structure(name, &sizing))
	}

	fn create_bottom_level_acceleration_structure(
		&mut self,
		description: &graphics_hardware_interface::BottomLevelAccelerationStructure,
	) -> graphics_hardware_interface::BottomLevelAccelerationStructureHandle {
		let geometry: Retained<mtl::MTLAccelerationStructureGeometryDescriptor> = match description.description {
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::Mesh {
				vertex_position_encoding,
				triangle_count,
				index_format,
				..
			} => {
				let geometry = mtl::MTLAccelerationStructureTriangleGeometryDescriptor::descriptor();
				geometry.setVertexFormat(to_vertex_format(vertex_position_encoding));
				geometry.setIndexType(to_index_type(index_format));
				geometry.setTriangleCount(triangle_count as usize);
				Retained::into_super(geometry)
			}
			graphics_hardware_interface::BottomLevelAccelerationStructureDescriptions::AABB { transform_count } => {
				let geometry = mtl::MTLAccelerationStructureBoundingBoxGeometryDescriptor::descriptor();
				geometry.setBoundingBoxCount(transform_count as usize);
				Retained::into_super(geometry)
			}
		};
		let sizing = mtl::MTLPrimitiveAccelerationStructureDescriptor::descriptor();
		sizing.setGeometryDescriptors(Some(&NSArray::from_retained_slice(&[geometry])));

		graphics_hardware_interface::BottomLevelAccelerationStructureHandle(self.create_acceleration_structure(None, &sizing))
	}

	fn create_synchronizer(&mut self, _name: Option<&str>, _signaled: bool) -> graphics_hardware_interface::SynchronizerHandle {
		// Metal synchronizers are signaled whenever they have no pending work, so the initial state needs no storage.
		let (master, mut previous) = self.synchronizers.add(synchronizer::Synchronizer::new());

		for _ in 1..self.frames {
			let handle = self.synchronizers.add_with_master(synchronizer::Synchronizer::new(), master);
			self.synchronizers.set_next(previous, Some(handle));
			previous = handle;
		}

		master
	}
}
