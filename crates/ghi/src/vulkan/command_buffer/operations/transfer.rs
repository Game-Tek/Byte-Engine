use super::*;

impl crate::command_buffer::CommandBufferRecording for CommandBufferRecording<'_> {
	fn frame_key(&self) -> FrameKey {
		self.frame_key.expect(
			"Command buffer recording has no frame key. The most likely cause is that it was created from a command buffer instead of a frame.",
		)
	}

	fn transfer_textures(
		&mut self,
		image_handles: &[graphics_hardware_interface::BaseImageHandle],
	) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		// CPU-write images also own staging buffers, but those buffers cannot receive readbacks.
		let readbacks = image_handles
			.iter()
			.filter_map(|image_handle| {
				let internal = self.get_internal_base_image_handle(*image_handle);
				let image = self.get_image(internal);
				image
					.access
					.contains(crate::DeviceAccesses::CpuRead)
					.then_some(image.staging_buffer)
					.flatten()
					.map(|staging| (internal, staging))
			})
			.collect::<SmallVec<[_; 8]>>();
		if readbacks.is_empty() {
			return Vec::new();
		}

		self.consume_resources(readbacks.iter().map(|(image, _)| Consumption {
			handle: Handles::Image(*image),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::READ,
			layout: crate::Layouts::Transfer,
		}))
		.apply(self);
		self.vulkan_consume_resources(readbacks.iter().map(|(_, buffer)| VulkanConsumption {
			handle: Handles::VkBuffer(*buffer),
			stages: vk::PipelineStageFlags2::TRANSFER,
			access: vk::AccessFlags2::TRANSFER_WRITE,
			layout: vk::ImageLayout::UNDEFINED,
			range: None,
		}))
		.apply(self);

		let command_buffer = self.get_command_buffer().command_buffer;
		for (image_handle, staging_buffer) in &readbacks {
			let image = self.get_image(*image_handle);
			let layer_count = image.layers.map_or(1, std::num::NonZeroU32::get);
			let aspect = if image.format_.is_depth() {
				vk::ImageAspectFlags::DEPTH
			} else {
				vk::ImageAspectFlags::COLOR
			};
			let regions = [vk::BufferImageCopy2KHR::default()
				.buffer_offset(0)
				.buffer_row_length(0)
				.buffer_image_height(0)
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(aspect)
						.mip_level(0)
						.base_array_layer(0)
						.layer_count(layer_count),
				)
				.image_offset(vk::Offset3D::default())
				.image_extent(extent_into_vk_extent(image.extent))];
			let copy = vk::CopyImageToBufferInfo2KHR::default()
				.src_image(image.image)
				.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
				.dst_buffer(*staging_buffer)
				.regions(&regions);

			unsafe {
				self.device.device.cmd_copy_image_to_buffer2(command_buffer, &copy);
			}
		}

		readbacks
			.into_iter()
			.map(|(image, _)| graphics_hardware_interface::TextureCopyHandle(image.0))
			.collect()
	}

	fn copy_images_to_buffer(&mut self, copies: &[crate::ImageBufferCopyDescriptor]) {
		if copies.is_empty() {
			return;
		}

		let resolved = copies
			.iter()
			.map(|copy| {
				let source = self.get_image_or_swapchain_handle(copy.source);
				let destination_root = self.get_internal_buffer_handle(copy.destination_buffer);
				let destination_buffer = self.get_buffer(destination_root);
				// Only CPU-read staging buffers have transfer-destination usage; CPU-write staging buffers are sources.
				let destination = if destination_buffer.access.contains(crate::DeviceAccesses::CpuRead) {
					destination_buffer.staging.unwrap_or(destination_root)
				} else {
					destination_root
				};
				(*copy, source, destination)
			})
			.collect::<SmallVec<[_; 8]>>();

		self.consume_resources(resolved.iter().flat_map(|(_, source, destination)| {
			[
				Consumption {
					handle: Handles::Image(*source),
					stages: crate::Stages::TRANSFER,
					access: crate::AccessPolicies::READ,
					layout: crate::Layouts::Transfer,
				},
				Consumption {
					handle: Handles::Buffer(*destination),
					stages: crate::Stages::TRANSFER,
					access: crate::AccessPolicies::WRITE,
					layout: crate::Layouts::Transfer,
				},
			]
		}))
		.apply(self);

		let command_buffer = self.get_command_buffer().command_buffer;
		for (copy, source_handle, destination_handle) in resolved {
			let source = self.get_image(source_handle);
			let destination = self.get_buffer(destination_handle);
			let layer_count = source.layers.map_or(1, std::num::NonZeroU32::get);
			let (compact_bytes_per_row, compact_row_count, _) = source
				.format_
				.compact_copy_layout(source.extent.width().max(1), source.extent.height().max(1));

			assert!(
				copy.destination_bytes_per_row >= compact_bytes_per_row
					&& copy.destination_bytes_per_image >= copy.destination_bytes_per_row * compact_row_count
					&& copy.destination_bytes_per_image % copy.destination_bytes_per_row == 0,
				"Invalid Vulkan image readback pitch. The most likely cause is that the destination row or image pitch is smaller than the source image layout."
			);
			let required_bytes = copy
				.destination_bytes_per_image
				.checked_mul(layer_count as usize)
				.and_then(|size| copy.destination_offset.checked_add(size))
				.expect(
					"Vulkan image readback bounds overflowed. The most likely cause is an invalid destination offset, pitch, or layer count.",
				);

			assert!(
				required_bytes <= destination.size,
				"Vulkan image readback destination is too small. The most likely cause is that the destination buffer does not contain every copied layer."
			);

			let row_count = copy.destination_bytes_per_image / copy.destination_bytes_per_row;
			let aspect = if source.format_.is_depth() {
				vk::ImageAspectFlags::DEPTH
			} else {
				vk::ImageAspectFlags::COLOR
			};
			let regions = [vk::BufferImageCopy2KHR::default()
				.buffer_offset(copy.destination_offset as _)
				.buffer_row_length(buffer_row_length(source.format_, copy.destination_bytes_per_row))
				.buffer_image_height(buffer_image_height(source.format_, row_count))
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(aspect)
						.mip_level(0)
						.base_array_layer(0)
						.layer_count(layer_count),
				)
				.image_offset(vk::Offset3D::default())
				.image_extent(extent_into_vk_extent(source.extent))];
			let copy_info = vk::CopyImageToBufferInfo2::default()
				.src_image(source.image)
				.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
				.dst_buffer(destination.buffer)
				.regions(&regions);

			unsafe {
				self.device.device.cmd_copy_image_to_buffer2(command_buffer, &copy_info);
			}
		}
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
		}
		self.consume_resources(attachments.iter().map(|attachment| Consumption {
			handle: Handles::Image(self.get_attachment_image_handle(attachment)),
			stages: crate::Stages::FRAGMENT,
			access: if attachment.load {
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

	fn build_top_level_acceleration_structure(
		&mut self,
		acceleration_structure_build: &crate::rt::TopLevelAccelerationStructureBuild,
	) {
		let (acceleration_structure_handle, acceleration_structure) =
			self.get_top_level_acceleration_structure(acceleration_structure_build.acceleration_structure);

		let (as_geometries, offsets) = match acceleration_structure_build.description {
			crate::rt::TopLevelAccelerationStructureBuildDescriptions::Instance {
				instances_buffer,
				instance_count,
			} => (
				vec![vk::AccelerationStructureGeometryKHR::default()
					.geometry_type(vk::GeometryTypeKHR::INSTANCES)
					.geometry(vk::AccelerationStructureGeometryDataKHR {
						instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
							.array_of_pointers(false)
							.data(vk::DeviceOrHostAddressConstKHR {
								device_address: self.device.get_buffer_address(instances_buffer),
							}),
					})
					.flags(vk::GeometryFlagsKHR::OPAQUE)],
				vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
					.primitive_count(instance_count)
					.primitive_offset(0)
					.first_vertex(0)
					.transform_offset(0)],
			),
		};

		let scratch_buffer_address = unsafe {
			let buffer = self.get_buffer(self.get_internal_buffer_handle(acceleration_structure_build.scratch_buffer.buffer));
			self.device
				.device
				.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
				+ acceleration_structure_build.scratch_buffer.offset as u64
		};

		let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.dst_acceleration_structure(acceleration_structure.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR {
				device_address: scratch_buffer_address,
			});

		self.states.insert(
			Handles::TopLevelAccelerationStructure(
				self.get_internal_top_level_acceleration_structure_handle(acceleration_structure_handle),
			),
			TransitionState::new(
				vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
				vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
				vk::ImageLayout::UNDEFINED,
			),
		);

		let infos = vec![build_geometry_info];
		let build_range_infos = vec![offsets];
		let geometries = vec![as_geometries];

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let infos = infos
			.iter()
			.zip(geometries.iter())
			.map(|(info, geos)| info.geometries(geos))
			.collect::<Vec<_>>();

		let build_range_infos = build_range_infos
			.iter()
			.map(|build_range_info| Some(build_range_info.as_slice()))
			.collect::<Vec<_>>();

		unsafe {
			self.device
				.acceleration_structure
				.cmd_build_acceleration_structures(vk_command_buffer, &infos, &build_range_infos)
		}
	}

	fn build_bottom_level_acceleration_structures(
		&mut self,
		acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
	) {
		if acceleration_structure_builds.is_empty() {
			return;
		}

		fn visit(
			this: &mut CommandBufferRecording,
			acceleration_structure_builds: &[crate::rt::BottomLevelAccelerationStructureBuild],
			mut infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR>,
			mut geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR>>,
			mut build_range_infos: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>>,
		) {
			if let Some(build) = acceleration_structure_builds.first() {
				let (acceleration_structure_handle, acceleration_structure) =
					this.get_bottom_level_acceleration_structure(build.acceleration_structure);

				let (as_geometries, offsets) = match &build.description {
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::AABB { .. } => (vec![], vec![]),
					crate::rt::BottomLevelAccelerationStructureBuildDescriptions::Mesh {
						vertex_buffer,
						index_buffer,
						vertex_position_encoding,
						index_format,
						triangle_count,
						vertex_count,
					} => {
						let vertex_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(vertex_buffer.buffer_offset.buffer));
							this.device
								.device
								.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
								+ vertex_buffer.buffer_offset.offset as u64
						};

						let index_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(index_buffer.buffer_offset.buffer));
							this.device
								.device
								.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
								+ index_buffer.buffer_offset.offset as u64
						};

						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR {
								device_address: vertex_data_address,
							})
							.index_data(vk::DeviceOrHostAddressConstKHR {
								device_address: index_data_address,
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

						let build_range_info = vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
							.primitive_count(*triangle_count)
							.primitive_offset(0)
							.first_vertex(0)
							.transform_offset(0)];

						(
							vec![vk::AccelerationStructureGeometryKHR::default()
								.flags(vk::GeometryFlagsKHR::OPAQUE)
								.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
								.geometry(vk::AccelerationStructureGeometryDataKHR { triangles })],
							build_range_info,
						)
					}
				};

				let scratch_buffer_address = unsafe {
					let buffer = this.get_buffer(this.get_internal_buffer_handle(build.scratch_buffer.buffer));
					this.device
						.device
						.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer))
						+ build.scratch_buffer.offset as u64
				};

				let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.dst_acceleration_structure(acceleration_structure.acceleration_structure)
					.scratch_data(vk::DeviceOrHostAddressKHR {
						device_address: scratch_buffer_address,
					});

				this.states.insert(
					Handles::BottomLevelAccelerationStructure(
						this.get_internal_bottom_level_acceleration_structure_handle(acceleration_structure_handle),
					),
					TransitionState::new(
						vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
						vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
						vk::ImageLayout::UNDEFINED,
					),
				);

				infos.push(build_geometry_info);
				build_range_infos.push(offsets);
				geometries.push(as_geometries);

				visit(
					this,
					&acceleration_structure_builds[1..],
					infos,
					geometries,
					build_range_infos,
				);
			} else {
				let command_buffer = this.get_command_buffer();

				let infos = infos
					.iter()
					.zip(geometries.iter())
					.map(|(info, geos)| info.geometries(geos))
					.collect::<Vec<_>>();

				let build_range_infos = build_range_infos
					.iter()
					.map(|build_range_info| Some(build_range_info.as_slice()))
					.collect::<Vec<_>>();

				unsafe {
					this.device.acceleration_structure.cmd_build_acceleration_structures(
						command_buffer.command_buffer,
						&infos,
						&build_range_infos,
					)
				}
			}
		}

		visit(self, acceleration_structure_builds, Vec::new(), Vec::new(), Vec::new());
	}

	fn blit_image(
		&mut self,
		source_image: graphics_hardware_interface::BaseImageHandle,
		source_layout: crate::Layouts,
		destination_image: graphics_hardware_interface::BaseImageHandle,
		destination_layout: crate::Layouts,
	) {
		self.consume_resources([
			Consumption {
				handle: Handles::Image(self.get_internal_base_image_handle(source_image)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::READ,
				layout: source_layout,
			},
			Consumption {
				handle: Handles::Image(self.get_internal_base_image_handle(destination_image)),
				stages: crate::Stages::TRANSFER,
				access: crate::AccessPolicies::WRITE,
				layout: destination_layout,
			},
		])
		.apply(self);

		let command_buffer = self.get_command_buffer();
		let source_image = self.get_image(self.get_internal_base_image_handle(source_image));
		let destination_image = self.get_image(self.get_internal_base_image_handle(destination_image));
		unsafe {
			let blit = vk::ImageBlit2::default()
				.src_subresource(vk::ImageSubresourceLayers {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					mip_level: 0,
					base_array_layer: 0,
					layer_count: 1,
				})
				.src_offsets([
					vk::Offset3D { x: 0, y: 0, z: 0 },
					vk::Offset3D {
						x: source_image.extent.width() as i32,
						y: source_image.extent.height() as i32,
						z: 1,
					},
				])
				.dst_subresource(vk::ImageSubresourceLayers {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					mip_level: 0,
					base_array_layer: 0,
					layer_count: 1,
				})
				.dst_offsets([
					vk::Offset3D { x: 0, y: 0, z: 0 },
					vk::Offset3D {
						x: destination_image.extent.width() as i32,
						y: destination_image.extent.height() as i32,
						z: 1,
					},
				]);

			let blits = [blit];

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
			self.device.device.cmd_blit_image2(command_buffer.command_buffer, &blit_info);
		}
	}

	fn clear_images(
		&mut self,
		textures: &[(
			graphics_hardware_interface::BaseImageHandle,
			graphics_hardware_interface::ClearValue,
		)],
	) {
		self.consume_resources(textures.iter().map(|(image_handle, _)| Consumption {
			handle: Handles::Image(self.get_internal_base_image_handle(*image_handle)),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}))
		.apply(self);

		for (image_handle, clear_value) in textures {
			let image = self.get_image(self.get_internal_base_image_handle(*image_handle));

			if image.image.is_null() {
				continue;
			} // Skip unset textures

			if !image.format_.is_depth() {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearColorValue {
						float32: [0.0, 0.0, 0.0, 0.0],
					},
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
					self.device.device.cmd_clear_color_image(
						self.get_command_buffer().command_buffer,
						image.image,
						vk::ImageLayout::TRANSFER_DST_OPTIMAL,
						&clear_value,
						&[vk::ImageSubresourceRange {
							aspect_mask: vk::ImageAspectFlags::COLOR,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						}],
					);
				}
			} else {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearDepthStencilValue { depth: 0.0, stencil: 0 },
					graphics_hardware_interface::ClearValue::Color(_) => {
						panic!("Color clear value for depth texture")
					}
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearDepthStencilValue {
						depth: *depth,
						stencil: 0,
					},
					graphics_hardware_interface::ClearValue::Integer(..) => {
						panic!("Integer clear value for depth texture")
					}
				};

				unsafe {
					self.device.device.cmd_clear_depth_stencil_image(
						self.get_command_buffer().command_buffer,
						image.image,
						vk::ImageLayout::TRANSFER_DST_OPTIMAL,
						&clear_value,
						&[vk::ImageSubresourceRange {
							aspect_mask: vk::ImageAspectFlags::DEPTH,
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						}],
					);
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
		let consumptions = copies
			.iter()
			.flat_map(|copy| {
				[
					Consumption {
						handle: Handles::Buffer(self.get_internal_buffer_handle(copy.source_buffer)),
						stages: crate::Stages::TRANSFER,
						access: crate::AccessPolicies::READ,
						layout: crate::Layouts::Transfer,
					},
					Consumption {
						handle: Handles::Image(self.get_internal_base_image_handle(copy.destination_image)),
						stages: crate::Stages::TRANSFER,
						access: crate::AccessPolicies::WRITE,
						layout: crate::Layouts::Transfer,
					},
				]
			})
			.collect::<Vec<_>>();
		self.consume_resources(consumptions).apply(self);

		let command_buffer = self.get_command_buffer().command_buffer;

		for copy in copies {
			let source_buffer_handle = self.get_internal_buffer_handle(copy.source_buffer);
			let destination_image_handle = self.get_internal_base_image_handle(copy.destination_image);
			let source_buffer = self.get_buffer(source_buffer_handle);
			let destination_image = self.get_image(destination_image_handle);

			assert!(
				copy.destination_mip_level < destination_image.mip_levels,
				"Vulkan texture copy mip level is out of range. The most likely cause is that the upload metadata does not match the allocated image."
			);
			let destination_extent = crate::image::mip_extent(destination_image.extent, copy.destination_mip_level);
			let source_row_count = copy.source_bytes_per_image / copy.source_bytes_per_row;

			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(copy.source_offset as _)
				.buffer_row_length(buffer_row_length(destination_image.format_, copy.source_bytes_per_row))
				.buffer_image_height(buffer_image_height(destination_image.format_, source_row_count))
				.image_subresource(
					vk::ImageSubresourceLayers::default()
						.aspect_mask(vk::ImageAspectFlags::COLOR)
						.mip_level(copy.destination_mip_level)
						.base_array_layer(0)
						.layer_count(destination_image.layers.map(|layers| layers.get()).unwrap_or(1)),
				)
				.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
				.image_extent(extent_into_vk_extent(destination_extent))];

			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(source_buffer.buffer)
				.dst_image(destination_image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.device
					.device
					.cmd_copy_buffer_to_image2(command_buffer, &buffer_image_copy);
			}
		}

		self.consume_resources(copies.iter().map(|copy| Consumption {
			handle: Handles::Image(self.get_internal_base_image_handle(copy.destination_image)),
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
		self.consume_resources(buffer_handles.iter().map(|buffer_handle| Consumption {
			handle: Handles::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}))
		.apply(self);

		for buffer_handle in buffer_handles {
			let internal_buffer_handle = self.get_internal_buffer_handle(*buffer_handle);
			let buffer = self.get_buffer(internal_buffer_handle);

			if buffer.buffer.is_null() {
				continue;
			}

			unsafe {
				self.device.device.cmd_fill_buffer(
					self.get_command_buffer().command_buffer,
					buffer.buffer,
					0,
					vk::WHOLE_SIZE,
					0,
				);
			}

			self.states.insert(
				Handles::Buffer(internal_buffer_handle),
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
		let (Some(buffer), Some(pointer)) = (texture.staging_buffer, texture.pointer) else {
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

		self.consume_resources([Consumption {
			handle: Handles::Image(internal_image_handle),
			stages: crate::Stages::TRANSFER,
			access: crate::AccessPolicies::WRITE,
			layout: crate::Layouts::Transfer,
		}])
		.apply(self);

		let regions = [vk::BufferImageCopy2KHR::default()
			.buffer_offset(0)
			.buffer_row_length(0)
			.buffer_image_height(0)
			.image_subresource(
				vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(layer_count),
			)
			.image_offset(vk::Offset3D::default())
			.image_extent(extent_into_vk_extent(extent))];
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
			let vk_queue = command_buffer
				.vk_queue
				.lock()
				.expect("Failed to lock Vulkan queue for command-buffer submission. The most likely cause is that another thread panicked while holding the queue lock.");
			self.device
				.device
				.queue_submit2(*vk_queue, &[submit_info], synchronizer.fence)
				.expect("Failed to submit Vulkan command buffer. The most likely cause is that the command buffer was not recorded for this queue.");
		}

		for (handle, state) in self.states {
			self.device.states.insert(handle, state);
		}
		for (handle, states) in self.buffer_states {
			self.device.buffer_states.insert(handle, states);
		}
	}
}
