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

		let scope = self.allocate_encoder_scope();
		self.active_encoder_scope = Some(scope);
		self.active_render_encoder = Some(rce);
		let mut initial_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		let mut final_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		for (attachment, texture, ..) in &attachments {
			let resource_use = |access| match attachment.target {
				ImageOrSwapchain::Image(image) => synchronization::MetalResourceUse::image(
					self.get_internal_image_handle(image),
					Some(0),
					attachment.layer,
					mtl::MTLStages::Fragment,
					access,
				),
				ImageOrSwapchain::Swapchain(_) => {
					synchronization::MetalResourceUse::drawable(texture.as_ref(), mtl::MTLStages::Fragment, access)
				}
			};
			let initial_access = crate::AccessPolicies::WRITE
				| if attachment.load {
					crate::AccessPolicies::READ
				} else {
					crate::AccessPolicies::NONE
				};
			initial_attachment_uses.push(resource_use(initial_access));
			final_attachment_uses.push(resource_use(crate::AccessPolicies::WRITE));
		}
		self.consume_render_resources(initial_attachment_uses);
		self.active_render_attachment_uses = final_attachment_uses;

		let rce = self.active_render_encoder.as_ref().expect(
			"Metal 4 render encoder setup failed. The most likely cause is that attachment synchronization ended the encoder early.",
		);
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
			let handle = self.get_internal_buffer_handle(*buffer_handle);
			let (buffer, size) = {
				let buffer = self.device.buffers.resource(handle);
				(buffer.buffer.clone(), buffer.size)
			};
			if size == 0 {
				continue;
			}
			self.command_buffer.retain_buffer(buffer.clone());
			self.consume_compute_resources([synchronization::MetalResourceUse::buffer(
				handle,
				0,
				size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			)]);
			unsafe {
				transfer_encoder.fillBuffer_range_value(buffer.as_ref(), NSRange::new(0, size), 0);
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
			let source_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_handle = self.get_internal_buffer_handle(copy.destination_buffer);
			let source = self.device.buffers.resource(source_handle).buffer.clone();
			let destination = self.device.buffers.resource(destination_handle).buffer.clone();

			self.command_buffer.retain_buffer(source.clone());
			self.command_buffer.retain_buffer(destination.clone());
			self.consume_compute_resources([
				synchronization::MetalResourceUse::buffer(
					source_handle,
					copy.source_offset,
					copy.size,
					mtl::MTLStages::Blit,
					crate::AccessPolicies::READ,
				),
				synchronization::MetalResourceUse::buffer(
					destination_handle,
					copy.destination_offset,
					copy.size,
					mtl::MTLStages::Blit,
					crate::AccessPolicies::WRITE,
				),
			]);
			unsafe {
				transfer_encoder.copyFromBuffer_sourceOffset_toBuffer_destinationOffset_size(
					source.as_ref(),
					copy.source_offset as _,
					destination.as_ref(),
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
			let source_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_handle = self.get_internal_image_handle(copy.destination_image);
			let source_size = copy
				.source_bytes_per_image
				.checked_mul(self.device.images.resource(destination_handle).array_layers as usize)
				.expect(
					"Metal texture copy tracked range overflowed. The most likely cause is an invalid source pitch or array layer count.",
				);
			self.consume_compute_resources([
				synchronization::MetalResourceUse::buffer(
					source_handle,
					copy.source_offset,
					source_size,
					mtl::MTLStages::Blit,
					crate::AccessPolicies::READ,
				),
				synchronization::MetalResourceUse::image(
					destination_handle,
					Some(copy.destination_mip_level),
					None,
					mtl::MTLStages::Blit,
					crate::AccessPolicies::WRITE,
				),
			]);
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

	fn transfer_texture(
		&mut self,
		source: graphics_hardware_interface::ImageOrSwapchain,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		let (source_use, source_texture, format, extent, array_layers, uses) = match source {
			ImageOrSwapchain::Image(image) => {
				if self.device.images.get_single(image).is_none() {
					return Err(crate::TextureTransferError::InvalidSource);
				}
				let handle = self.get_internal_image_handle(image);
				let source = self.device.images.resource(handle);
				(
					synchronization::MetalResourceUse::image(
						handle,
						Some(0),
						None,
						mtl::MTLStages::Blit,
						crate::AccessPolicies::READ,
					),
					source.texture.clone(),
					source.format,
					source.extent,
					source.array_layers,
					source.uses,
				)
			}
			ImageOrSwapchain::Swapchain(swapchain) => {
				let swapchain_resource = self
					.device
					.swapchains
					.get(swapchain.0 as usize)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
				if !swapchain_resource.uses.contains(crate::Uses::TransferSource) {
					return Err(crate::TextureTransferError::MissingTransferSource);
				}
				if let Some(proxy) = swapchain_resource.images[self.sequence_index as usize] {
					let source = self.device.images.resource(proxy);
					(
						synchronization::MetalResourceUse::image(
							proxy,
							Some(0),
							None,
							mtl::MTLStages::Blit,
							crate::AccessPolicies::READ,
						),
						source.texture.clone(),
						source.format,
						source.extent,
						source.array_layers,
						swapchain_resource.uses,
					)
				} else {
					let drawable = self
						.drawables
						.iter()
						.find(|(handle, _)| *handle == swapchain)
						.map(|(_, drawable)| drawable.texture())
						.ok_or(crate::TextureTransferError::InvalidSource)?;
					(
						synchronization::MetalResourceUse::drawable(
							drawable.as_ref(),
							mtl::MTLStages::Blit,
							crate::AccessPolicies::READ,
						),
						drawable,
						crate::Formats::BGRAu8,
						swapchain_resource.extent,
						1,
						swapchain_resource.uses,
					)
				}
			}
		};
		let layout = crate::context::texture_transfer_layout(format, extent, array_layers, uses)?;
		let bytes_per_row = layout.bytes_per_row;
		let row_count = layout.row_count;
		let bytes_per_image = layout.bytes_per_image;
		let native_bytes_per_row = bytes_per_row
			.checked_add(255)
			.map(|bytes| bytes & !255)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let native_bytes_per_image = native_bytes_per_row
			.checked_mul(row_count)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let size = native_bytes_per_image;
		let compact_size = bytes_per_image;
		let mut bytes = Vec::new();
		bytes
			.try_reserve_exact(compact_size)
			.map_err(|_| crate::TextureTransferError::AllocationFailed)?;
		bytes.resize(compact_size, 0);
		let staging = self
			.device
			.metal_device
			.newBufferWithLength_options(size, mtl::MTLResourceOptions::StorageModeShared)
			.ok_or(crate::TextureTransferError::AllocationFailed)?;

		let transfer_encoder = self.prepare_transfer().clone();
		self.consume_compute_resources([source_use]);
		self.command_buffer.retain_texture(source_texture.clone());
		self.command_buffer.retain_buffer(staging.clone());
		let mut source_size = utils::texture_copy_size(format, extent);
		source_size.depth = 1;
		let source_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };
		for slice in 0..array_layers as usize {
			unsafe {
				transfer_encoder.copyFromTexture_sourceSlice_sourceLevel_sourceOrigin_sourceSize_toBuffer_destinationOffset_destinationBytesPerRow_destinationBytesPerImage(
					source_texture.as_ref(),
					slice,
					0,
					source_origin,
					source_size,
					staging.as_ref(),
					(slice * native_bytes_per_image) as _,
					native_bytes_per_row as _,
					native_bytes_per_image as _,
				);
			}
		}

		let handle = self.commit.texture_readbacks.insert(context::TextureReadbackStorage {
			buffer: staging,
			bytes,
			extent,
			format,
			bytes_per_row,
			bytes_per_image,
			native_bytes_per_row,
			native_bytes_per_image,
			row_count,
			image_count: 1,
		});
		self.texture_readbacks.push(handle);
		Ok(handle)
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle);

		let (texture, format, extent, array_layers, has_staging) = {
			let image = self.device.images.resource(image_handle);
			(
				image.texture.clone(),
				image.format,
				image.extent,
				image.array_layers,
				image.staging.is_some(),
			)
		};
		if !has_staging || utils::texture_upload_layout(format, extent).is_none() {
			return;
		}

		// The upload buffer snapshots caller memory now; the tracked blit performs the GPU-visible write in command order.
		let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data)) };
		self.command_buffer.retain_texture(texture.clone());
		let transfer_encoder = self.prepare_transfer().clone();
		self.consume_compute_resources([synchronization::MetalResourceUse::image(
			image_handle,
			Some(0),
			None,
			mtl::MTLStages::Blit,
			crate::AccessPolicies::WRITE,
		)]);
		let upload_buffer = encode_texture_upload(
			self.device.metal_device,
			transfer_encoder.as_ref(),
			texture.as_ref(),
			format,
			extent,
			array_layers,
			bytes,
		)
		.expect(
			"Metal image-data upload layout disappeared. The most likely cause is that format validation and upload encoding used different image metadata.",
		);
		self.command_buffer.retain_buffer(upload_buffer);
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
		self.consume_compute_resources([
			synchronization::MetalResourceUse::image(
				source_internal,
				None,
				None,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::READ,
			),
			synchronization::MetalResourceUse::image(
				destination_internal,
				None,
				None,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			),
		]);

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
		self.prepare_render_draw([]);
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
		self.record_render_attachment_writes();
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		self.apply_bound_vertex_buffers();
		let resource_uses = self.bound_vertex_resource_uses();
		self.prepare_render_draw(resource_uses);
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
		self.record_render_attachment_writes();
	}

	fn draw_indexed(
		&mut self,
		index_count: u32,
		instance_count: u32,
		first_index: u32,
		vertex_offset: i32,
		first_instance: u32,
	) {
		self.apply_bound_vertex_buffers();
		let (buffer_handle, offset, index_type) = self
			.bound_index_buffer
			.expect("No index buffer bound. The most likely cause is that draw_indexed was called before bind_index_buffer.");
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle);
		let (buffer_size, buffer_gpu_address, native_buffer) = {
			let buffer = self.device.buffers.resource(internal_buffer);
			(buffer.size, buffer.gpu_address, buffer.buffer.clone())
		};
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
		let index_data_size = (index_count as usize).checked_mul(index_size).expect(
			"Metal indexed draw range overflowed. The most likely cause is that index_count exceeds the host address range.",
		);
		let index_data_end = index_buffer_offset.checked_add(index_data_size).expect(
			"Metal indexed draw range overflowed. The most likely cause is that the index offset and count exceed the host address range.",
		);

		assert!(
			index_data_end <= buffer_size,
			"Metal indexed draw exceeds the index buffer. The most likely cause is that the bound offset, first index, or index count exceeds the buffer size. range_end={index_data_end}, buffer_size={buffer_size}",
		);
		// Metal 4 measures the accessible index range from the shifted GPU address, not from the buffer allocation's start.
		let index_buffer_length = buffer_size - index_buffer_offset;
		let index_buffer_address = buffer_gpu_address.checked_add(index_buffer_offset as u64).expect(
			"Metal index-buffer GPU address overflowed. The most likely cause is that the bound index range exceeds the native address space.",
		);
		self.command_buffer.retain_buffer(native_buffer);

		let mut resource_uses = self.bound_vertex_resource_uses();
		if index_data_size > 0 {
			resource_uses.push(synchronization::MetalResourceUse::buffer(
				internal_buffer,
				index_buffer_offset,
				index_data_size,
				mtl::MTLStages::Vertex,
				crate::AccessPolicies::READ,
			));
		}
		self.prepare_render_draw(resource_uses);
		self.flush_render_push_constants();

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
		self.record_render_attachment_writes();
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		self.prepare_render_draw([]);
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
		self.record_render_attachment_writes();
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let threadgroups = dispatch.get_extent();
		let threads_per_threadgroup = dispatch.get_workgroup_extent();
		self.prepare_compute_dispatch([]);
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

		self.prepare_compute_dispatch([synchronization::MetalResourceUse::buffer(
			internal_buffer,
			indirect_offset,
			std::mem::size_of::<[u32; 3]>(),
			mtl::MTLStages::Dispatch,
			crate::AccessPolicies::READ,
		)]);
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
