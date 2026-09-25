use super::*;
use crate::metal::context::resources::acceleration_structures;

impl CommandBufferRecording<'_> {
	/// Resolves one public transfer source to the retained Metal texture and synchronization use recorded by a copy.
	///
	/// `frame_offset` selects another frame's copy of a per-frame image. Swapchains only have this frame's image.
	fn resolve_transfer_texture_source(
		&self,
		source: graphics_hardware_interface::ImageOrSwapchain,
		frame_offset: i32,
	) -> Result<
		(
			synchronization::MetalResourceUse,
			Retained<ProtocolObject<dyn mtl::MTLTexture>>,
			crate::Formats,
			Extent,
			u32,
			crate::Uses,
		),
		crate::TextureTransferError,
	> {
		Ok(match source {
			ImageOrSwapchain::Image(image) => {
				if self.device.images.get_single(image).is_none() {
					return Err(crate::TextureTransferError::InvalidSource);
				}
				let frame_index = crate::frame_resources::frame_index_with_offset(
					self.sequence_index as usize,
					frame_offset,
					self.device.frames as usize,
				);
				let handle = self
					.device
					.images
					.nth_handle(image, frame_index)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
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
					source.description.format,
					source.description.extent,
					source.description.array_layers,
					source.description.uses,
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
						source.description.format,
						source.description.extent,
						source.description.array_layers,
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
		})
	}

	/// Records one copy of a transfer source, or another frame's copy of a per-frame image, into shared staging.
	fn record_texture_transfer(
		&mut self,
		source: graphics_hardware_interface::ImageOrSwapchain,
		frame_offset: i32,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		let (source_use, source_texture, format, extent, array_layers, uses) =
			self.resolve_transfer_texture_source(source, frame_offset)?;
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
		let size = native_bytes_per_image
			.checked_mul(layout.depth_slices)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let compact_size = bytes_per_image
			.checked_mul(layout.depth_slices)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
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
		#[cfg(debug_assertions)]
		if let Some(label) = source_texture.label() {
			staging.setLabel(Some(&NSString::from_str(&format!("{label} Readback"))));
		}

		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([source_use]);
		self.command_buffer.retain_allocation(source_texture.clone());
		self.command_buffer.retain_allocation(staging.clone());
		let source_size = utils::mtl_size(extent);
		let source_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };
		for slice in 0..array_layers as usize {
			// SAFETY: The source subresource and readback buffer layout cover this array slice.
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
			image_count: layout.depth_slices,
		});
		self.texture_readbacks.push(handle);
		Ok(handle)
	}
}

/// The `AccelerationStructureRange` struct pairs the Metal address range for one build input with its tracked access.
struct AccelerationStructureRange {
	range: mtl::MTL4BufferRange,
	use_: synchronization::MetalResourceUse,
}

