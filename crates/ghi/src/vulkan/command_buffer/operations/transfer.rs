use super::*;

fn transfer_consumption(handle: Handles, access: crate::AccessPolicies) -> Consumption {
	Consumption {
		handle,
		stages: crate::Stages::TRANSFER,
		access,
		layout: crate::Layouts::Transfer,
	}
}

fn subresource_layers(aspect_mask: vk::ImageAspectFlags, mip_level: u32, layer_count: u32) -> vk::ImageSubresourceLayers {
	vk::ImageSubresourceLayers::default()
		.aspect_mask(aspect_mask)
		.mip_level(mip_level)
		.layer_count(layer_count)
}

/// Geometry and instance data read by an acceleration-structure build, per the vkCmdBuildAccelerationStructuresKHR rules.
fn acceleration_structure_input(handle: Handles) -> VulkanConsumption {
	acceleration_structure_build_access(handle, vk::AccessFlags2::SHADER_READ)
}

fn acceleration_structure_scratch(handle: Handles) -> VulkanConsumption {
	acceleration_structure_build_access(
		handle,
		vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR | vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
	)
}

fn acceleration_structure_destination(handle: Handles) -> VulkanConsumption {
	acceleration_structure_build_access(handle, vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR)
}

/// A bottom-level structure that a top-level build reads through its instances.
fn acceleration_structure_source(handle: Handles) -> VulkanConsumption {
	acceleration_structure_build_access(handle, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR)
}

fn acceleration_structure_build_access(handle: Handles, access: vk::AccessFlags2) -> VulkanConsumption {
	vulkan_consumption(handle, vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR, access)
}

impl CommandBufferRecording<'_> {
	fn buffer_descriptor_address(&self, descriptor: &crate::BufferDescriptor) -> vk::DeviceAddress {
		let buffer = self.get_buffer(self.get_internal_buffer_handle(descriptor.buffer)).buffer;
		let address = unsafe {
			self.device
				.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer))
		};
		address + descriptor.offset as u64
	}

	fn acceleration_structure_build_info(
		&self,
		ty: vk::AccelerationStructureTypeKHR,
		destination: &AccelerationStructure,
		scratch_buffer: &crate::BufferDescriptor,
	) -> vk::AccelerationStructureBuildGeometryInfoKHR<'static> {
		vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(ty)
			.dst_acceleration_structure(destination.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR {
				device_address: self.buffer_descriptor_address(scratch_buffer),
			})
	}

	fn record_acceleration_structure_builds(
		&self,
		builds: &[(
			vk::AccelerationStructureBuildGeometryInfoKHR,
			Vec<vk::AccelerationStructureGeometryKHR>,
			Vec<vk::AccelerationStructureBuildRangeInfoKHR>,
		)],
	) {
		let infos = builds
			.iter()
			.map(|(info, geometries, _)| info.geometries(geometries))
			.collect::<Vec<_>>();
		let build_range_infos = builds
			.iter()
			.map(|(_, _, ranges)| Some(ranges.as_slice()))
			.collect::<Vec<_>>();
		unsafe {
			self.device.acceleration_structure.cmd_build_acceleration_structures(
				self.get_command_buffer().command_buffer,
				&infos,
				&build_range_infos,
			)
		}
	}

	fn record_buffer_to_image_copy(&self, buffer: vk::Buffer, image: vk::Image, region: vk::BufferImageCopy2) {
		let regions = [region];
		let copy = vk::CopyBufferToImageInfo2::default()
			.src_buffer(buffer)
			.dst_image(image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&regions);
		unsafe {
			self.device
				.device
				.cmd_copy_buffer_to_image2(self.get_command_buffer().command_buffer, &copy);
		}
	}
}

