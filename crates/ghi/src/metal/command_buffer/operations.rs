use super::*;

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	fn frame_key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		_acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		_acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		// TODO: Map acceleration structure build to MTLAccelerationStructureCommandEncoder.
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		self.end_compute_encoder();

		let render_target_array_length =
			graphics_hardware_interface::AttachmentInformation::render_pass_layer_count(attachments);
		let layered = attachments.first().is_some_and(|attachment| attachment.layer_count.is_some());
		let attachments = attachments
			.iter()
			.map(|attachment| match attachment.target {
				ImageOrSwapchain::Image(image) => {
					let image = self.device.images.resource(self.get_internal_image_handle(image));

					validate_attachment_layer_selection(attachment.layer, attachment.layer_count, image.array_layers);
					(attachment, image.texture.clone(), image.format, image.array_layers)
				}
				ImageOrSwapchain::Swapchain(swapchain) => {
					let drawable = self
						.drawables
						.iter()
						.find(|(handle, _)| *handle == swapchain)
						.expect("Swapchain image not found");

					validate_attachment_layer_selection(attachment.layer, attachment.layer_count, 1);
					(attachment, drawable.1.texture(), crate::Formats::BGRAu8, 1) // TODO: get actual format
				}
			})
			.collect::<SmallVec<[_; 8]>>();

		let rpd = mtl::MTL4RenderPassDescriptor::new();
		if layered {
			rpd.setRenderTargetArrayLength(render_target_array_length as _);
		}

		for (i, (attachment, image, format, array_layers)) in
			attachments.iter().filter(|(_, _, format, _)| !format.is_depth()).enumerate()
		{
			let att = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(i) };
			self.command_buffer.retain_texture(image.clone());
			let texture_view = attachment_texture_view(image, *format, *array_layers, attachment.layer);
			self.command_buffer.retain_texture(texture_view.clone());

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearColor(utils::clear_color(attachment.clear));
		}

		if let Some((attachment, image, format, array_layers)) = attachments.iter().find(|(_, _, format, _)| format.is_depth())
		{
			let att = rpd.depthAttachment();
			self.command_buffer.retain_texture(image.clone());
			let texture_view = attachment_texture_view(image, *format, *array_layers, attachment.layer);
			self.command_buffer.retain_texture(texture_view.clone());

			att.setTexture(Some(texture_view.as_ref()));
			att.setLoadAction(utils::load_action(attachment.load));
			att.setStoreAction(utils::store_action(attachment.store));
			att.setClearDepth(utils::clear_depth(attachment.clear));
		}

		let rce = self.command_buffer.render_command_encoder(&rpd).expect(
			"Metal 4 render command encoder creation failed. The most likely cause is that the command buffer could not start the render pass.",
		);
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			rce.setLabel(Some(&self.next_encoder_block_label()));
			self.push_active_render_debug_regions(rce.as_ref());
		}

		rce.setViewport(mtl::MTLViewport {
			originX: 0.0,
			originY: 0.0,
			width: extent.width() as f64,
			height: extent.height() as f64,
			znear: 0.0,
			zfar: 1.0,
		});
		rce.setScissorRect(mtl::MTLScissorRect {
			x: 0,
			y: 0,
			width: extent.width() as _,
			height: extent.height() as _,
		});

		self.active_render_encoder = Some(rce);
		self.encoded_render_pipeline = None;
		self.applied_render_descriptor_binding = None;
		self.render_push_constants_dirty = !self.push_constant_data.is_empty();
		self.render_vertex_buffers_dirty = !self.bound_vertex_buffers.is_empty();
		self.encoded_vertex_buffer_count = 0;

		self
	}

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		if textures.is_empty() {
			return;
		}

		self.end_compute_encoder();
		self.end_render_encoder();

		let mut batch = SmallVec::<[(ImageHandle, graphics_hardware_interface::ClearValue); 9]>::new();
		let mut batch_extent = None;
		let mut batch_array_layers = 0;
		let mut color_count = 0;
		let mut has_depth = false;

		for (handle, clear_value) in textures {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);
			self.command_buffer.retain_texture(image.texture.clone());
			let is_depth = image.format.is_depth();
			let compatible = batch.is_empty()
				|| (batch_extent == Some(image.extent)
					&& batch_array_layers == image.array_layers
					&& !batch.iter().any(|(resident_handle, _)| *resident_handle == image_handle)
					&& if is_depth { !has_depth } else { color_count < 8 });

			if !compatible {
				self.encode_image_clear_batch(&batch);
				batch.clear();
				color_count = 0;
				has_depth = false;
			}

			if batch.is_empty() {
				batch_extent = Some(image.extent);
				batch_array_layers = image.array_layers;
			}
			batch.push((image_handle, *clear_value));
			if is_depth {
				has_depth = true;
			} else {
				color_count += 1;
			}
		}

		self.encode_image_clear_batch(&batch);
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		if buffer_handles.is_empty() {
			return;
		}

		let transfer_encoder = self.prepare_transfer().clone();

		for buffer_handle in buffer_handles {
			let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(*buffer_handle));
			if buffer.size == 0 {
				continue;
			}
			self.command_buffer.retain_buffer(buffer.buffer.clone());
			unsafe {
				transfer_encoder.fillBuffer_range_value(buffer.buffer.as_ref(), NSRange::new(0, buffer.size), 0);
			}
		}
	}

	fn copy_buffers(&mut self, copies: &[crate::BufferCopyDescriptor]) {
		if !copies.iter().any(|copy| copy.size > 0) {
			return;
		}

		let transfer_encoder = self.prepare_transfer().clone();

		for copy in copies {
			if copy.size == 0 {
				continue;
			}
			let source = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.source_buffer));
			let destination = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.destination_buffer));

			self.command_buffer.retain_buffer(source.buffer.clone());
			self.command_buffer.retain_buffer(destination.buffer.clone());
			unsafe {
				transfer_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
					source.buffer.as_ref(),
					copy.source_offset as _,
					destination.buffer.as_ref(),
					copy.destination_offset as _,
					copy.size as _,
				);
			}
		}
	}

	fn copy_buffer_to_images(&mut self, copies: &[crate::BufferImageCopyDescriptor]) {
		if copies.is_empty() {
			return;
		}

		let transfer_encoder = self.prepare_transfer().clone();

		for copy in copies {
			let source = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.source_buffer));
			let destination = self
				.device
				.images
				.resource(self.get_internal_image_handle(copy.destination_image));
			self.command_buffer.retain_buffer(source.buffer.clone());
			self.command_buffer.retain_texture(destination.texture.clone());
			assert!(
				copy.destination_mip_level < destination.mip_levels,
				"Metal texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image. mip_level={}, mip_levels={}",
				copy.destination_mip_level,
				destination.mip_levels
			);
			let destination_extent = crate::image::mip_extent(destination.extent, copy.destination_mip_level);
			let Some((compact_bytes_per_row, row_count, compact_bytes_per_image)) =
				utils::texture_upload_layout(destination.format, destination_extent)
			else {
				panic!(
					"Metal texture copy layout is unsupported. The most likely cause is that the destination format has no upload layout. format={:?}, extent={:?}",
					destination.format, destination_extent
				);
			};
			let expected_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
			let expected_bytes_per_image = expected_bytes_per_row * row_count;
			assert_eq!(
				copy.source_offset % 256,
				0,
				"Metal texture copy source offset alignment mismatch. The most likely cause is that the staging allocator did not provide a 256-byte aligned texture upload offset. source_offset={}, source_bytes_per_row={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				copy.source_offset,
				copy.source_bytes_per_row,
				copy.source_bytes_per_image,
				destination.format,
				destination.extent
			);
			assert_eq!(
				copy.source_bytes_per_row, expected_bytes_per_row,
				"Metal texture copy row pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about BC block row padding. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_row={}, expected={expected_bytes_per_row}",
				destination.format, destination.extent, copy.source_bytes_per_row
			);
			assert_eq!(
				copy.source_bytes_per_image, expected_bytes_per_image,
				"Metal texture copy image pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about padded rows per image. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_image={}, expected={expected_bytes_per_image}",
				destination.format, destination.extent, copy.source_bytes_per_image
			);
			let required_source_bytes = copy
				.source_bytes_per_image
				.checked_mul(destination.array_layers as usize)
				.and_then(|copy_bytes| copy.source_offset.checked_add(copy_bytes))
				.expect(
					"Metal texture copy source bounds overflowed. The most likely cause is an invalid array layer count or image pitch.",
				);
			assert!(
				required_source_bytes <= source.size,
				"Metal texture copy source buffer is too small. The most likely cause is that the staging buffer allocation is smaller than the recorded texture copy. source_size={}, required_source_bytes={required_source_bytes}, source_offset={}, array_layers={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				source.size,
				copy.source_offset,
				destination.array_layers,
				copy.source_bytes_per_image,
				destination.format,
				destination.extent
			);

			let mut source_size = utils::texture_copy_size(destination.format, destination_extent);
			source_size.depth = 1;
			let destination_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

			for slice in 0..destination.array_layers as usize {
				let source_offset = copy.source_offset + slice * copy.source_bytes_per_image;

				unsafe {
					transfer_encoder.copyFromBuffer_sourceOffset_sourceBytesPerRow_sourceBytesPerImage_sourceSize_toTexture_destinationSlice_destinationLevel_destinationOrigin(
						source.buffer.as_ref(),
						source_offset as _,
						copy.source_bytes_per_row as _,
						copy.source_bytes_per_image as _,
						source_size,
						destination.texture.as_ref(),
						slice,
						copy.destination_mip_level as _,
						destination_origin,
					);
				}
			}
		}
	}

	fn copy_images_to_buffer(&mut self, copies: &[crate::ImageBufferCopyDescriptor]) {
		if copies.is_empty() {
			return;
		}

		let transfer_encoder = self.prepare_transfer().clone();

		for copy in copies {
			let (source_texture, source_format, source_extent, source_array_layers) = match copy.source {
				ImageOrSwapchain::Image(image) => {
					let source = self.device.images.resource(self.get_internal_image_handle(image));
					(source.texture.clone(), source.format, source.extent, source.array_layers)
				}
				ImageOrSwapchain::Swapchain(swapchain) => {
					if let Some(proxy) = self.device.swapchains[swapchain.0 as usize].images[self.sequence_index as usize] {
						let source = self.device.images.resource(proxy);
						(source.texture.clone(), source.format, source.extent, source.array_layers)
					} else {
						(
							self.drawable_texture(crate::swapchain::SwapchainHandle(swapchain.0)),
							crate::Formats::BGRAu8,
							self.device.swapchains[swapchain.0 as usize].extent,
							1,
						)
					}
				}
			};
			let destination = self
				.device
				.buffers
				.resource(self.get_internal_buffer_handle(copy.destination_buffer));
			self.command_buffer.retain_texture(source_texture.clone());
			self.command_buffer.retain_buffer(destination.buffer.clone());
			let Some((compact_bytes_per_row, row_count, _)) = utils::texture_upload_layout(source_format, source_extent) else {
				panic!(
					"Metal texture copy layout is unsupported. The most likely cause is that the source format has no buffer copy layout. format={source_format:?}, extent={source_extent:?}"
				);
			};
			let expected_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
			let expected_bytes_per_image = expected_bytes_per_row * row_count;
			assert_eq!(
				copy.destination_offset % 256,
				0,
				"Metal image copy destination offset alignment mismatch. The most likely cause is that the destination buffer offset is not 256-byte aligned. destination_offset={}, destination_bytes_per_row={}, destination_bytes_per_image={}, format={source_format:?}, extent={source_extent:?}",
				copy.destination_offset,
				copy.destination_bytes_per_row,
				copy.destination_bytes_per_image,
			);
			assert_eq!(
				copy.destination_bytes_per_row, expected_bytes_per_row,
				"Metal image copy row pitch mismatch. The most likely cause is that readback preparation and Metal copy recording disagree about row padding. format={source_format:?}, extent={source_extent:?}, compact_bytes_per_row={compact_bytes_per_row}, row_count={row_count}, destination_bytes_per_row={}, expected={expected_bytes_per_row}",
				copy.destination_bytes_per_row
			);
			assert_eq!(
				copy.destination_bytes_per_image, expected_bytes_per_image,
				"Metal image copy image pitch mismatch. The most likely cause is that readback preparation and Metal copy recording disagree about padded rows per image. format={source_format:?}, extent={source_extent:?}, compact_bytes_per_row={compact_bytes_per_row}, row_count={row_count}, destination_bytes_per_image={}, expected={expected_bytes_per_image}",
				copy.destination_bytes_per_image
			);
			let required_destination_bytes = copy
				.destination_bytes_per_image
				.checked_mul(source_array_layers as usize)
				.and_then(|copy_bytes| copy.destination_offset.checked_add(copy_bytes))
				.expect(
					"Metal image copy destination bounds overflowed. The most likely cause is an invalid array layer count or image pitch.",
				);
			assert!(
				required_destination_bytes <= destination.size,
				"Metal image copy destination buffer is too small. The most likely cause is that the readback buffer allocation is smaller than the recorded texture copy. destination_size={}, required_destination_bytes={required_destination_bytes}, destination_offset={}, array_layers={source_array_layers}, destination_bytes_per_image={}, format={source_format:?}, extent={source_extent:?}",
				destination.size,
				copy.destination_offset,
				copy.destination_bytes_per_image,
			);

			let mut source_size = utils::texture_copy_size(source_format, source_extent);
			source_size.depth = 1;
			let source_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

			for slice in 0..source_array_layers as usize {
				let destination_offset = copy.destination_offset + slice * copy.destination_bytes_per_image;
				unsafe {
					transfer_encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
						source_texture.as_ref(),
						slice as _,
						0,
						source_origin,
						source_size,
						destination.buffer.as_ref(),
						destination_offset as _,
						copy.destination_bytes_per_row as _,
						copy.destination_bytes_per_image as _,
					);
				}
			}
		}
	}

	fn transfer_textures(
		&mut self,
		texture_handles: &[graphics_hardware_interface::BaseImageHandle],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		let mut copies = Vec::with_capacity(texture_handles.len());

		for handle in texture_handles {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);
			if !image.access.contains(crate::DeviceAccesses::CpuRead) {
				continue;
			}

			// Match Vulkan: the copy handle identifies the internal image whose shared readback buffer receives the copy.
			copies.push(graphics_hardware_interface::TextureCopyHandle(image_handle.0));
		}

		copies
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle);

		let image = self.device.images.resource(image_handle);

		let Some(_) = image.staging.as_ref() else {
			return;
		};

		// Metal accepts a CPU pointer for immediate texture replacement, so the caller-provided
		// pixel slice can be used directly instead of cloning through the image staging Vec.
		let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(data)) };

		let texture = image.texture.clone();
		let format = image.format;
		let extent = image.extent;
		let array_layers = image.array_layers;

		replace_texture_from_bytes(texture.as_ref(), format, extent, array_layers, bytes);
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		_source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		_destination_layout: crate::Layouts,
	) {
		let source_internal = self.get_internal_image_handle(source_image);
		let destination_internal = self.get_internal_image_handle(destination_image);

		let source_texture = self.device.images.resource(source_internal).texture.clone();
		let destination_texture = self.device.images.resource(destination_internal).texture.clone();
		self.command_buffer.retain_texture(source_texture.clone());
		self.command_buffer.retain_texture(destination_texture.clone());
		let transfer_encoder = self.prepare_transfer().clone();

		unsafe {
			transfer_encoder.copyFromTexture_toTexture(source_texture.as_ref(), destination_texture.as_ref());
		}
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		CommandBufferRecording::sync_buffer(self, buffer_handle);
	}

	fn execute(self, _synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.finish(_synchronizer);
	}
}

