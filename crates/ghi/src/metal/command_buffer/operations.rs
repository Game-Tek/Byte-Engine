use objc2_metal::{MTLComputePipelineState as _, MTLVisibleFunctionTable as _};

use super::*;
use crate::metal::acceleration_structures::{self as structures, BottomLevel, BuildState, INSTANCE_DESCRIPTOR_SIZE};

impl CommandBufferRecording<'_> {
	/// Records one copy of a transfer source, or another frame's copy of a per-frame image, into shared staging.
	fn record_texture_transfer(
		&mut self,
		source: graphics_hardware_interface::ImageOrSwapchain,
		frame_offset: i32,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		if let ImageOrSwapchain::Swapchain(swapchain) = source
			&& self
				.device
				.swapchains
				.get(swapchain.0 as usize)
				.is_some_and(|swapchain| !swapchain.uses.contains(crate::Uses::TransferSource))
		{
			return Err(crate::TextureTransferError::MissingTransferSource);
		}
		let surface = self
			.surface(source, frame_offset)
			.ok_or(crate::TextureTransferError::InvalidSource)?;
		let source_use = synchronization::MetalResourceUse::image(
			surface.image,
			Some(0),
			None,
			mtl::MTLStages::Blit,
			crate::AccessPolicies::READ,
		);
		let Surface {
			texture: source_texture,
			format,
			extent,
			array_layers,
			uses,
			..
		} = surface;
		let layout = crate::context::texture_transfer_layout(format, extent, array_layers, uses)?;
		let (native_bytes_per_row, native_bytes_per_image) =
			crate::aligned_copy_pitches(layout.bytes_per_row, layout.row_count)
				.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let size = native_bytes_per_image
			.checked_mul(layout.depth_slices)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let compact_size = layout
			.bytes_per_image
			.checked_mul(layout.depth_slices)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		// Mapping fills every reserved byte, so the vector is only reserved here.
		let mut bytes = Vec::new();
		bytes
			.try_reserve_exact(compact_size)
			.map_err(|_| crate::TextureTransferError::AllocationFailed)?;
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
		// Hazard tracking never sees the staging buffer, so the command retains it here.
		self.command_buffer.retain_allocation(&*staging);
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
			layout,
			native_bytes_per_row,
		});
		self.texture_readbacks.push(handle);
		Ok(handle)
	}

	/// Resolves `size` bytes of one acceleration-structure build input to a Metal address range and its tracked access.
	///
	/// Metal 4 reads build inputs by GPU address, so consuming the returned use is what makes the buffer resident.
	fn resolve_acceleration_structure_buffer(
		&self,
		buffer: &crate::BufferDescriptor,
		size: usize,
		access: crate::AccessPolicies,
	) -> (mtl::MTL4BufferRange, synchronization::MetalResourceUse) {
		let offset = buffer.offset;
		let handle = self.get_internal_buffer_handle(buffer.buffer);
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

		(
			mtl::MTL4BufferRange {
				bufferAddress: address,
				length: size as u64,
			},
			synchronization::MetalResourceUse::buffer(handle, offset, size, mtl::MTLStages::AccelerationStructure, access),
		)
	}

	/// Resolves one indirect argument record to its GPU address and the read `stages` make of it.
	fn resolve_indirect_record(
		&self,
		buffer_handle: graphics_hardware_interface::BaseBufferHandle,
		entry: std::ops::Range<usize>,
		stages: mtl::MTLStages,
	) -> (mtl::MTLGPUAddress, synchronization::MetalResourceUse) {
		let handle = self.get_internal_buffer_handle(buffer_handle);
		let buffer = self.device.buffers.resource(handle);
		assert!(
			entry.end <= buffer.size,
			"Metal indirect entry exceeds the buffer. The most likely cause is that the typed buffer metadata does not match its native allocation. entry_end={}, buffer_size={}",
			entry.end,
			buffer.size,
		);
		let address = buffer.gpu_address.checked_add(entry.start as u64).expect(
			"Metal indirect GPU address overflowed. The most likely cause is that the selected entry exceeds the native address space.",
		);
		let record_use =
			synchronization::MetalResourceUse::buffer(handle, entry.start, entry.len(), stages, crate::AccessPolicies::READ);
		(address, record_use)
	}

	/// Encodes the copies that compact bottom-level structures into the storage that replaced theirs.
	///
	/// Each copy writes its structure's slot, so the top-level builds recorded after it on the queue wait for it.
	pub(in crate::metal) fn compact_acceleration_structures(&mut self, copies: &[structures::CompactionCopy]) {
		for copy in copies {
			let encoder = self.ensure_compute_encoder().clone();
			self.consume_resources([synchronization::MetalResourceUse::bottom_level(
				copy.slot,
				crate::AccessPolicies::WRITE,
			)]);
			self.command_buffer.retain_allocation(&*copy.source);
			self.command_buffer.retain_allocation(&*copy.destination);
			// SAFETY: Both structures are retained and pinned resident, they do not overlap, and the destination was
			// sized for the compacted size Metal reported once the source's build completed.
			unsafe {
				encoder.copyAndCompactAccelerationStructure_toAccelerationStructure(&copy.source, &copy.destination);
			}
			self.note_bottom_level_work();
		}
	}
}