impl crate::command_buffer::CommandBufferRecording for CommandBufferRecording<'_> {
	fn frame_key(&self) -> FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn transfer_texture(
		&mut self,
		source: graphics_hardware_interface::ImageOrSwapchain,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		let (source_handle, format, extent, declared_uses) = match source {
			graphics_hardware_interface::ImageOrSwapchain::Image(handle) => {
				if self.device.images.get(handle.0 as usize).is_none() {
					return Err(crate::TextureTransferError::InvalidSource);
				}
				let source_handle = self.get_internal_base_image_handle(handle);
				let image = self
					.device
					.images
					.get(source_handle.0 as usize)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
				(source_handle, image.format_, image.extent, image.uses)
			}
			graphics_hardware_interface::ImageOrSwapchain::Swapchain(handle) => {
				let swapchain = self
					.device
					.swapchains
					.get(handle.0 as usize)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
				let image_index = usize::from(swapchain.acquired_image_indices[self.sequence_index as usize]);
				let source_handle = *swapchain
					.images
					.get(image_index)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
				let source_image = self
					.device
					.images
					.get(source_handle.0 as usize)
					.ok_or(crate::TextureTransferError::InvalidSource)?;
				let declared_uses = if swapchain.uses_proxy_images {
					swapchain.proxy_uses
				} else {
					source_image.uses
				};
				(
					source_handle,
					source_image.format_,
					Extent::rectangle(swapchain.extent.width, swapchain.extent.height),
					declared_uses,
				)
			}
		};
		self.record_texture_transfer(source_handle, format, extent, declared_uses)
	}

	fn transfer_texture_with_frame(
		&mut self,
		image: graphics_hardware_interface::DynamicImageHandle,
		frame_offset: i32,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		let handle = graphics_hardware_interface::BaseImageHandle::from(image);
		if self.device.images.get(handle.0 as usize).is_none() {
			return Err(crate::TextureTransferError::InvalidSource);
		}
		// Descriptor writes select other frames' copies the same way, so both paths agree on which copy is "previous".
		let source_handle = self.device.resolve_descriptor_image_handle(
			graphics_hardware_interface::ImageHandle(handle),
			self.sequence_index as usize,
			frame_offset,
		);
		let source_image = self
			.device
			.images
			.get(source_handle.0 as usize)
			.ok_or(crate::TextureTransferError::InvalidSource)?;
		let (format, extent, uses) = (source_image.format_, source_image.extent, source_image.uses);
		self.record_texture_transfer(source_handle, format, extent, uses)
	}

	fn start_render_pass(
		&mut self,
		extent: Extent,
		attachments: &[graphics_hardware_interface::AttachmentInformation],
	) -> &mut impl crate::command_buffer::RasterizationRenderPassMode {
		assert!(
			!self.active_rendering && self.pending_rendering.is_none(),
			"A Vulkan render pass is already active. The most likely cause is that start_render_pass was called twice without end_render_pass.",
		);
		graphics_hardware_interface::AttachmentInformation::render_pass_layer_count(attachments);
		for attachment in attachments {
			self.get_attachment_image_view(attachment);
			// A pass that clears or discards an attachment gives an image-group member new contents.
			if let (graphics_hardware_interface::ImageOrSwapchain::Image(image), false) =
				(attachment.target, attachment.loads())
			{
				self.initialize_group_member(image);
			}
		}
		self.consume_resources(attachments.iter().map(|attachment| Consumption {
			handle: Handles::Image(self.get_attachment_image_handle(attachment)),
			stages: crate::Stages::FRAGMENT,
			access: if attachment.loads() {
				crate::AccessPolicies::READ_WRITE
			} else {
				crate::AccessPolicies::WRITE
			},
			layout: attachment.layout,
		}))
		.apply(self);
		// Delay vkCmdBeginRendering until the first draw so descriptor resources can transition outside rendering.
		self.pending_rendering = Some((extent, attachments.iter().copied().collect()));
		self
	}

	fn build_top_level_acceleration_structure(&mut self, build: &crate::rt::TopLevelAccelerationStructureBuild) {
		let crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
			instances_buffer,
			instance_count,
		} = build.description;
		let top_level_handle =
			Handles::TopLevelAccelerationStructure(TopLevelAccelerationStructureHandle(build.acceleration_structure.0));
		// Instances reference bottom-level structures by address, so wait for every build still pending a write.
		let pending_bottom_level_builds = self
			.states
			.iter()
			.filter(|(handle, state)| {
				matches!(handle, Handles::BottomLevelAccelerationStructure(_))
					&& TransitionState::access_includes_write(state.access)
			})
			.map(|(handle, _)| *handle)
			.collect::<SmallVec<[Handles; 16]>>();
		let consumptions = [
			acceleration_structure_input(self.buffer_resource(instances_buffer)),
			acceleration_structure_scratch(self.buffer_resource(build.scratch_buffer.buffer)),
			acceleration_structure_destination(top_level_handle),
		]
		.into_iter()
		.chain(pending_bottom_level_builds.into_iter().map(acceleration_structure_source));
		self.vulkan_consume_resources(consumptions).apply(self);

		let geometry = vk::AccelerationStructureGeometryKHR::default()
			.geometry_type(vk::GeometryTypeKHR::INSTANCES)
			.geometry(vk::AccelerationStructureGeometryDataKHR {
				instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
					.array_of_pointers(false)
					.data(vk::DeviceOrHostAddressConstKHR {
						device_address: self.device.get_buffer_address(instances_buffer),
					}),
			})
			.flags(vk::GeometryFlagsKHR::OPAQUE);
		let range = vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(instance_count);
		let info = self.acceleration_structure_build_info(
			vk::AccelerationStructureTypeKHR::TOP_LEVEL,
			&self.device.acceleration_structures[build.acceleration_structure.0 as usize],
			&build.scratch_buffer,
		);
		self.record_acceleration_structure_builds(&[(info, vec![geometry], vec![range])]);
	}

	fn build_bottom_level_acceleration_structures(&mut self, builds: &[crate::rt::BottomLevelAccelerationStructureBuild]) {
		if builds.is_empty() {
			return;
		}

		let mut consumptions = SmallVec::<[VulkanConsumption; 16]>::new();
		for build in builds {
			consumptions.push(acceleration_structure_destination(Handles::BottomLevelAccelerationStructure(
				BottomLevelAccelerationStructureHandle(build.acceleration_structure.0),
			)));
			consumptions.push(acceleration_structure_scratch(
				self.buffer_resource(build.scratch_buffer.buffer),
			));
			if let crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
				vertex_buffer,
				index_buffer,
				..
			} = &build.description
			{
				for input in [vertex_buffer.buffer_offset.buffer, index_buffer.buffer_offset.buffer] {
					consumptions.push(acceleration_structure_input(self.buffer_resource(input)));
				}
			}
		}
		self.vulkan_consume_resources(consumptions).apply(self);

		let build_infos = builds
			.iter()
			.map(|build| {
				let (geometries, ranges) = match &build.description {
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB { .. } => (Vec::new(), Vec::new()),
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer,
						index_buffer,
						vertex_position_encoding,
						index_format,
						triangle_count,
						vertex_count,
					} => {
						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR {
								device_address: self.buffer_descriptor_address(&vertex_buffer.buffer_offset),
							})
							.index_data(vk::DeviceOrHostAddressConstKHR {
								device_address: self.buffer_descriptor_address(&index_buffer.buffer_offset),
							})
							.max_vertex(vertex_count - 1)
							.vertex_format(match vertex_position_encoding {
								crate::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position encoding"),
							})
							.index_type(match index_format {
								crate::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								crate::DataTypes::U16 => vk::IndexType::UINT16,
								crate::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
							.vertex_stride(vertex_buffer.stride as vk::DeviceSize);
						let geometry = vk::AccelerationStructureGeometryKHR::default()
							.flags(vk::GeometryFlagsKHR::OPAQUE)
							.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
							.geometry(vk::AccelerationStructureGeometryDataKHR { triangles });
						let range = vk::AccelerationStructureBuildRangeInfoKHR::default().primitive_count(*triangle_count);
						(vec![geometry], vec![range])
					}
				};
				let info = self.acceleration_structure_build_info(
					vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL,
					&self.device.acceleration_structures[build.acceleration_structure.0 as usize],
					&build.scratch_buffer,
				);
				(info, geometries, ranges)
			})
			.collect::<Vec<_>>();
		self.record_acceleration_structure_builds(&build_infos);
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		destination_layout: crate::Layouts,
	) {
		let source_handle = self.get_internal_base_image_handle(source_image);
		let destination_handle = self.get_internal_base_image_handle(destination_image);
		self.consume_resources([
			Consumption {
				layout: source_layout,
				..transfer_consumption(Handles::Image(source_handle), crate::AccessPolicies::READ)
			},
			Consumption {
				layout: destination_layout,
				..transfer_consumption(Handles::Image(destination_handle), crate::AccessPolicies::WRITE)
			},
		])
		.apply(self);

		let source_image = self.get_image(source_handle);
		let destination_image = self.get_image(destination_handle);
		let far_corner = |image: &Image| vk::Offset3D {
			x: image.extent.width() as i32,
			y: image.extent.height() as i32,
			z: 1,
		};
		let blits = [vk::ImageBlit2::default()
			.src_subresource(subresource_layers(image_aspect_mask(source_image.format), 0, 1))
			.src_offsets([vk::Offset3D::default(), far_corner(source_image)])
			.dst_subresource(subresource_layers(image_aspect_mask(destination_image.format), 0, 1))
			.dst_offsets([vk::Offset3D::default(), far_corner(destination_image)])];
		let blit_info = vk::BlitImageInfo2::default()
			.src_image(source_image.image)
			.src_image_layout(texture_format_and_resource_use_to_image_layout(
				source_image.format_,
				source_layout,
				Some(crate::AccessPolicies::READ),
			))
			.dst_image(destination_image.image)
			.dst_image_layout(texture_format_and_resource_use_to_image_layout(
				destination_image.format_,
				destination_layout,
				Some(crate::AccessPolicies::WRITE),
			))
			.regions(&blits)
			.filter(vk::Filter::LINEAR);
		unsafe {
			self.device
				.device
				.cmd_blit_image2(self.get_command_buffer().command_buffer, &blit_info);
		}
	}

	fn discard_images(&mut self, images: &[graphics_hardware_interface::BaseImageHandle]) {
		for &image in images {
			if !self.initialize_group_member(image) {
				// An image outside a group only loses its contents: its next barrier starts from an undefined layout.
				let handle = Handles::Image(self.get_internal_base_image_handle(image));
				if let Some(state) = self.states.get_mut(&handle) {
					state.layout = vk::ImageLayout::UNDEFINED;
				}
			}
		}
	}

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		for (image_handle, _) in textures {
			self.initialize_group_member(*image_handle);
		}
		self.consume_resources(
			textures.iter().map(|(image_handle, _)| {
				transfer_consumption(self.image_resource(*image_handle), crate::AccessPolicies::WRITE)
			}),
		)
		.apply(self);

		for (image_handle, clear_value) in textures {
			let image = self.get_image(self.get_internal_base_image_handle(*image_handle));
			// Skip unset textures.
			if image.image.is_null() {
				continue;
			}

			let command_buffer = self.get_command_buffer().command_buffer;
			let is_depth = image.format_.is_depth();
			let range = vk::ImageSubresourceRange::default()
				.aspect_mask(if is_depth {
					vk::ImageAspectFlags::DEPTH
				} else {
					vk::ImageAspectFlags::COLOR
				})
				.level_count(vk::REMAINING_MIP_LEVELS)
				.layer_count(vk::REMAINING_ARRAY_LAYERS);
			let layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
			if is_depth {
				let depth = match clear_value {
					graphics_hardware_interface::ClearValue::None => 0.0,
					graphics_hardware_interface::ClearValue::Depth(depth) => *depth,
					graphics_hardware_interface::ClearValue::Color(_) => panic!("Color clear value for depth texture"),
					graphics_hardware_interface::ClearValue::Integer(..) => panic!("Integer clear value for depth texture"),
				};
				let clear_value = vk::ClearDepthStencilValue { depth, stencil: 0 };
				unsafe {
					self.device.device.cmd_clear_depth_stencil_image(
						command_buffer,
						image.image,
						layout,
						&clear_value,
						&[range],
					);
				}
			} else {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearColorValue::default(),
					graphics_hardware_interface::ClearValue::Color(color) => vk::ClearColorValue {
						float32: [color.r, color.g, color.b, color.a],
					},
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearColorValue {
						float32: [*depth, 0.0, 0.0, 0.0],
					},
					graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearColorValue {
						uint32: [*r, *g, *b, *a],
					},
				};
				unsafe {
					self.device
						.device
						.cmd_clear_color_image(command_buffer, image.image, layout, &clear_value, &[range]);
				}
			}
		}
	}

	fn copy_buffers(&mut self, copies: &[crate::BufferCopyDescriptor]) {
		let copies = copies
			.iter()
			.filter(|copy| copy.size > 0)
			.map(|copy| {
				BufferCopy::new(
					self.get_internal_buffer_handle(copy.source_buffer),
					copy.source_offset as vk::DeviceSize,
					self.get_internal_buffer_handle(copy.destination_buffer),
					copy.destination_offset as vk::DeviceSize,
					copy.size,
				)
			})
			.collect::<Vec<_>>();
		self.sync_buffers(copies.into_iter());
	}

	fn copy_buffer_to_images(&mut self, copies: &[crate::BufferImageCopyDescriptor]) {
		self.consume_resources(copies.iter().flat_map(|copy| {
			[
				transfer_consumption(self.buffer_resource(copy.source_buffer), crate::AccessPolicies::READ),
				transfer_consumption(self.image_resource(copy.destination_image), crate::AccessPolicies::WRITE),
			]
		}))
		.apply(self);

		for copy in copies {
			let source_buffer = self.get_buffer(self.get_internal_buffer_handle(copy.source_buffer)).buffer;
			let destination_image = self.get_image(self.get_internal_base_image_handle(copy.destination_image));
			assert!(
				copy.destination_mip_level < destination_image.mip_levels,
				"Vulkan texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image."
			);
			let destination_extent = crate::image::mip_extent(destination_image.extent, copy.destination_mip_level);
			let source_row_count = copy.source_bytes_per_image / copy.source_bytes_per_row;
			let subresource = subresource_layers(
				image_aspect_mask(destination_image.format),
				copy.destination_mip_level,
				destination_image.layers.map_or(1, std::num::NonZeroU32::get),
			);
			let region = vk::BufferImageCopy2::default()
				.buffer_offset(copy.source_offset as _)
				.buffer_row_length(buffer_row_length(destination_image.format_, copy.source_bytes_per_row))
				.buffer_image_height(buffer_image_height(destination_image.format_, source_row_count))
				.image_subresource(subresource)
				.image_extent(extent_into_vk_extent(destination_extent));
			self.record_buffer_to_image_copy(source_buffer, destination_image.image, region);
		}

		self.consume_resources(copies.iter().map(|copy| Consumption {
			handle: self.image_resource(copy.destination_image),
			stages: crate::Stages::COMPUTE | crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}))
		.apply(self);
	}

	fn sync_buffer(&mut self, buffer_handle: impl Into<graphics_hardware_interface::BaseBufferHandle>) {
		CommandBufferRecording::sync_buffer(self, buffer_handle);
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		self.consume_resources(
			buffer_handles
				.iter()
				.map(|buffer_handle| transfer_consumption(self.buffer_resource(*buffer_handle), crate::AccessPolicies::WRITE)),
		)
		.apply(self);

		for buffer_handle in buffer_handles {
			let handle = self.get_internal_buffer_handle(*buffer_handle);
			let buffer = self.get_buffer(handle).buffer;
			if buffer.is_null() {
				continue;
			}

			unsafe {
				self.device
					.device
					.cmd_fill_buffer(self.get_command_buffer().command_buffer, buffer, 0, vk::WHOLE_SIZE, 0);
			}
			self.states.insert(
				Handles::Buffer(handle),
				TransitionState::new(
					vk::PipelineStageFlags2::TRANSFER,
					vk::AccessFlags2::TRANSFER_WRITE,
					vk::ImageLayout::UNDEFINED,
				),
			);
		}
	}

	fn write_image_data(
		&mut self,
		image_handle: graphics_hardware_interface::BaseImageHandle,
		data: &[graphics_hardware_interface::RGBAu8],
	) {
		let internal_image_handle = self.get_internal_base_image_handle(image_handle);
		let texture = self.get_image(internal_image_handle);
		if !texture.access.contains(crate::DeviceAccesses::CpuWrite) {
			return;
		}
		let (Some(buffer), Some(pointer)) = (texture.staging_buffer, texture.pointer.map(|pointer| pointer.0)) else {
			return;
		};

		assert!(
			!pointer.is_null(),
			"Vulkan image upload pointer is null. The most likely cause is that the host-visible staging allocation was not mapped."
		);
		assert_eq!(
			texture.format_.size(),
			std::mem::size_of::<graphics_hardware_interface::RGBAu8>(),
			"Unsupported Vulkan RGBA image upload format. The most likely cause is that write_image_data was used with a compressed or non-four-byte format."
		);

		let layer_count = texture.layers.map_or(1, std::num::NonZeroU32::get);
		let pixel_count = texture
			.extent
			.width()
			.checked_mul(texture.extent.height().max(1))
			.and_then(|count| count.checked_mul(texture.extent.depth().max(1)))
			.and_then(|count| count.checked_mul(layer_count))
			.expect("Vulkan image upload size overflowed. The most likely cause is an invalid extent or array-layer count.")
			as usize;
		let required_bytes = pixel_count
			.checked_mul(std::mem::size_of::<graphics_hardware_interface::RGBAu8>())
			.expect("Vulkan image upload byte size overflowed. The most likely cause is an oversized image.");

		assert!(
			data.len() >= pixel_count,
			"Vulkan image upload data is too small. The most likely cause is that the source does not contain every pixel and array layer."
		);
		assert!(
			required_bytes <= texture.size,
			"Vulkan image upload staging storage is too small. The most likely cause is that the image staging allocation does not include every array layer."
		);
		let image = texture.image;
		let extent = texture.extent;

		// The Vulkan staging buffer is tightly packed; image-memory row pitches do not apply to it.
		unsafe {
			std::ptr::copy_nonoverlapping(data.as_ptr().cast::<u8>(), pointer, required_bytes);
		}

		self.consume_resources([transfer_consumption(
			Handles::Image(internal_image_handle),
			crate::AccessPolicies::WRITE,
		)])
		.apply(self);
		let region = vk::BufferImageCopy2::default()
			.image_subresource(subresource_layers(vk::ImageAspectFlags::COLOR, 0, layer_count))
			.image_extent(extent_into_vk_extent(extent));
		self.record_buffer_to_image_copy(buffer, image, region);
		self.consume_resources([Consumption {
			handle: Handles::Image(internal_image_handle),
			stages: crate::Stages::FRAGMENT,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Read,
		}])
		.apply(self);
	}

	fn execute(mut self, synchronizer: crate::SynchronizerHandle) {
		self.consume_last_resources();
		self.end_recording();

		let command_buffer = self.get_command_buffer();
		let command_buffer_infos = [vk::CommandBufferSubmitInfo::default().command_buffer(command_buffer.command_buffer)];
		let submit_info = vk::SubmitInfo2::default().command_buffer_infos(&command_buffer_infos);
		let synchronizer_handle = self.device.get_syncronizer_handles(synchronizer)[self.sequence_index as usize];
		let synchronizer = &self.device.synchronizers[synchronizer_handle.0 as usize];

		unsafe {
			self.device.device.reset_fences(&[synchronizer.fence]).expect(
				"Failed to reset Vulkan command buffer synchronizer. The most likely cause is that the fence is invalid or already in use.",
			);
			let vk_queue = self.device.vk_queues[command_buffer.vk_queue_index]
				.lock()
				.expect("Failed to lock Vulkan queue for command-buffer submission. The most likely cause is that another thread panicked while holding the queue lock.");
			self.device
				.device
				.queue_submit2(*vk_queue, &[submit_info], synchronizer.fence)
				.expect("Failed to submit Vulkan command buffer. The most likely cause is that the command buffer was not recorded for this queue.");
		}
		self.device.synchronizers[synchronizer_handle.0 as usize].armed = true;

		for handle in &self.texture_readbacks {
			self.device.texture_readbacks.mark_submitted(*handle);
		}
		self.readbacks_finalized = true;
		self.device.states.extend(std::mem::take(&mut self.states));
		self.device.buffer_states.extend(std::mem::take(&mut self.buffer_states));
	}
}