impl CommonCommandBufferMode for CommandBufferRecording<'_> {
	fn bind_compute_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundComputePipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline_layout = self.device.pipelines[pipeline_handle.0 as usize].layout;
		if self.active_pipeline_layout != Some(pipeline_layout) {
			self.active_pipeline_layout = Some(pipeline_layout);
			self.resize_push_constants_for_layout(pipeline_layout);
		}

		self
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRayTracingPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);
		self.active_pipeline_layout = Some(self.device.pipelines[pipeline_handle.0 as usize].layout);
		self
	}

	fn start_region(&mut self, _write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result) {
		#[cfg(debug_assertions)]
		let write_label = _write_label;
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			let mut label = crate::command_buffer::DebugLabelWriter::new();
			write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");
			let name = label.as_str();
			let name = NSString::from_str(name);

			if let Some(encoder) = self.active_compute_encoder.as_ref() {
				encoder.pushDebugGroup(&name);
				self.compute_debug_region_depth += 1;
			}
			if let Some(encoder) = self.active_render_encoder.as_ref() {
				encoder.pushDebugGroup(&name);
				self.render_debug_region_depth += 1;
			}
			self.debug_regions.push(name);
		}
	}

	fn end_region(&mut self) {
		#[cfg(debug_assertions)]
		if self.device.debug_labels {
			self.debug_regions.pop().expect(
				"Unbalanced Metal debug region. The most likely cause is that end_region was called without start_region.",
			);

			if let Some(encoder) = self.active_compute_encoder.as_ref() {
				encoder.popDebugGroup();
				self.compute_debug_region_depth -= 1;
			}
			if let Some(encoder) = self.active_render_encoder.as_ref() {
				encoder.popDebugGroup();
				self.render_debug_region_depth -= 1;
			}
		}
	}

	fn region(
		&mut self,
		write_label: impl FnOnce(&mut crate::command_buffer::DebugLabelWriter) -> std::fmt::Result,
		f: impl FnOnce(&mut Self),
	) {
		self.start_region(write_label);
		f(self);
		self.end_region();
	}
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRasterizationPipelineMode {
		self.bound_pipeline = Some(pipeline_handle);

		let pipeline_layout = self.device.pipelines[pipeline_handle.0 as usize].layout;

		if self.active_pipeline_layout != Some(pipeline_layout) {
			self.active_pipeline_layout = Some(pipeline_layout);
			self.resize_push_constants_for_layout(pipeline_layout);
		}

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[crate::BufferDescriptor]) {
		assert!(
			buffer_descriptors.len() <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal vertex buffers were bound. The most likely cause is that ordinary vertex bindings overlap the reserved push-constant or argument-buffer slots."
		);
		let bindings = buffer_descriptors
			.iter()
			.map(|buffer_descriptor| (buffer_descriptor.buffer, buffer_descriptor.offset))
			.collect::<SmallVec<[_; 8]>>();
		if self.bound_vertex_buffers != bindings {
			self.bound_vertex_buffers = bindings;
			self.render_vertex_buffers_dirty = true;
		}
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &crate::BufferDescriptor) {
		let index_type = buffer_descriptor.index_type.expect(
			"Missing index buffer type. The most likely cause is that bind_index_buffer was called with a BufferDescriptor that did not specify index_type(DataTypes::U16) or index_type(DataTypes::U32).",
		);

		self.bound_index_buffer = Some((buffer_descriptor.buffer, buffer_descriptor.offset, index_type));
	}

	fn end_render_pass(&mut self) {
		self.end_render_encoder();
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		self.active_pipeline_layout.expect(
			"No pipeline layout is active. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		self.bound_pipeline.expect(
			"No pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		// Binding replaces the complete flat set union; native argument-buffer work is deferred until execution.
		self.update_bound_descriptor_sets(sets);
		self
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, offset: u32, data: T)
	where
		[(); std::mem::size_of::<T>()]: Sized,
	{
		let pipeline_layout_handle = self.active_pipeline_layout.expect(
			"No pipeline bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let pipeline_layout = &self.device.pipeline_layouts[pipeline_layout_handle.0 as usize];
		let end = offset as usize + std::mem::size_of::<T>();

		assert!(
			end <= pipeline_layout.push_constant_size,
			"Push constant write exceeds the Metal pipeline layout push constant storage. The most likely cause is that the write offset or type size does not match the pipeline's declared push constant ranges.",
		);

		if self.push_constant_data.len() < pipeline_layout.push_constant_size {
			self.resize_push_constants_for_layout(pipeline_layout_handle);
		}

		unsafe {
			std::ptr::copy_nonoverlapping(
				&data as *const T as *const u8,
				self.push_constant_data[offset as usize..end].as_mut_ptr(),
				std::mem::size_of::<T>(),
			);
		}

		self.compute_push_constants_dirty = true;
		self.render_push_constants_dirty = true;
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		self.prepare_render_draw();
		self.flush_render_push_constants();
		let mesh_index = mesh_handle.0 as usize;
		let vertex_buffer_count = self.device.meshes[mesh_index].vertex_buffers.len();
		assert!(
			vertex_buffer_count <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal mesh vertex buffers were bound. The most likely cause is that mesh bindings overlap the reserved push-constant or argument-buffer slots."
		);

		// Metal 4 snapshots mesh vertex addresses through the shared stage argument table ABI.
		let binding_count = vertex_buffer_count.max(self.encoded_vertex_buffer_count);
		for binding in 0..binding_count {
			let vertex_buffer = self.device.meshes[mesh_index].vertex_buffers.get(binding).cloned().flatten();
			let address = vertex_buffer.as_ref().map_or(0, |vertex_buffer| vertex_buffer.gpuAddress());
			if let Some(vertex_buffer) = vertex_buffer {
				self.command_buffer.retain_buffer(vertex_buffer);
			}
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, address);
		}

		let mesh = &self.device.meshes[mesh_index];
		let index_buffer = mesh.index_buffer.clone();
		let index_count = mesh.index_count;
		let index_buffer_address = index_buffer.gpuAddress();
		let index_buffer_length = index_buffer.length();
		self.command_buffer.retain_buffer(index_buffer);
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that draw_mesh was called outside start_render_pass.");

		unsafe {
			encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferLength(
				mtl::MTLPrimitiveType::Triangle,
				index_count as _,
				mtl::MTLIndexType::UInt16,
				index_buffer_address,
				index_buffer_length,
			);
		}
		self.encoded_vertex_buffer_count = vertex_buffer_count;
		// Mesh-owned bindings replace the ordinary logical bindings even when that logical list is empty.
		self.render_vertex_buffers_dirty = true;
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		self.prepare_render_draw();
		self.apply_bound_vertex_buffers();
		self.flush_render_push_constants();
		unsafe {
			self.active_render_encoder
				.as_ref()
				.expect("No active render pass. The most likely cause is that draw was called outside start_render_pass.")
				.drawPrimitives_vertexStart_vertexCount_instanceCount_baseInstance(
					mtl::MTLPrimitiveType::Triangle,
					first_vertex as _,
					vertex_count as _,
					instance_count as _,
					first_instance as _,
				);
		}
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		self.prepare_render_draw();
		self.apply_bound_vertex_buffers();
		self.flush_render_push_constants();
		let (buffer_handle, offset, index_type) = self
			.bound_index_buffer
			.expect("No index buffer bound. The most likely cause is that draw_indexed was called before bind_index_buffer.");
		let buffer = self.device.buffers.resource(self.get_internal_buffer_handle(buffer_handle));
		let (metal_index_type, index_size) = match index_type {
			crate::DataTypes::U16 => (mtl::MTLIndexType::UInt16, std::mem::size_of::<u16>()),
			crate::DataTypes::U32 => (mtl::MTLIndexType::UInt32, std::mem::size_of::<u32>()),
			_ => panic!(
				"Unsupported index buffer type. The most likely cause is that bind_index_buffer was given a DataTypes value other than U16 or U32."
			),
		};
		let first_index_offset = (first_index as usize).checked_mul(index_size).expect(
			"Metal indexed draw offset overflowed. The most likely cause is that first_index exceeds the host address range.",
		);
		let index_buffer_offset = offset.checked_add(first_index_offset).expect(
			"Metal indexed draw offset overflowed. The most likely cause is that the bound offset and first_index exceed the host address range.",
		);
		// Metal 4 measures the accessible index range from the shifted GPU address, not from the buffer allocation's start.
		let index_buffer_length = buffer.size.checked_sub(index_buffer_offset).expect(
			"Metal indexed draw starts past the index buffer. The most likely cause is that the bound offset or first_index exceeds the buffer size.",
		);
		let index_buffer_address = buffer.gpu_address.checked_add(index_buffer_offset as u64).expect(
			"Metal index-buffer GPU address overflowed. The most likely cause is that the bound index range exceeds the native address space.",
		);
		self.command_buffer.retain_buffer(buffer.buffer.clone());

		unsafe {
			self.active_render_encoder
				.as_ref()
				.expect(
					"No active render pass. The most likely cause is that draw_indexed was called outside start_render_pass.",
				)
				.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferLength_instanceCount_baseVertex_baseInstance(
					mtl::MTLPrimitiveType::Triangle,
					index_count as _,
					metal_index_type,
					index_buffer_address,
					index_buffer_length as _,
					instance_count as _,
					vertex_offset as _,
					first_instance as _,
				);
		}
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		self.prepare_render_draw();
		self.flush_render_push_constants();
		let bound_pipeline = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that dispatch_meshes was called before bind_raster_pipeline.");
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let mesh_threadgroup_size = pipeline.mesh_threadgroup_size.expect(
			"Metal mesh dispatch requires mesh threadgroup metadata. The most likely cause is that the mesh shader was not generated with Metal mesh threadgroup size metadata.",
		);
		let object_threadgroup_size = pipeline.object_threadgroup_size.unwrap_or(Extent::new(1, 1, 1));

		self.active_render_encoder
			.as_ref()
			.expect(
				"No active render pass. The most likely cause is that dispatch_meshes was called outside start_render_pass.",
			)
			.drawMeshThreadgroups_threadsPerObjectThreadgroup_threadsPerMeshThreadgroup(
				mtl::MTLSize {
					width: x as _,
					height: y as _,
					depth: z as _,
				},
				mtl::MTLSize {
					width: object_threadgroup_size.width() as _,
					height: object_threadgroup_size.height() as _,
					depth: object_threadgroup_size.depth() as _,
				},
				mtl::MTLSize {
					width: mesh_threadgroup_size.width() as _,
					height: mesh_threadgroup_size.height() as _,
					depth: mesh_threadgroup_size.depth() as _,
				},
			);
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let threadgroups = dispatch.get_extent();
		let threads_per_threadgroup = dispatch.get_workgroup_extent();
		self.prepare_compute_dispatch();
		self.flush_compute_push_constants();

		self.ensure_compute_encoder().dispatchThreadgroups_threadsPerThreadgroup(
			mtl::MTLSize {
				width: threadgroups.width() as _,
				height: threadgroups.height() as _,
				depth: threadgroups.depth() as _,
			},
			mtl::MTLSize {
				width: threads_per_threadgroup.width().max(1) as _,
				height: threads_per_threadgroup.height().max(1) as _,
				depth: threads_per_threadgroup.depth().max(1) as _,
			},
		);
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<[[u32; 3]; N]>,
		entry_index: usize,
	) {
		assert!(
			entry_index < N,
			"Metal indirect dispatch entry is out of bounds. The most likely cause is that entry_index exceeds the typed indirect buffer length. entry_index={entry_index}, entry_count={N}",
		);
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle.into());
		let buffer = self.device.buffers.resource(internal_buffer);
		let indirect_offset = entry_index.checked_mul(std::mem::size_of::<[u32; 3]>()).expect(
			"Metal indirect dispatch offset overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		let indirect_end = indirect_offset.checked_add(std::mem::size_of::<[u32; 3]>()).expect(
			"Metal indirect dispatch range overflowed. The most likely cause is that entry_index exceeds the host address range.",
		);
		assert!(
			indirect_end <= buffer.size,
			"Metal indirect dispatch entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={indirect_end}, buffer_size={}",
			buffer.size,
		);
		let indirect_buffer_address = buffer.gpu_address.checked_add(indirect_offset as u64).expect(
			"Metal indirect dispatch GPU address overflowed. The most likely cause is that the selected entry exceeds the native address space.",
		);
		self.command_buffer.retain_buffer(buffer.buffer.clone());

		self.prepare_compute_dispatch();
		self.flush_compute_push_constants();

		let bound_pipeline = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that indirect_dispatch was called before bind_compute_pipeline.",
		);
		let pipeline = &self.device.pipelines[bound_pipeline.0 as usize];
		let threadgroup_extent = pipeline.compute_threadgroup_size.unwrap_or(Extent::line(128));

		unsafe {
			self.ensure_compute_encoder()
				.dispatchThreadgroupsWithIndirectBuffer_threadsPerThreadgroup(
					indirect_buffer_address,
					mtl::MTLSize {
						width: threadgroup_extent.width().max(1) as _,
						height: threadgroup_extent.height().max(1) as _,
						depth: threadgroup_extent.depth().max(1) as _,
					},
				);
		}
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, _x: u32, _y: u32, _z: u32) {
		// TODO: Encode Metal ray tracing dispatch.
	}
}