impl CommandBufferRecordingTrait for CommandBufferRecording<'_> {
	fn frame_key(&self) -> graphics_hardware_interface::FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn unordered(&mut self, f: impl FnOnce(&mut Self)) {
		// Metal 4 orders nothing inside an encoder by itself, so leaving out the barriers is all the group needs.
		self.resource_tracker.begin_unordered();
		f(self);
		self.resource_tracker.end_unordered();
	}

	fn build_top_level_acceleration_structure(
		&mut self,
		structure: crate::TopLevelAccelerationStructureHandle,
		instances: &[crate::rt::Instance],
	) {
		let frame_key = self.frame_key.expect(
			"The top-level acceleration structure build was recorded outside a frame. The most likely cause is that the recording was created from a command buffer instead of a frame. Record top-level builds in a frame's recording, because each frame builds its own copy.",
		);
		// The structures outlive the recording, so holding them does not borrow the recording.
		let acceleration_structures = self.device.acceleration_structures;
		let top_level = acceleration_structures.top_level(structure);
		assert!(
			instances.len() <= top_level.max_instance_count as usize,
			"The top-level acceleration structure build has more instances than the structure holds. The most likely cause is that the scene grew past the maximum instance count given at creation. instances={}, max_instance_count={}. Create the structure with a larger maximum instance count.",
			instances.len(),
			top_level.max_instance_count,
		);
		let copy = frame_key.sequence_index;
		let target = &top_level
			.copies
			.get(copy as usize)
			.expect(
				"The top-level acceleration structure has no copy for this frame. The most likely cause is that the frames in flight grew after the structure was created. Create top-level structures after you set the frames in flight.",
			)
			.structure;

		// Metal reads the instance records while the build runs, so they live in the frame's upload pages, which stay
		// untouched until the frame completes.
		let (page, offset) = self
			.commit
			.upload_arena
			.allocate(self.device.metal_device, instances.len().max(1) * INSTANCE_DESCRIPTOR_SIZE);
		for (index, instance) in instances.iter().enumerate() {
			assert!(
				instance.custom_index < crate::rt::INSTANCE_INDEX_LIMIT
					&& instance.hit_group_offset < crate::rt::INSTANCE_INDEX_LIMIT,
				"An acceleration structure instance has a custom index or hit group offset of 2^24 or more. The most likely cause is that an index table outgrew 24 bits. custom_index={}, hit_group_offset={}. Keep both values below rt::INSTANCE_INDEX_LIMIT.",
				instance.custom_index,
				instance.hit_group_offset,
			);
			let bottom_level = acceleration_structures.bottom_level(instance.bottom_level);
			assert!(
				bottom_level.build_state.get() != BuildState::Unbuilt,
				"An acceleration structure instance places a bottom-level structure that was never built. The most likely cause is that its build was not recorded before this top-level build. Record its build with build_bottom_level_acceleration_structures in this recording or an earlier one.",
			);
			let record = structures::instance_record(instance, bottom_level.resource_id);
			// SAFETY: The allocation holds a record for every instance, and upload pages are CPU-visible shared memory.
			unsafe {
				page.contents
					.add(offset + index * INSTANCE_DESCRIPTOR_SIZE)
					.cast::<mtl::MTLIndirectAccelerationStructureInstanceDescriptor>()
					.write_unaligned(record);
			}
		}
		let descriptor = structures::instance_descriptor(
			mtl::MTL4BufferRange {
				bufferAddress: page.gpu_address + offset as u64,
				length: (instances.len() * INSTANCE_DESCRIPTOR_SIZE) as u64,
			},
			instances.len(),
		);
		self.command_buffer.retain_allocation(&*page.buffer);
		let (scratch, scratch_range) = self
			.commit
			.scratch_arena
			.allocate(self.device.metal_device, top_level.build_scratch_size);
		self.command_buffer.retain_allocation(&**scratch);

		let encoder = self.ensure_compute_encoder().clone();
		// Reading every bottom-level slot orders the build after the bottom-level builds and compactions before it.
		self.consume_resources([
			synchronization::MetalResourceUse::every_bottom_level(),
			synchronization::MetalResourceUse::top_level(
				structure,
				copy,
				mtl::MTLStages::AccelerationStructure,
				crate::AccessPolicies::WRITE,
			),
		]);
		// SAFETY: The copy was sized for the structure's maximum instance count, which bounds this build; the scratch
		// range covers the size Metal reported for it; the records name bottom-level structures that stay pinned
		// resident; and the copy and every buffer are retained.
		unsafe {
			encoder.buildAccelerationStructure_descriptor_scratchBuffer(target, &descriptor, scratch_range);
		}
		self.builds.top_levels.push((structure, copy, acceleration_structures.epoch));
	}

	fn build_bottom_level_acceleration_structures(&mut self, handles: &[crate::BottomLevelAccelerationStructureHandle]) {
		if handles.is_empty() {
			return;
		}
		// The structures outlive the recording, so holding them does not borrow the recording.
		let acceleration_structures = self.device.acceleration_structures;
		// Each build writes only its structure's slot, so the builds run without barriers between them.
		for &handle in handles {
			let bottom_level = acceleration_structures.bottom_level(handle);
			assert!(
				bottom_level.build_state.get() == BuildState::Unbuilt,
				"The bottom-level acceleration structure was already built. The most likely cause is that its build was recorded twice, or that a recording which has not executed yet holds its build. Build each bottom-level structure once, and create a new structure for different geometry.",
			);
			bottom_level.build_state.set(BuildState::Recorded);
			let mut uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
			let descriptor = structures::primitive_descriptor(&bottom_level.geometries, |buffer, size| {
				let (range, resource_use) =
					self.resolve_acceleration_structure_buffer(buffer, size, crate::AccessPolicies::READ);
				uses.push(resource_use);
				range
			});
			let (scratch, scratch_range) = self
				.commit
				.scratch_arena
				.allocate(self.device.metal_device, bottom_level.build_scratch_size);
			self.command_buffer.retain_allocation(&**scratch);
			self.command_buffer.retain_allocation(&*bottom_level.structure);

			let encoder = self.ensure_compute_encoder().clone();
			let slot = crate::slots::Slots::<BottomLevel>::index(handle.0);
			self.consume_resources(uses.into_iter().chain([synchronization::MetalResourceUse::bottom_level(
				slot,
				crate::AccessPolicies::WRITE,
			)]));
			// SAFETY: The structure was sized for this geometry at creation, the scratch range covers the size Metal
			// reported for it, and the structure and every buffer are retained and resident.
			unsafe {
				encoder.buildAccelerationStructure_descriptor_scratchBuffer(
					&bottom_level.structure,
					&descriptor,
					scratch_range,
				);
			}
			self.note_bottom_level_work();
		}

		// The compacted sizes read what the builds wrote, so they follow all of them after one barrier. They land in
		// the upload pages, which stay untouched until the recording completes and the context reads them.
		let size_bytes = size_of::<u64>();
		let (page, offset) = self
			.commit
			.upload_arena
			.allocate(self.device.metal_device, handles.len() * size_bytes);
		let size_buffer = page.buffer.clone();
		let size_address = page.gpu_address + offset as u64;
		self.command_buffer.retain_allocation(&*size_buffer);
		for (index, &handle) in handles.iter().enumerate() {
			let bottom_level = acceleration_structures.bottom_level(handle);
			let encoder = self.ensure_compute_encoder().clone();
			self.consume_resources([synchronization::MetalResourceUse::bottom_level(
				crate::slots::Slots::<BottomLevel>::index(handle.0),
				crate::AccessPolicies::READ,
			)]);
			let size_offset = offset + index * size_bytes;
			// SAFETY: The structure is retained and resident, and the range is eight bytes of a retained shared page.
			unsafe {
				encoder.writeCompactedAccelerationStructureSize_toBuffer(
					&bottom_level.structure,
					mtl::MTL4BufferRange {
						bufferAddress: size_address + (index * size_bytes) as u64,
						length: size_bytes as u64,
					},
				);
			}
			self.note_bottom_level_work();
			self.builds.bottom_levels.push(structures::RecordedBuild {
				handle,
				size_buffer: size_buffer.clone(),
				size_offset,
			});
		}
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl RasterizationRenderPassMode {
		self.end_encoder();

		let render_target_array_length =
			graphics_hardware_interface::AttachmentInformation::render_pass_layer_count(attachments);
		let layered = attachments.first().is_some_and(|attachment| attachment.layer_count.is_some());
		let attachments = attachments
			.iter()
			.map(|attachment| {
				// A swapchain resolves to its image for this frame, which presentation copies to the drawable.
				let surface = self.surface(attachment.target, 0).expect(MISSING_SURFACE);
				if let Some(layer) = attachment.layer {
					assert!(
						layer < surface.array_layers,
						"Render-pass attachment layer is out of bounds. The most likely cause is that the selected layer does not exist in the target image. layer={layer}, available_layers={}",
						surface.array_layers,
					);
				}
				let layer_count = attachment.layer_count.map_or(1, std::num::NonZeroU32::get);
				assert!(
					attachment.first_layer + layer_count <= surface.array_layers,
					"Render-pass attachment layer range is out of bounds. The most likely cause is that layered rendering requested layers the target image does not provide. first_layer={}, requested_layers={layer_count}, available_layers={}",
					attachment.first_layer,
					surface.array_layers,
				);
				// A layer of an array image is rendered through a 2D view of that layer, and a layer range that does not start
				// at the first layer through a 2D array view of that range.
				let view = match attachment.layer {
					Some(layer) if surface.array_layers > 1 => {
						Some(texture_view_2d(&surface.texture, surface.format, 0, layer))
					}
					None if attachment.first_layer > 0 => Some(texture_view_2d_array(
						&surface.texture,
						surface.format,
						attachment.first_layer,
						layer_count,
					)),
					_ => None,
				};
				(attachment, surface, view)
			})
			.collect::<SmallVec<[_; 8]>>();

		let rpd = mtl::MTL4RenderPassDescriptor::new();
		if layered {
			rpd.setRenderTargetArrayLength(render_target_array_length as _);
		}

		let mut color_index = 0;
		for (attachment, surface, view) in &attachments {
			let target = view.as_ref().unwrap_or(&surface.texture);
			// Attachment tracking retains the texture, but a layer view is a separate object nothing else retains.
			if let Some(view) = view {
				self.command_buffer.retain_allocation(&**view);
			}
			let descriptor: Retained<mtl::MTLRenderPassAttachmentDescriptor> = if surface.format.is_depth() {
				let depth = rpd.depthAttachment();
				depth.setClearDepth(utils::clear_depth(attachment.clear_value()));
				Retained::into_super(depth)
			} else {
				// SAFETY: `color_index` counts only color attachments and stays within Metal's attachment array.
				let color = unsafe { rpd.colorAttachments().objectAtIndexedSubscript(color_index) };
				color_index += 1;
				color.setClearColor(utils::clear_color(attachment.clear_value()));
				Retained::into_super(color)
			};
			descriptor.setTexture(Some(target));
			descriptor.setLoadAction(utils::load_action(attachment.load));
			descriptor.setStoreAction(utils::store_action(attachment.store));
		}

		let rce = self.command_buffer.renderCommandEncoderWithDescriptor(&rpd).expect(
			"Metal 4 render command encoder creation failed. The most likely cause is that the command buffer could not start the render pass.",
		);
		self.begin_encoder(
			ActiveEncoder::Render(rce),
			"Render",
			attachments.iter().map(|(_, surface, _)| surface.image),
		);

		let mut initial_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		let mut final_attachment_uses = SmallVec::<[synchronization::MetalResourceUse; 8]>::new();
		for (attachment, surface, _) in &attachments {
			let resource_use = |access| {
				synchronization::MetalResourceUse::image(
					surface.image,
					Some(0),
					attachment.layer,
					mtl::MTLStages::Fragment,
					access,
				)
			};
			let initial_access = crate::AccessPolicies::WRITE
				| if attachment.loads() {
					crate::AccessPolicies::READ
				} else {
					crate::AccessPolicies::NONE
				};
			initial_attachment_uses.push(resource_use(initial_access));
			final_attachment_uses.push(resource_use(crate::AccessPolicies::WRITE).in_group_memory(self.device.images));
			// A pass that clears or discards an attachment gives an image-group member new contents.
			if let (ImageOrSwapchain::Image(image), false) = (attachment.target, attachment.loads()) {
				self.commit.image_groups.initialize(image);
			}
		}
		self.consume_resources(initial_attachment_uses);
		// Every draw records the same attachment writes, so they are consolidated once for the whole pass.
		synchronization::MetalResourceTracker::consolidate_in_place(&mut final_attachment_uses);
		self.active_render_attachment_uses = final_attachment_uses;

		let rce = self.render_encoder("start_render_pass");
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

		self
	}

	fn discard_images(&mut self, images: &[graphics_hardware_interface::BaseImageHandle]) {
		// Metal has no image layouts, and hazard tracking already orders a member's next access after every access to
		// the heap bytes it shares, so discarding only records that the member holds new contents.
		for &image in images {
			self.commit.image_groups.initialize(image);
		}
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

		self.end_encoder();

		let mut batch = SmallVec::<[(ImageHandle, graphics_hardware_interface::ClearValue); 9]>::new();
		let mut color_count = 0;
		let mut has_depth = false;

		for (handle, clear_value) in textures {
			let image_handle = self.get_internal_image_handle(*handle);
			let image = self.device.images.resource(image_handle);
			self.commit.image_groups.initialize(*handle);
			let is_depth = image.description.format.is_depth();
			// A batch shares the extent and layer count of its first image.
			let compatible = batch.first().is_none_or(|&(first, _)| {
				let first = &self.device.images.resource(first).description;
				first.extent == image.description.extent
					&& first.array_layers == image.description.array_layers
					&& !batch.iter().any(|(resident_handle, _)| *resident_handle == image_handle)
					&& if is_depth { !has_depth } else { color_count < 8 }
			});

			if !compatible {
				self.encode_image_clear_batch(&batch);
				batch.clear();
				color_count = 0;
				has_depth = false;
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
			let buffer = self.device.buffers.resource(handle);
			if buffer.size == 0 {
				continue;
			}
			self.consume_resources([synchronization::MetalResourceUse::buffer(
				handle,
				0,
				buffer.size,
				mtl::MTLStages::Blit,
				crate::AccessPolicies::WRITE,
			)]);
			// SAFETY: The whole destination buffer range is valid and tracked for an exclusive transfer write.
			unsafe {
				transfer_encoder.fillBuffer_range_value(&buffer.buffer, NSRange::new(0, buffer.size), 0);
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
			let source = &self.device.buffers.resource(source_handle).buffer;
			let destination = &self.device.buffers.resource(destination_handle).buffer;

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
					source,
					copy.source_offset as _,
					destination,
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
			let source = self.device.buffers.resource(source_handle);
			let destination = self.device.images.resource(destination_handle);
			let source_size = copy
				.source_bytes_per_image
				.checked_mul(destination.description.array_layers as usize)
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

			assert!(
				copy.destination_mip_level < destination.description.mip_levels,
				"Metal texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image. mip_level={}, mip_levels={}",
				copy.destination_mip_level,
				destination.description.mip_levels
			);
			let destination_extent = crate::image::mip_extent(destination.description.extent, copy.destination_mip_level);
			let (compact_bytes_per_row, row_count, compact_bytes_per_image) =
				utils::texture_upload_layout(destination.description.format, destination_extent);
			let (expected_bytes_per_row, expected_bytes_per_image) =
				utils::texture_copy_pitches(compact_bytes_per_row, row_count);

			assert_eq!(
				copy.source_offset % crate::TEXTURE_COPY_PITCH_ALIGNMENT,
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
			let required_source_bytes = copy.source_offset.checked_add(source_size).expect(
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
						&source.buffer,
						source_offset as _,
						copy.source_bytes_per_row as _,
						copy.source_bytes_per_image as _,
						source_size,
						&destination.texture,
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
		// The upload buffer snapshots caller memory now; the tracked blit performs the GPU-visible write in command order.
		self.upload_texture(self.get_internal_image_handle(image_handle), bytemuck::cast_slice(data), None);
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

		let source_texture = &self.device.images.resource(source_internal).texture;
		let destination_texture = &self.device.images.resource(destination_internal).texture;
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
			transfer_encoder.copyFromTexture_toTexture(source_texture, destination_texture);
		}
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		self.sync_private_buffer(self.get_internal_buffer_handle(buffer_handle.into()));
	}

	/// Ends and submits a non-frame recording as a one-command Metal 4 batch that `synchronizer` signals.
	fn execute(mut self, synchronizer: graphics_hardware_interface::SynchronizerHandle) {
		self.end_encoder();
		self.publish_resource_states();
		let synchronizer = context::synchronizer_for_sequence(self.commit.synchronizers, synchronizer, self.sequence_index);
		for handle in self.texture_readbacks.drain(..) {
			self.commit.texture_readbacks.mark_submitted(handle, Some(synchronizer));
		}
		std::mem::take(&mut self.builds).submit(
			self.device.acceleration_structures,
			synchronizer,
			self.commit.pending_compactions,
		);

		let commands = SmallVec::<[queue::NativeCommand; 4]>::from_iter([self.command_buffer.take()]);
		let submitted = self.commit.queue.submit_batch(self.commit.queue_handle, commands);
		// The synchronizer owns the submitted batch until its completion message arrives.
		self.commit.synchronizers.resource_mut(synchronizer).signal(submitted);
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
		if self.device.debug_labels {
			let mut label = crate::command_buffer::DebugLabelWriter::new();
			_write_label(&mut label).expect("Invalid debug label. The label closure most likely failed while formatting.");
			let name = NSString::from_str(label.as_str());

			if let Some(state) = self.encoder.as_mut() {
				state.encoder.common().pushDebugGroup(&name);
				state.debug_region_depth += 1;
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

			if let Some(state) = self.encoder.as_mut() {
				state.encoder.common().popDebugGroup();
				state.debug_region_depth -= 1;
			}
		}
	}

	fn start_counter(&mut self, counter: crate::CounterHandle) {
		let slot = self
			.commit
			.counters
			.start(crate::counters::frame_sequence(self.frame_key), counter);
		self.write_timestamp(slot);
	}

	fn end_counter(&mut self, counter: crate::CounterHandle) {
		let slot = self
			.commit
			.counters
			.end(crate::counters::frame_sequence(self.frame_key), counter);
		self.write_timestamp(slot);
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
		let rce = self.render_encoder("set_scissor");
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
		self.end_encoder();
	}
}

impl BoundPipelineLayoutMode for CommandBufferRecording<'_> {
	fn bind_descriptor_sets(&mut self, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut Self {
		self.bound_pipeline.expect(
			"No pipeline is bound. The most likely cause is that bind_descriptor_sets was called before binding a pipeline.",
		);
		// Binding replaces the complete flat set union; native argument-buffer work is deferred until execution.
		if self.bound_descriptor_set_roots.as_slice() != sets {
			self.bound_descriptor_set_roots.clear();
			self.bound_descriptor_set_roots.extend_from_slice(sets);
			self.bound_descriptor_sets.clear();
			for &descriptor_set_handle in sets {
				let resolved = self
					.commit
					.descriptor_sets
					.nth_handle(descriptor_set_handle, self.sequence_index as usize)
					.expect(
						"Missing frame-local Metal descriptor set. The most likely cause is that the set handle came from another context.",
					);
				// The version is read before each command, since writes can follow the bind.
				self.bound_descriptor_sets.push((resolved, 0));
			}
		}
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

		if let Some(state) = self.encoder.as_mut() {
			state.push_constants_dirty = true;
		}
	}
}

impl BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		self.note_command();
		self.prepare_draw("draw_mesh", []);
		let mesh = &self.device.meshes[mesh_handle.0 as usize];

		assert!(
			mesh.vertex_buffers.len() <= PUSH_CONSTANT_BINDING_INDEX as usize,
			"Too many Metal mesh vertex buffers were bound. The most likely cause is that mesh bindings overlap the reserved push-constant or argument-buffer slots."
		);

		// Metal 4 snapshots mesh vertex addresses through the shared stage argument table ABI. Hazard tracking never
		// sees mesh buffers, so the command retains them here.
		// The binding count is checked against the push-constant slot above, so the addresses never spill.
		let mut addresses = SmallVec::<[mtl::MTLGPUAddress; PUSH_CONSTANT_BINDING_INDEX as usize]>::new();
		for vertex_buffer in &mesh.vertex_buffers {
			addresses.push(vertex_buffer.as_ref().map_or(0, |vertex_buffer| {
				self.command_buffer.retain_allocation(&**vertex_buffer);
				vertex_buffer.gpuAddress()
			}));
		}
		self.command_buffer.retain_allocation(&*mesh.index_buffer);
		self.encode_vertex_addresses(&addresses);

		// SAFETY: Mesh metadata provides a live retained index buffer and a bounds-checked index range.
		unsafe {
			self.render_encoder("draw_mesh")
				.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferLength(
					mtl::MTLPrimitiveType::Triangle,
					mesh.index_count as _,
					mtl::MTLIndexType::UInt16,
					mesh.index_buffer.gpuAddress(),
					mesh.index_buffer.length(),
				);
		}
		// Mesh-owned bindings replace the ordinary logical bindings even when that logical list is empty.
		self.render_vertex_buffers_dirty = true;
		self.record_render_attachment_writes();
	}

	fn draw(&mut self, vertex_count: u32, instance_count: u32, first_vertex: u32, first_instance: u32) {
		self.note_command();
		self.use_bound_vertex_buffers();
		self.prepare_draw("draw", []);
		self.apply_bound_vertex_buffers();
		// SAFETY: An active render encoder exists and the validated vertex range belongs to the bound pipeline.
		unsafe {
			self.render_encoder("draw")
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
		self.note_command();
		let (buffer_handle, offset, index_type) = self
			.bound_index_buffer
			.expect("No index buffer bound. The most likely cause is that draw_indexed was called before bind_index_buffer.");
		let internal_buffer = self.get_internal_buffer_handle(buffer_handle);
		let buffer = self.device.buffers.resource(internal_buffer);
		let (metal_index_type, index_size) = (utils::to_index_type(index_type), index_type.size());
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
			index_data_end <= buffer.size,
			"Metal indexed draw exceeds the index buffer. The most likely cause is that the bound offset, first index, or index count exceeds the buffer size. range_end={index_data_end}, buffer_size={}",
			buffer.size,
		);
		// Metal 4 measures the accessible index range from the shifted GPU address, not from the buffer allocation's start.
		let index_buffer_length = buffer.size - index_buffer_offset;
		let index_buffer_address = buffer.gpu_address.checked_add(index_buffer_offset as u64).expect(
			"Metal index-buffer GPU address overflowed. The most likely cause is that the bound index range exceeds the native address space.",
		);
		self.use_bound_vertex_buffers();
		if index_data_size > 0 {
			self.command_uses.push(synchronization::MetalResourceUse::buffer(
				internal_buffer,
				index_buffer_offset,
				index_data_size,
				mtl::MTLStages::Vertex,
				crate::AccessPolicies::READ,
			));
		} else {
			// An empty index range is not tracked, but the draw still names the buffer, so the command retains it.
			self.command_buffer.retain_allocation(&*buffer.buffer);
		}
		self.prepare_draw("draw_indexed", []);
		self.apply_bound_vertex_buffers();

		// SAFETY: An active render encoder exists and the index-buffer address and draw ranges were validated above.
		unsafe {
			self.render_encoder("draw_indexed")
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
		self.note_command();
		self.prepare_draw("dispatch_meshes", []);
		let bound_pipeline = self
			.bound_pipeline
			.expect("No pipeline bound. The most likely cause is that dispatch_meshes was called before bind_raster_pipeline.");
		let raster = self.device.pipelines[bound_pipeline.0 as usize].raster();
		let mesh_threadgroup_size = raster.mesh_threadgroup_size.expect(
			"Metal mesh dispatch requires mesh threadgroup metadata. The most likely cause is that the mesh shader was not generated with Metal mesh threadgroup size metadata.",
		);
		let object_threadgroup_size = raster.object_threadgroup_size.unwrap_or(Extent::new(1, 1, 1));

		self.render_encoder("dispatch_meshes")
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

	fn draw_indirect<const N: usize>(
		&mut self,
		buffer_handle: graphics_hardware_interface::BufferHandle<[[u32; 4]; N]>,
		entry_index: usize,
	) {
		self.note_command();
		let entry = crate::command_buffer::indirect_entry_range::<[u32; 4], N>(entry_index);
		// The vertex stage consumes the draw record, so it must wait for whichever GPU work wrote the counts.
		let (indirect_buffer_address, record_use) =
			self.resolve_indirect_record(buffer_handle.into(), entry, mtl::MTLStages::Vertex);
		self.use_bound_vertex_buffers();
		self.prepare_draw("draw_indirect", [record_use]);
		self.apply_bound_vertex_buffers();

		self.render_encoder("draw_indirect")
			.drawPrimitives_indirectBuffer(mtl::MTLPrimitiveType::Triangle, indirect_buffer_address);
		self.record_render_attachment_writes();
	}
}

impl BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		self.prepare_dispatch([]);
		// `get_extent` rounds every dimension up to at least one threadgroup, so `mtl_size` keeps it unchanged.
		self.ensure_compute_encoder().dispatchThreadgroups_threadsPerThreadgroup(
			utils::mtl_size(dispatch.get_extent()),
			utils::mtl_size(dispatch.get_workgroup_extent()),
		);
	}

	fn indirect_dispatch<const N: usize>(
		&mut self,
		buffer_handle: impl Into<crate::command_buffer::IndirectDispatchBuffer<N>>,
		entry_index: usize,
	) {
		let entry = crate::command_buffer::indirect_entry_range::<[u32; 3], N>(entry_index);
		let (indirect_buffer_address, record_use) =
			self.resolve_indirect_record(buffer_handle.into().0, entry, mtl::MTLStages::Dispatch);
		self.prepare_dispatch([record_use]);

		let bound_pipeline = self.bound_pipeline.expect(
			"No pipeline bound. The most likely cause is that indirect_dispatch was called before bind_compute_pipeline.",
		);
		let threadgroup_extent = self.device.pipelines[bound_pipeline.0 as usize]
			.compute_threadgroup_size()
			.unwrap_or(Extent::line(128));

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
	/// Binds the visible function table that holds the table's records and dispatches one thread of the ray
	/// generation kernel for each launch point.
	fn trace_rays(&mut self, table: graphics_hardware_interface::ShaderBindingTableHandle, extent: Extent) {
		let bound_pipeline = self.bound_pipeline.expect(
			"Metal trace_rays has no pipeline bound. The most likely cause is that trace_rays was called before bind_ray_tracing_pipeline. Bind the ray tracing pipeline first.",
		);
		// The tables outlive the recording, so the records do not borrow it.
		let tables = self.device.shader_binding_tables;
		let shader_binding_table = tables.get(table.0 as usize).expect(
			"Metal trace_rays failed because the shader binding table handle names no table of this context. The most likely cause is that the handle came from another context. Trace with tables the recording's context created.",
		);
		let Some((written_for, functions)) = &shader_binding_table.written else {
			panic!(
				"Metal trace_rays failed because shader binding table {:?} has never been written. The most likely cause is that the table was traced before write_shader_binding_table gave it records. Write the table for the bound pipeline before tracing with it.",
				shader_binding_table.name,
			);
		};
		assert!(
			*written_for == bound_pipeline,
			"Metal trace_rays failed because shader binding table {:?} was last written for pipeline {written_for:?}, and pipeline {bound_pipeline:?} is bound. The most likely cause is that the pipeline was created again, with more hit groups, and the table wasn't written for the new one. Write the table for the bound pipeline before tracing with it.",
			shader_binding_table.name,
		);
		let PipelineState::RayTracing { state, .. } = &self.device.pipelines[bound_pipeline.0 as usize].pipeline else {
			unreachable!(
				"Metal trace_rays found a table written for a pipeline that isn't a ray tracing pipeline. The most likely cause is that table writes stopped checking the pipeline kind. Restore the check in build_shader_binding_table."
			);
		};

		// A height or depth of 0 counts as 1, as one- and two-dimensional extents leave them, and a width of 0 traces
		// nothing.
		if extent.width() == 0 {
			return;
		}
		let grid = mtl::MTLSize {
			width: extent.width() as usize,
			height: extent.height().max(1) as usize,
			depth: extent.depth().max(1) as usize,
		};
		self.prepare_dispatch([]);
		self.command_buffer.retain_allocation(&**functions);
		self.set_stage_resource(
			ArgumentTableStage::Compute,
			SHADER_BINDING_TABLE_BINDING_INDEX,
			functions.gpuResourceID(),
		);

		// Rays share no workgroup memory, so the threadgroup only decides which launch points run side by side: a row
		// of a one-dimensional launch, which keeps rays the application sorted together, or a square tile otherwise.
		let max_threads = state.maxTotalThreadsPerThreadgroup();
		let (width, height) = if grid.height == 1 && grid.depth == 1 {
			(64, 1)
		} else {
			(8, 8)
		};
		let width = width.min(max_threads);
		let threadgroup = mtl::MTLSize {
			width,
			height: height.min(max_threads / width),
			depth: 1,
		};
		// Metal ends the threadgroups at the grid's edges early, so exactly one thread runs for each launch point.
		self.ensure_compute_encoder()
			.dispatchThreads_threadsPerThreadgroup(grid, threadgroup);
	}
}