impl CommandBufferRecording<'_> {
	/// Resolves one acceleration-structure build input to a Metal address range and retains its buffer.
	///
	/// Metal 4 reads build inputs by GPU address, so the buffer is declared resident here rather than bound.
	fn resolve_acceleration_structure_buffer(
		&mut self,
		buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		offset: usize,
		size: usize,
		access: crate::AccessPolicies,
	) -> AccelerationStructureRange {
		let handle = self.get_internal_buffer_handle(buffer_handle);
		let buffer = self.device.buffers.resource(handle);

		assert!(
			offset + size <= buffer.size,
			"Metal acceleration structure build range exceeds its buffer. The most likely cause is that the build description declares more geometry than the buffer holds. range_end={}, buffer_size={}",
			offset + size,
			buffer.size,
		);
		let address = buffer.gpu_address.checked_add(offset as u64).expect(
			"Metal acceleration structure build address overflowed. The most likely cause is that the build offset exceeds the native address space.",
		);
		let native_buffer = buffer.buffer.clone();

		self.command_buffer.retain_allocation(native_buffer);

		AccelerationStructureRange {
			range: mtl::MTL4BufferRange {
				bufferAddress: address,
				length: size as u64,
			},
			use_: synchronization::MetalResourceUse::buffer(
				handle,
				offset,
				size,
				mtl::MTLStages::AccelerationStructure,
				access,
			),
		}
	}

	/// Resolves one strided geometry range from a build description.
	fn resolve_acceleration_structure_strided_range(
		&mut self,
		range: &crate::BufferStridedRange,
		access: crate::AccessPolicies,
	) -> AccelerationStructureRange {
		self.resolve_acceleration_structure_buffer(range.buffer_offset.buffer, range.buffer_offset.offset, range.size, access)
	}

	/// Encodes one acceleration-structure build into the recording's compute encoder.
	///
	/// Metal 4 builds acceleration structures on the compute timeline, so this shares the encoder and barrier
	/// tracking every other compute command in the recording uses.
	fn encode_acceleration_structure_build(
		&mut self,
		structure_index: usize,
		descriptor: &mtl::MTL4AccelerationStructureDescriptor,
		scratch_buffer: &crate::BufferDescriptor,
		input_uses: impl IntoIterator<Item = synchronization::MetalResourceUse>,
	) {
		let (structure, build_scratch_size) = {
			let acceleration_structure = &self.device.acceleration_structures[structure_index];
			(
				acceleration_structure.structure.clone(),
				acceleration_structure.build_scratch_size,
			)
		};
		let scratch = self.resolve_acceleration_structure_buffer(
			scratch_buffer.buffer,
			scratch_buffer.offset,
			build_scratch_size,
			crate::AccessPolicies::WRITE,
		);
		self.command_buffer.retain_allocation(structure.clone());

		let encoder = self.ensure_compute_encoder().clone();

		self.consume_resources(input_uses.into_iter().chain([
			scratch.use_,
			synchronization::MetalResourceUse::acceleration_structure(
				structure_index,
				mtl::MTLStages::AccelerationStructure,
				crate::AccessPolicies::WRITE,
			),
		]));

		// SAFETY: The structure was sized for this descriptor's geometry at creation, the scratch range covers the
		// size Metal reported for that build, and every referenced buffer is retained and resident.
		unsafe {
			encoder.buildAccelerationStructure_descriptor_scratchBuffer(structure.as_ref(), descriptor, scratch.range);
		}
	}
}

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	fn frame_key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		let crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
			instances_buffer,
			instance_count,
		} = acceleration_structure_build.description;

		let structure_index = acceleration_structure_build.acceleration_structure.0 as usize;
		let instance_count = instance_count as usize;
		let instances = self.resolve_acceleration_structure_buffer(
			instances_buffer,
			0,
			instance_count * acceleration_structures::INSTANCE_DESCRIPTOR_SIZE,
			crate::AccessPolicies::READ,
		);

		let descriptor = mtl::MTL4InstanceAccelerationStructureDescriptor::new();
		// SAFETY: The instance range was bounds-checked against the instance buffer that backs it.
		unsafe {
			descriptor.setInstanceDescriptorBuffer(instances.range);
			descriptor.setInstanceDescriptorStride(acceleration_structures::INSTANCE_DESCRIPTOR_SIZE);
			descriptor.setInstanceCount(instance_count);
		}
		descriptor.setInstanceDescriptorType(mtl::MTLAccelerationStructureInstanceDescriptorType::Indirect);

		// Instance records name their bottom-level structures by GPU resource handle, which the descriptor does not
		// enumerate, so every structure this context owns stays resident for the build.
		for acceleration_structure in self.device.acceleration_structures {
			self.command_buffer
				.retain_allocation(acceleration_structure.structure.clone());
		}

		// The instance records also make every bottom-level structure a read input of this build, so the build
		// waits on the bottom-level builds recorded before it.
		let bottom_level_reads = (0..self.device.acceleration_structures.len())
			.filter(|index| *index != structure_index)
			.map(|index| {
				synchronization::MetalResourceUse::acceleration_structure(
					index,
					mtl::MTLStages::AccelerationStructure,
					crate::AccessPolicies::READ,
				)
			})
			.collect::<SmallVec<[_; 8]>>();

		self.encode_acceleration_structure_build(
			structure_index,
			&descriptor,
			&acceleration_structure_build.scratch_buffer,
			std::iter::once(instances.use_).chain(bottom_level_reads),
		);
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		for build in acceleration_structure_builds {
			let structure_index = build.acceleration_structure.0 as usize;
			let (geometry_descriptor, uses) = match &build.description {
				crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
					vertex_buffer,
					vertex_count: _,
					vertex_position_encoding,
					index_buffer,
					triangle_count,
					index_format,
				} => {
					let vertices =
						self.resolve_acceleration_structure_strided_range(vertex_buffer, crate::AccessPolicies::READ);
					let indices = self.resolve_acceleration_structure_strided_range(index_buffer, crate::AccessPolicies::READ);
					let descriptor = mtl::MTL4AccelerationStructureTriangleGeometryDescriptor::new();

					descriptor.setVertexFormat(acceleration_structures::to_vertex_format(*vertex_position_encoding));
					descriptor.setIndexType(acceleration_structures::to_index_type(*index_format));
					// SAFETY: Both ranges were bounds-checked against the buffers that back them, and the counts they
					// describe come from the caller's geometry description.
					unsafe {
						descriptor.setVertexBuffer(vertices.range);
						descriptor.setVertexStride(vertex_buffer.stride);
						descriptor.setIndexBuffer(indices.range);
						descriptor.setTriangleCount(*triangle_count as usize);
					}

					(
						Retained::into_super(descriptor),
						SmallVec::<[_; 2]>::from_slice(&[vertices.use_, indices.use_]),
					)
				}
				crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB {
					aabb_buffer,
					transform_count,
					..
				} => {
					let bounding_box_count = *transform_count as usize;
					let bounding_boxes = self.resolve_acceleration_structure_buffer(
						*aabb_buffer,
						0,
						bounding_box_count * std::mem::size_of::<mtl::MTLAxisAlignedBoundingBox>(),
						crate::AccessPolicies::READ,
					);
					let descriptor = mtl::MTL4AccelerationStructureBoundingBoxGeometryDescriptor::new();

					// SAFETY: The bounding-box range was bounds-checked against the buffer that backs it.
					unsafe {
						descriptor.setBoundingBoxBuffer(bounding_boxes.range);
						descriptor.setBoundingBoxStride(std::mem::size_of::<mtl::MTLAxisAlignedBoundingBox>());
						descriptor.setBoundingBoxCount(bounding_box_count);
					}

					(
						Retained::into_super(descriptor),
						SmallVec::<[_; 2]>::from_slice(&[bounding_boxes.use_]),
					)
				}
			};

			let descriptor = mtl::MTL4PrimitiveAccelerationStructureDescriptor::new();
			let geometry_descriptors = NSArray::from_retained_slice(&[geometry_descriptor]);
			descriptor.setGeometryDescriptors(Some(&geometry_descriptors));

			self.encode_acceleration_structure_build(structure_index, &descriptor, &build.scratch_buffer, uses);
		}
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
		let attachment_image = |handle: ImageHandle| {
			let image = self.device.images.resource(handle);
			let description = image.description;
			(
				Some(handle),
				image.texture.clone(),
				description.format,
				description.array_layers,
			)
		};
		let attachments = attachments
			.iter()
			.map(|attachment| {
				// `image` is `None` only for a drawable, which hazard tracking identifies by its texture.
				let (image, texture, format, array_layers) = match attachment.target {
					ImageOrSwapchain::Image(image) => attachment_image(self.get_internal_image_handle(image)),
					ImageOrSwapchain::Swapchain(swapchain) => {
						let swapchain = crate::swapchain::SwapchainHandle(swapchain.0);
						match self.swapchain_proxy(swapchain) {
							// Presentation copies the proxy to the drawable, so the pass must render into the proxy.
							Some(proxy) => attachment_image(proxy),
							// TODO: get the drawable's actual format.
							None => (None, self.drawable_texture(swapchain), crate::Formats::BGRAu8, 1),
						}
					}
				};
				validate_attachment_layer_selection(attachment.layer, attachment.layer_count, array_layers);
				// A layer of an array image is rendered through a 2D view of that layer.
				let view = attachment
					.layer
					.filter(|_| array_layers > 1)
					.map(|layer| texture_view_2d(&texture, format, 0, layer));
				(attachment, image, texture, view, format)
			})
			.collect::<SmallVec<[_; 8]>>();

		let rpd = mtl::MTL4RenderPassDescriptor::new();
		if layered {
			rpd.setRenderTargetArrayLength(render_target_array_length as _);
		}

		let mut color_index = 0;
		for (attachment, _, texture, view, format) in &attachments {
			let target = view.as_ref().unwrap_or(texture);
			self.command_buffer.retain_allocation(texture.clone());
			self.command_buffer.retain_allocation(target.clone());
			let descriptor: Retained<mtl::MTLRenderPassAttachmentDescriptor> = if format.is_depth() {
				let depth = rpd.depthAttachment();
				depth.setClearDepth(utils::clear_depth(attachment.clear));
				Retained::into_super(depth)
			} else {
				// SAFETY: `color_index` counts only color attachments and stays within Metal's attachment array.
				let color = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				color_index += 1;
				color.setClearColor(utils::clear_color(attachment.clear));
				Retained::into_super(color)
			};
			descriptor.setTexture(Some(target));
			descriptor.setLoadAction(utils::load_action(attachment.load));
			descriptor.setStoreAction(utils::store_action(attachment.store));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
			"Metal 4 render command encoder creation failed. The most likely cause is that the command buffer could not start the render pass.",
		);
		#[cfg(debug_assertions)]
		{
			self.render_debug_region_depth =
				self.begin_encoder_debug_regions(&*rce, "Render", attachments.iter().map(|(_, image, ..)| *image));
		}

		let scope = self.allocate_encoder_scope();
		self.active_encoder_scope = Some(scope);
		self.active_render_encoder = Some(rce);
		let mut initial_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		let mut final_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		for (attachment, image, texture, ..) in &attachments {
			let resource_use = |access| match *image {
				Some(image) => {
					synchronization::MetalResourceUse::image(image, Some(0), attachment.layer, mtl::MTLStages::Fragment, access)
				}
				None => synchronization::MetalResourceUse::drawable(texture.as_ref(), mtl::MTLStages::Fragment, access),
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
		self.consume_resources(initial_attachment_uses);
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

		self.active_render_extent = extent;
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
			self.command_buffer.retain_allocation(image.texture.clone());
			let is_depth = image.description.format.is_depth();
			let compatible = batch.is_empty()
				|| (batch_extent == Some(image.description.extent)
					&& batch_array_layers == image.description.array_layers
					&& !batch.iter().any(|(resident_handle, _)| *resident_handle == image_handle)
					&& if is_depth { !has_depth } else { color_count < 8 });

			if !compatible {
				self.encode_image_clear_batch(&batch);
				batch.clear();
				color_count = 0;
				has_depth = false;
			}

			if batch.is_empty() {
				batch_extent = Some(image.description.extent);
				batch_array_layers = image.description.array_layers;
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

		let transfer_encoder = self.ensure_compute_encoder().clone();
		for buffer_handle in buffer_handles {
			let handle = self.get_internal_buffer_handle(*buffer_handle);
			let (buffer, size) = {
				let buffer = self.device.buffers.resource(handle);
				(buffer.buffer.clone(), buffer.size)
			};
			if size == 0 {
				continue;
			}
			self.command_buffer.retain_allocation(buffer.clone());
			self.consume_resources([synchronization::MetalResourceUse::buffer(
				handle,
				0,
				size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			)]);
			// SAFETY: The whole destination buffer range is valid and tracked for an exclusive transfer write.
			unsafe {
				transfer_encoder.fillBuffer_range_value(buffer.as_ref(), NSRange::new(0, size), 0);
			}
		}
	}

	fn copy_buffers(&mut self, copies: &[crate::BufferCopyDescriptor]) {
		if !copies.iter().any(|copy| copy.size > 0) {
			return;
		}

		let transfer_encoder = self.ensure_compute_encoder().clone();
		for copy in copies {
			if copy.size == 0 {
				continue;
			}
			let source_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_handle = self.get_internal_buffer_handle(copy.destination_buffer);
			let source = self.device.buffers.resource(source_handle).buffer.clone();
			let destination = self.device.buffers.resource(destination_handle).buffer.clone();

			self.command_buffer.retain_allocation(source.clone());
			self.command_buffer.retain_allocation(destination.clone());
			self.consume_resources([
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
			// SAFETY: Source and destination ranges were bounds-checked above and reference distinct transfer regions.
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

		let transfer_encoder = self.ensure_compute_encoder().clone();
		for copy in copies {
			let source_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_handle = self.get_internal_image_handle(copy.destination_image);
			let source_size = copy
				.source_bytes_per_image
				.checked_mul(self.device.images.resource(destination_handle).description.array_layers as usize)
				.expect(
					"Metal texture copy tracked range overflowed. The most likely cause is an invalid source pitch or array layer count.",
				);
			self.consume_resources([
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
			let source = self.device.buffers.resource(source_handle);
			let destination = self.device.images.resource(destination_handle);
			self.command_buffer.retain_allocation(source.buffer.clone());
			self.command_buffer.retain_allocation(destination.texture.clone());

			assert!(
				copy.destination_mip_level < destination.description.mip_levels,
				"Metal texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image. mip_level={}, mip_levels={}",
				copy.destination_mip_level,
				destination.description.mip_levels
			);
			let destination_extent = crate::image::mip_extent(destination.description.extent, copy.destination_mip_level);
			let (compact_bytes_per_row, row_count, compact_bytes_per_image) =
				utils::texture_upload_layout(destination.description.format, destination_extent);
			let expected_bytes_per_row = compact_bytes_per_row.next_multiple_of(256);
			let expected_bytes_per_image = expected_bytes_per_row * row_count;

			assert_eq!(
				copy.source_offset % 256,
				0,
				"Metal texture copy source offset alignment mismatch. The most likely cause is that the staging allocator did not provide a 256-byte aligned texture upload offset. source_offset={}, source_bytes_per_row={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				copy.source_offset,
				copy.source_bytes_per_row,
				copy.source_bytes_per_image,
				destination.description.format,
				destination.description.extent
			);
			assert_eq!(
				copy.source_bytes_per_row, expected_bytes_per_row,
				"Metal texture copy row pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about BC block row padding. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_row={}, expected={expected_bytes_per_row}",
				destination.description.format, destination.description.extent, copy.source_bytes_per_row
			);
			assert_eq!(
				copy.source_bytes_per_image, expected_bytes_per_image,
				"Metal texture copy image pitch mismatch. The most likely cause is that upload preparation and Metal copy recording disagree about padded rows per image. format={:?}, extent={:?}, compact_bytes_per_row={compact_bytes_per_row}, compact_bytes_per_image={compact_bytes_per_image}, row_count={row_count}, source_bytes_per_image={}, expected={expected_bytes_per_image}",
				destination.description.format, destination.description.extent, copy.source_bytes_per_image
			);
			let required_source_bytes = copy
				.source_bytes_per_image
				.checked_mul(destination.description.array_layers as usize)
				.and_then(|copy_bytes| copy.source_offset.checked_add(copy_bytes))
				.expect(
					"Metal texture copy source bounds overflowed. The most likely cause is an invalid array layer count or image pitch.",
				);

			assert!(
				required_source_bytes <= source.size,
				"Metal texture copy source buffer is too small. The most likely cause is that the staging buffer allocation is smaller than the recorded texture copy. source_size={}, required_source_bytes={required_source_bytes}, source_offset={}, array_layers={}, source_bytes_per_image={}, format={:?}, extent={:?}",
				source.size,
				copy.source_offset,
				destination.description.array_layers,
				copy.source_bytes_per_image,
				destination.description.format,
				destination.description.extent
			);

			let mut source_size = utils::mtl_size(destination_extent);
			source_size.depth = 1;
			let destination_origin = mtl::MTLOrigin { x: 0, y: 0, z: 0 };

			for slice in 0..destination.description.array_layers as usize {
				let source_offset = copy.source_offset + slice * copy.source_bytes_per_image;

				// SAFETY: The source layout and destination subresource were validated before recording this copy.
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
		self.record_texture_transfer(source, 0)
	}

	fn transfer_texture_with_frame(
		&mut self,
		image: graphics_hardware_interface::DynamicImageHandle,
		frame_offset: i32,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		self.record_texture_transfer(image.into(), frame_offset)
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let image_handle = self.get_internal_image_handle(image_handle);

		let (texture, format, extent, array_layers) = {
			let image = self.device.images.resource(image_handle);
			(
				image.texture.clone(),
				image.description.format,
				image.description.extent,
				image.description.array_layers,
			)
		};

		// The upload buffer snapshots caller memory now; the tracked blit performs the GPU-visible write in command order.
		// SAFETY: `data` is a live initialized slice and the byte view preserves its exact extent.
		let bytes = unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data)) };
		self.command_buffer.retain_allocation(texture.clone());
		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([synchronization::MetalResourceUse::image(
			image_handle,
			Some(0),
			None,
			mtl::MTLStages::Blit,
			crate::AccessPolicies::WRITE,
		)]);
		let upload_buffer = encode_texture_upload(
			self.device.metal_device,
			self.commit.upload_arena,
			transfer_encoder.as_ref(),
			texture.as_ref(),
			format,
			extent,
			array_layers,
			bytes,
			None,
		);
		self.command_buffer.retain_allocation(upload_buffer);
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
		self.command_buffer.retain_allocation(source_texture.clone());
		self.command_buffer.retain_allocation(destination_texture.clone());
		let transfer_encoder = self.ensure_compute_encoder().clone();
		self.consume_resources([
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

		// SAFETY: Both textures are retained, format-compatible, and tracked for opposing copy accesses.
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
		self.bind_pipeline(pipeline_handle)
	}

	fn bind_ray_tracing_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRayTracingPipelineMode {
		self.bind_pipeline(pipeline_handle)
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
}

impl RasterizationRenderPassMode for CommandBufferRecording<'_> {
	fn bind_raster_pipeline(
		&mut self,
		pipeline_handle: graphics_hardware_interface::PipelineHandle,
	) -> &mut impl BoundRasterizationPipelineMode {
		self.bind_pipeline(pipeline_handle)
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

	fn set_scissor(&mut self, origin: [u32; 2], extent: Extent) {
		let rce = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that set_scissor was called outside start_render_pass.");
		// Metal rejects scissors outside the render target, so clamp to the pass extent.
		let (origin, extent) = crate::clamp_scissor(origin, extent, self.active_render_extent);
		rce.setScissorRect(mtl::MTLScissorRect {
			x: origin[0] as _,
			y: origin[1] as _,
			width: extent.width() as _,
			height: extent.height() as _,
		});
	}

	fn end_render_pass(&mut self) {
		self.end_render_encoder();
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		self.bound_pipeline.expect(
			"No pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		// Binding replaces the complete flat set union; native argument-buffer work is deferred until execution.
		self.update_bound_descriptor_sets(sets);
		self
	}

	fn write_push_constant<T: crate::Pod>(&mut self, offset: u32, data: T) {
		self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that write_push_constant was called before binding a pipeline.",
		);
		let end = offset as usize + std::mem::size_of::<T>();

		// Binding a pipeline sizes the push-constant storage to its layout.
		assert!(
			end <= self.push_constant_data.len(),
			"Push constant write exceeds the Metal pipeline layout push constant storage. The most likely cause is that the write offset or type size does not match the pipeline's declared push constant ranges.",
		);

		self.push_constant_data[offset as usize..end].copy_from_slice(bytemuck::bytes_of(&data));

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
				self.command_buffer.retain_allocation(vertex_buffer);
			}
			self.set_stage_buffer_address(ArgumentTableStage::Vertex, binding as u32, address);
		}

		let mesh = &self.device.meshes[mesh_index];
		let index_buffer = mesh.index_buffer.clone();
		let index_count = mesh.index_count;
		let index_buffer_address = index_buffer.gpuAddress();
		let index_buffer_length = index_buffer.length();
		self.command_buffer.retain_allocation(index_buffer);
		let encoder = self
			.active_render_encoder
			.as_ref()
			.expect("No active render pass. The most likely cause is that draw_mesh was called outside start_render_pass.");

		// SAFETY: Mesh metadata provides a live retained index buffer and a bounds-checked index range.
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
		// SAFETY: An active render encoder exists and the validated vertex range belongs to the bound pipeline.
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
		self.command_buffer.retain_allocation(native_buffer);

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

		// SAFETY: An active render encoder exists and the index-buffer address and draw ranges were validated above.
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
				utils::mtl_size(object_threadgroup_size),
				utils::mtl_size(mesh_threadgroup_size),
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
			utils::mtl_size(threads_per_threadgroup),
		);
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: impl Into<crate::command_buffer::IndirectDispatchBuffer<N>>,
		entry_index: usize,
	) {
		assert!(
			entry_index < N,
			"Metal indirect dispatch entry is out of bounds. The most likely cause is that entry_index exceeds the typed indirect buffer length. entry_index={entry_index}, entry_count={N}",
		);
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle.into().handle());
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
		self.command_buffer.retain_allocation(buffer.buffer.clone());

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

		// SAFETY: The indirect buffer address is retained, aligned, and valid for one Metal dispatch argument record.
		unsafe {
			self.ensure_compute_encoder()
				.dispatchThreadgroupsWithIndirectBuffer_threadsPerThreadgroup(
					indirect_buffer_address,
					utils::mtl_size(threadgroup_extent),
				);
		}
	}
}

impl BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, _binding_tables: crate::rt::BindingTables, x: u32, y: u32, z: u32) {
		// Metal resolves hit and miss behaviour inside the ray-generation function through the bound acceleration
		// structure, so the binding tables other backends index carry no work here and one ray is one thread.
		let bound_pipeline = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that trace_rays was called before bind_ray_tracing_pipeline.");
		let threadgroup_extent = self.device.pipelines[bound_pipeline.0 as usize]
			.compute_threadgroup_size
			.unwrap_or(Extent::square(8));

		self.prepare_compute_dispatch([]);
		self.flush_compute_push_constants();

		self.ensure_compute_encoder().dispatchThreadgroups_threadsPerThreadgroup(
			mtl::MTLSize {
				width: x.div_ceil(threadgroup_extent.width().max(1)) as _,
				height: y.div_ceil(threadgroup_extent.height().max(1)) as _,
				depth: z.div_ceil(threadgroup_extent.depth().max(1)) as _,
			},
			utils::mtl_size(threadgroup_extent),
		);
	}
}