impl CommandBufferRecording<'_> {
	/// Records one copy of a resolved image into CPU-readable staging.
	///
	/// `declared_uses` are the uses the public source was created with, which differ from the native image's uses
	/// for proxied swapchains.
	fn record_texture_transfer(
		&mut self,
		source_handle: ImageHandle,
		format: crate::Formats,
		extent: Extent,
		declared_uses: crate::Uses,
	) -> Result<graphics_hardware_interface::TextureCopyHandle, crate::TextureTransferError> {
		let image = self
			.device
			.images
			.get(source_handle.0 as usize)
			.ok_or(crate::TextureTransferError::InvalidSource)?;
		let array_layers = image.layers.map_or(1, std::num::NonZeroU32::get);
		let source_image = image.image;
		let aspect_mask = image_aspect_mask(image.format);
		if image.format == vk::Format::UNDEFINED {
			return Err(crate::TextureTransferError::UnsupportedFormat(format));
		}
		let layout = crate::context::texture_transfer_layout(format, extent, array_layers, declared_uses)?;
		let size = layout
			.bytes_per_image
			.checked_mul(layout.depth_slices)
			.ok_or(crate::TextureTransferError::UnsupportedLayout)?;
		let (staging, memory, pointer) = self.device.create_texture_readback_buffer(size)?;

		// Register staging before state tracking so unwinding the recording can reclaim every native object.
		let handle = self.device.texture_readbacks.insert(TextureReadbackStorage {
			buffer: staging,
			memory,
			pointer: crate::vulkan::MappedMemoryPointer(pointer),
			extent,
			format,
			bytes_per_row: layout.bytes_per_row,
			bytes_per_image: layout.bytes_per_image,
			size,
		});
		self.texture_readbacks.push(handle);

		self.consume_resources([transfer_consumption(
			Handles::Image(source_handle),
			crate::AccessPolicies::READ,
		)])
		.apply(self);
		self.vulkan_consume_resources([vulkan_consumption(
			Handles::VkBuffer(staging),
			vk::PipelineStageFlags2::TRANSFER,
			vk::AccessFlags2::TRANSFER_WRITE,
		)])
		.apply(self);

		let regions = [vk::BufferImageCopy2::default()
			.image_subresource(subresource_layers(aspect_mask, 0, 1))
			.image_extent(extent_into_vk_extent(extent))];
		let copy = vk::CopyImageToBufferInfo2::default()
			.src_image(source_image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_buffer(staging)
			.regions(&regions);
		// Waiting on the fence only makes the copy available; mapped host reads also need it made visible to the host.
		let host_read_barriers = [vk::BufferMemoryBarrier2::default()
			.src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
			.src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
			.dst_stage_mask(vk::PipelineStageFlags2::HOST)
			.dst_access_mask(vk::AccessFlags2::HOST_READ)
			.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
			.buffer(staging)
			.size(vk::WHOLE_SIZE)];
		let command_buffer = self.get_command_buffer().command_buffer;
		unsafe {
			self.device.device.cmd_copy_image_to_buffer2(command_buffer, &copy);
			self.device.device.cmd_pipeline_barrier2(
				command_buffer,
				&vk::DependencyInfo::default().buffer_memory_barriers(&host_read_barriers),
			);
		}

		Ok(handle)
	}
}
