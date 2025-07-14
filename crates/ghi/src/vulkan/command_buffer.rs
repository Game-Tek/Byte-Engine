use ash::vk::{self, Handle as _};
use utils::{hash::HashMap, partition, Extent};

use crate::{graphics_hardware_interface, FrameKey};

use super::{utils::{texture_format_and_resource_use_to_image_layout, to_access_flags, to_clear_value, to_load_operation, to_pipeline_stage_flags, to_store_operation}, AccelerationStructure, BottomLevelAccelerationStructureHandle, Buffer, BufferHandle, CommandBufferInternal, Consumption, Descriptor, DescriptorSet, DescriptorSetHandle, Device, Handle, Image, ImageHandle, Swapchain, Synchronizer, TopLevelAccelerationStructureHandle, TransitionState, VulkanConsumption};

pub struct CommandBufferRecording<'a> {
	ghi: &'a mut Device,
	command_buffer: graphics_hardware_interface::CommandBufferHandle,
	in_render_pass: bool,
	sequence_index: u8,
	states: HashMap<Handle, TransitionState>,
	pipeline_bind_point: vk::PipelineBindPoint,

	stages: vk::PipelineStageFlags2,

	bound_pipeline: Option<graphics_hardware_interface::PipelineHandle>,
	bound_descriptor_set_handles: Vec<(u32, DescriptorSetHandle)>,

	buffer_copies: Vec<BufferCopy>,
}

impl CommandBufferRecording<'_> {
	pub fn new(ghi: &'_ mut Device, command_buffer: graphics_hardware_interface::CommandBufferHandle, buffer_copies: Vec<BufferCopy>, frame_key: Option<FrameKey>) -> CommandBufferRecording<'_> {
		CommandBufferRecording {
			pipeline_bind_point: vk::PipelineBindPoint::GRAPHICS,
			command_buffer,
			sequence_index: frame_key.map(|f| f.sequence_index).unwrap_or(0),
			in_render_pass: false,
			states: ghi.states.clone(),

			stages: vk::PipelineStageFlags2::empty(),

			bound_pipeline: None,
			bound_descriptor_set_handles: Vec::new(),

			buffer_copies,

			ghi,
		}
	}

	fn get_buffer(&self, buffer_handle: BufferHandle) -> &Buffer {
		&self.ghi.buffers[buffer_handle.0 as usize]
	}

	fn get_internal_image_handle(&self, handle: graphics_hardware_interface::ImageHandle) -> ImageHandle {
		let mut i = 0;
		let mut internal_image_handle = ImageHandle(handle.0);
		loop {
			let image = &self.ghi.images[internal_image_handle.0 as usize];
			if i == self.sequence_index || image.next.is_none() {
				return internal_image_handle;
			}
			internal_image_handle = image.next.unwrap();
			i += 1;
		}
	}

	fn get_image(&self, image_handle: ImageHandle) -> &Image {
		&self.ghi.images[image_handle.0 as usize]
	}

	fn get_synchronizer(&self, syncronizer_handle: graphics_hardware_interface::SynchronizerHandle) -> &Synchronizer {
		&self.ghi.synchronizers[self.ghi.get_syncronizer_handles(syncronizer_handle)[self.sequence_index as usize].0 as usize]
	}

	fn get_swapchain(&self, swapchain_handle: graphics_hardware_interface::SwapchainHandle) -> &Swapchain {
		&self.ghi.swapchains[swapchain_handle.0 as usize]
	}

	fn get_internal_top_level_acceleration_structure_handle(&self, acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle) -> TopLevelAccelerationStructureHandle {
		TopLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	fn get_top_level_acceleration_structure(&self, acceleration_structure_handle: graphics_hardware_interface::TopLevelAccelerationStructureHandle) -> (graphics_hardware_interface::TopLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.ghi.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_internal_bottom_level_acceleration_structure_handle(&self, acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle) -> BottomLevelAccelerationStructureHandle {
		BottomLevelAccelerationStructureHandle(acceleration_structure_handle.0)
	}

	fn get_bottom_level_acceleration_structure(&self, acceleration_structure_handle: graphics_hardware_interface::BottomLevelAccelerationStructureHandle) -> (graphics_hardware_interface::BottomLevelAccelerationStructureHandle, &AccelerationStructure) {
		(acceleration_structure_handle, &self.ghi.acceleration_structures[acceleration_structure_handle.0 as usize])
	}

	fn get_command_buffer(&self) -> &CommandBufferInternal {
		&self.ghi.command_buffers[self.command_buffer.0 as usize].frames[self.sequence_index as usize]
	}

	fn get_internal_descriptor_set_handle(&self, descriptor_set_handle: graphics_hardware_interface::DescriptorSetHandle) -> DescriptorSetHandle {
		let mut i = 0;
		let mut handle = DescriptorSetHandle(descriptor_set_handle.0);
		loop {
			let descriptor_set = &self.ghi.descriptor_sets[handle.0 as usize];
			if i == self.sequence_index {
				return handle;
			}
			handle = descriptor_set.next.unwrap();
			i += 1;
		}
	}

	fn get_descriptor_set(&self, descriptor_set_handle: &DescriptorSetHandle) -> &DescriptorSet {
		&self.ghi.descriptor_sets[descriptor_set_handle.0 as usize]
	}

	fn consume_resources_current(&mut self, additional_transitions: &[graphics_hardware_interface::Consumption]) {
		let mut consumptions = Vec::with_capacity(32);

		let bound_pipeline_handle = self.bound_pipeline.expect("No bound pipeline");

		let pipeline = &self.ghi.pipelines[bound_pipeline_handle.0 as usize];

		for &((set_index, binding_index), (stages, access)) in &pipeline.resource_access {
			let set_handle = if let Some(&h) = self.bound_descriptor_set_handles.get(set_index as usize) { h.1 } else {
				continue;
			};

			let resources = match self.ghi.descriptors.get(&set_handle).map(|d| d.get(&binding_index)) {
				Some(Some(b)) => b.values(),
				_ => {
					continue;
				}
			};

			for idk in resources {
				let (layout, handle) = match idk {
					Descriptor::Buffer { buffer, .. } => {
						(graphics_hardware_interface::Layouts::General, Handle::Buffer(*buffer))
					}
					Descriptor::Image { layout, image } => {
						(*layout, Handle::Image(*image))
					}
					Descriptor::CombinedImageSampler { image, layout, .. } => {
						(*layout, Handle::Image(*image))
					}
				};

				consumptions.push(Consumption { handle, stages, access, layout, });
			}
		}

		consumptions.extend(additional_transitions.iter().map(|c|
			Consumption {
				handle: self.get_internal_handle(c.handle.clone()),
				stages: c.stages,
				access: c.access,
				layout: c.layout,
			}
		));

		unsafe { self.consume_resources(&consumptions) };
	}

	unsafe fn consume_resources(&mut self, consumptions: &[Consumption]) {
		if consumptions.is_empty() { return; } // Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).

		let consumptions = consumptions.iter().map(|consumption| {
			let format = match consumption.handle {
				Handle::Image(texture_handle) => {
					let image = self.get_image(texture_handle);
					Some(image.format_)
				}
				_ => { None }
			};

			let stages = to_pipeline_stage_flags(consumption.stages, Some(consumption.layout), format);
			let access = to_access_flags(consumption.access, consumption.stages, consumption.layout, format);

			let layout = match consumption.handle {
				Handle::Image(image_handle) => {
					let image = self.get_image(image_handle);
					texture_format_and_resource_use_to_image_layout(image.format_, consumption.layout, Some(consumption.access))
				}
				_ => vk::ImageLayout::UNDEFINED
			};

			VulkanConsumption {
				handle: consumption.handle,
				stages,
				access,
				layout,
			}
		}).collect::<Vec<_>>();

		self.vulkan_consume_resources(&consumptions);
	}

	unsafe fn vulkan_consume_resources(&mut self, consumptions: &[VulkanConsumption]) {
		if consumptions.is_empty() { return; } // Skip submitting barriers if there are none (cheaper and leads to cleaner traces in GPU debugging).

		let mut image_memory_barriers = Vec::new();
		let mut buffer_memory_barriers = Vec::new();
		let mut memory_barriers = Vec::new();

		for consumption in consumptions {
			let new_stage_mask = consumption.stages;
			let new_access_mask = consumption.access;

			let transition_state = TransitionState {
				stage: new_stage_mask,
				access: new_access_mask,
				layout: consumption.layout,
			};

			if let Some(state) = self.states.get(&consumption.handle) {
				if &transition_state == state { continue; } // If current state is equal to new intended state, skip.
			}

			match consumption.handle {
				Handle::Image(handle) => {
					let image = self.get_image(handle);

					if image.image.is_null() { continue; }

					let new_layout = consumption.layout;

					let image_memory_barrier = if let Some(barrier_source) = self.states.get(&consumption.handle) {
							vk::ImageMemoryBarrier2::default().old_layout(barrier_source.layout).src_stage_mask(barrier_source.stage).src_access_mask(barrier_source.access)
						} else {
							vk::ImageMemoryBarrier2::default().old_layout(vk::ImageLayout::UNDEFINED).src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty())
						}
						.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.new_layout(new_layout)
						.dst_stage_mask(new_stage_mask)
						.dst_access_mask(new_access_mask)
						.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
						.image(image.image)
						.subresource_range(vk::ImageSubresourceRange {
							aspect_mask: if image.format != vk::Format::D32_SFLOAT { vk::ImageAspectFlags::COLOR } else { vk::ImageAspectFlags::DEPTH },
							base_mip_level: 0,
							level_count: vk::REMAINING_MIP_LEVELS,
							base_array_layer: 0,
							layer_count: vk::REMAINING_ARRAY_LAYERS,
						})
					;

					image_memory_barriers.push(image_memory_barrier);
				}
				Handle::Buffer(handle) => {
					let buffer = self.get_buffer(handle);

					if buffer.buffer.is_null() { continue; }

					let buffer_memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::BufferMemoryBarrier2::default().src_stage_mask(source.stage).src_access_mask(source.access).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					} else {
						vk::BufferMemoryBarrier2::default().src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty()).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask)
					.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
					.buffer(buffer.buffer)
					.offset(0)
					.size(vk::WHOLE_SIZE);

					buffer_memory_barriers.push(buffer_memory_barrier);
				},
				Handle::TopLevelAccelerationStructure(_) | Handle::BottomLevelAccelerationStructure(_)=> {
					let memory_barrier = if let Some(source) = self.states.get(&consumption.handle) {
						vk::MemoryBarrier2::default().src_stage_mask(source.stage).src_access_mask(source.access)
					} else {
						vk::MemoryBarrier2::default().src_stage_mask(vk::PipelineStageFlags2::empty()).src_access_mask(vk::AccessFlags2KHR::empty())
					}
					.dst_stage_mask(new_stage_mask)
					.dst_access_mask(new_access_mask);

					memory_barriers.push(memory_barrier);
				}
			};

			// Update current resource state, AFTER generating the barrier.
			self.states.insert(consumption.handle, transition_state);
		}

		if image_memory_barriers.is_empty() && buffer_memory_barriers.is_empty() && memory_barriers.is_empty() { return; } // consumptions may have had some elements but they may have been skipped.

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.buffer_memory_barriers(&buffer_memory_barriers)
			.memory_barriers(&memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION);

		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.cmd_pipeline_barrier2(command_buffer.command_buffer, &dependency_info) };
	}

	fn get_internal_buffer_handle(&self, handle: graphics_hardware_interface::BaseBufferHandle) -> BufferHandle {
		let mut i = 0;
		let mut internal_buffer_handle = BufferHandle(handle.0);
		loop {
			let buffer = &self.ghi.buffers[internal_buffer_handle.0 as usize];
			if i == self.sequence_index || buffer.next.is_none() {
				return internal_buffer_handle;
			}
			internal_buffer_handle = buffer.next.unwrap();
			i += 1;
		}
	}

	fn get_internal_handle(&self, handle: graphics_hardware_interface::Handle) -> Handle {
		match handle {
			graphics_hardware_interface::Handle::Image(handle) => Handle::Image(self.get_internal_image_handle(handle)),
			graphics_hardware_interface::Handle::Buffer(handle) => Handle::Buffer(self.get_internal_buffer_handle(handle)),
			graphics_hardware_interface::Handle::TopLevelAccelerationStructure(handle) => Handle::TopLevelAccelerationStructure(self.get_internal_top_level_acceleration_structure_handle(handle)),
			graphics_hardware_interface::Handle::BottomLevelAccelerationStructure(handle) => Handle::BottomLevelAccelerationStructure(self.get_internal_bottom_level_acceleration_structure_handle(handle)),
			_ => unimplemented!(),
		}
	}
}

impl graphics_hardware_interface::CommandBufferRecordable for CommandBufferRecording<'_> {
	fn begin(&mut self) {
		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.reset_command_pool(command_buffer.command_pool, vk::CommandPoolResetFlags::empty()).expect("No command pool reset") };

		let command_buffer_begin_info = vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT);

		unsafe { self.ghi.device.begin_command_buffer(command_buffer.command_buffer, &command_buffer_begin_info).expect("No command buffer begin") };
	}

	fn sync_buffers(&mut self) {
		let copy_buffers = self.buffer_copies.drain(..).collect::<Vec<_>>();

		unsafe {
			self.vulkan_consume_resources(&copy_buffers.iter().map(|e| {
				VulkanConsumption {
					handle: Handle::Buffer(e.dst_buffer),
					stages: vk::PipelineStageFlags2::COPY,
					access: vk::AccessFlags2::TRANSFER_WRITE,
					layout: vk::ImageLayout::UNDEFINED,
				}
			}).collect::<Vec<_>>());
		}

		for e in copy_buffers { // Copy all staging buffers to their respective buffers
			let src_buffer = self.get_buffer(e.src_buffer);
			let dst_buffer = self.get_buffer(e.dst_buffer);

			let src_vk_buffer = src_buffer.buffer;
			let dst_vk_buffer = dst_buffer.buffer;

			let command_buffer = self.get_command_buffer();

			let regions = [vk::BufferCopy2KHR::default()
				.src_offset(e.src_offset)
				.dst_offset(e.dst_offset)
				.size(e.size as u64)
			];

			let copy_buffer_info = vk::CopyBufferInfo2KHR::default()
				.src_buffer(src_vk_buffer)
				.dst_buffer(dst_vk_buffer)
				.regions(&regions)
			;

			unsafe {
				self.ghi.device.cmd_copy_buffer2(command_buffer.command_buffer, &copy_buffer_info);
			}
		}

		self.stages |= vk::PipelineStageFlags2::TRANSFER;
	}

	fn sync_textures(&mut self,) {
		let image_handles = {
			let mut dirty_images = self.ghi.image_writes_queue.lock();

			dirty_images.retain(|_, v| *v < self.ghi.frames as u32);
			dirty_images.iter_mut().for_each(|(_, f)| *f += 1);

			dirty_images.keys().map(|&b| self.get_internal_image_handle(b)).filter(|b| { let b = self.get_image(*b); b.staging_buffer.is_some() && b.size != 0 }).collect::<Vec<_>>()
		};

		unsafe { self.vulkan_consume_resources(&image_handles.iter().map(|image_handle|
			VulkanConsumption {
				handle: Handle::Image(*image_handle),
				stages: vk::PipelineStageFlags2::TRANSFER,
				access: vk::AccessFlags2::TRANSFER_WRITE,
				layout: vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			}
		).collect::<Vec<_>>()) };

		let command_buffer = self.get_command_buffer();

		for image_handle in &image_handles {
			let image = self.get_image(*image_handle);

			let regions = [vk::BufferImageCopy2::default()
				.buffer_offset(0)
				.buffer_row_length(0)
				.buffer_image_height(0)
				.image_subresource(vk::ImageSubresourceLayers::default()
					.aspect_mask(vk::ImageAspectFlags::COLOR)
					.mip_level(0)
					.base_array_layer(0)
					.layer_count(1)
				)
				.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
				.image_extent(vk::Extent3D::default().width(image.extent.width).height(image.extent.height).depth(image.extent.depth))];

			let buffer = self.get_buffer(image.staging_buffer.expect("No staging buffer"));

			// Copy to images from staging buffer
			let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
				.src_buffer(buffer.buffer)
				.dst_image(image.image)
				.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.regions(&regions);

			unsafe {
				self.ghi.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
			}
		}

		// TODO: bad. remove this.
		unsafe { self.vulkan_consume_resources(&image_handles.iter().map(|image_handle|
			VulkanConsumption {
				handle: Handle::Image(*image_handle),
				stages: vk::PipelineStageFlags2::FRAGMENT_SHADER,
				access: vk::AccessFlags2::SHADER_SAMPLED_READ,
				layout: vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
			}
		).collect::<Vec<_>>()) };

		self.stages |= vk::PipelineStageFlags2::TRANSFER;
	}

	fn transfer_textures(&mut self, image_handles: &[graphics_hardware_interface::ImageHandle]) -> Vec<graphics_hardware_interface::TextureCopyHandle> {
		unsafe {
			let buffer_handles = image_handles.iter().filter_map(|image_handle| self.get_image(self.get_internal_image_handle(*image_handle)).staging_buffer).collect::<Vec<_>>();

			self.consume_resources(&image_handles.iter().map(|image_handle| Consumption {
				handle: Handle::Image(self.get_internal_image_handle(*image_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}).chain(buffer_handles.iter().map(|buffer_handle| Consumption {
				handle: Handle::Buffer(*buffer_handle),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			})).collect::<Vec<_>>());
		};

		let command_buffer = self.get_command_buffer();
		let command_buffer = command_buffer.command_buffer;

		for image_handle in image_handles {
			let image = self.get_image(self.get_internal_image_handle(*image_handle));
			// If texture has an associated staging_buffer_handle, copy texture data to staging buffer
			if let Some(staging_buffer_handle) = image.staging_buffer {
				let staging_buffer = self.get_buffer(staging_buffer_handle);

				let regions = [vk::BufferImageCopy2KHR::default()
					.buffer_offset(0)
					.buffer_row_length(0)
					.buffer_image_height(0)
					.image_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
					.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
					.image_extent(image.extent)
				];

				let copy_image_to_buffer_info = vk::CopyImageToBufferInfo2KHR::default()
					.src_image(image.image)
					.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
					.dst_buffer(staging_buffer.buffer)
					.regions(&regions)
				;

				unsafe {
					self.ghi.device.cmd_copy_image_to_buffer2(command_buffer, &copy_image_to_buffer_info);
				}
			}
		}

		let mut texture_copies = Vec::new();

		for image_handle in image_handles {
			let internal_image_handle = self.get_internal_image_handle(*image_handle);
			let image = self.get_image(internal_image_handle);
			if let Some(_) = image.staging_buffer {
				texture_copies.push(graphics_hardware_interface::TextureCopyHandle(internal_image_handle.0));
			}
		}

		texture_copies
	}

	fn start_render_pass(&mut self, extent: Extent, attachments: &[graphics_hardware_interface::AttachmentInformation]) -> &mut impl graphics_hardware_interface::RasterizationRenderPassMode {
		unsafe {
			self.consume_resources(&attachments.iter().map(|attachment|
				Consumption{
					handle: Handle::Image(self.get_internal_image_handle(attachment.image)),
					stages: graphics_hardware_interface::Stages::FRAGMENT,
					access: graphics_hardware_interface::AccessPolicies::WRITE,
					layout: attachment.layout,
				}
			).collect::<Vec<_>>());
		}

		let render_area = vk::Rect2D::default().offset(vk::Offset2D::default().x(0).y(0)).extent(vk::Extent2D::default().width(extent.width()).height(extent.height()));

		let color_attchments = attachments.iter().filter(|a| a.format != graphics_hardware_interface::Formats::Depth32).map(|attachment| {
			let image = self.get_image(self.get_internal_image_handle(attachment.image));
			vk::RenderingAttachmentInfo::default()
				.image_view(if let Some(index) = attachment.layer { image.image_views[index as usize] } else { image.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
		}).collect::<Vec<_>>();

		let depth_attachment = attachments.iter().find(|attachment| attachment.format == graphics_hardware_interface::Formats::Depth32).map(|attachment| {
			let image = self.get_image(self.get_internal_image_handle(attachment.image));
			vk::RenderingAttachmentInfo::default()
				.image_view(if let Some(index) = attachment.layer { image.image_views[index as usize] } else { image.image_view })
				.image_layout(texture_format_and_resource_use_to_image_layout(attachment.format, attachment.layout, None))
				.load_op(to_load_operation(attachment.load))
				.store_op(to_store_operation(attachment.store))
				.clear_value(to_clear_value(attachment.clear))
		}).or(Some(vk::RenderingAttachmentInfo::default())).unwrap();

		let rendering_info = vk::RenderingInfoKHR::default().color_attachments(color_attchments.as_slice()).depth_attachment(&depth_attachment).render_area(render_area).layer_count(1);

		let viewports = [
			vk::Viewport {
				x: 0.0,
				y: (extent.height() as f32),
				width: extent.width() as f32,
				height: -(extent.height() as f32),
				min_depth: 0.0,
				max_depth: 1.0,
			}
		];

		let command_buffer = self.get_command_buffer();

		unsafe { self.ghi.device.cmd_set_scissor(command_buffer.command_buffer, 0, &[render_area]); }
		unsafe { self.ghi.device.cmd_set_viewport(command_buffer.command_buffer, 0, &viewports); }
		unsafe { self.ghi.device.cmd_begin_rendering(command_buffer.command_buffer, &rendering_info); }

		self.in_render_pass = true;

		self
	}

	fn build_top_level_acceleration_structure(&mut self, acceleration_structure_build: &graphics_hardware_interface::TopLevelAccelerationStructureBuild) {
		use graphics_hardware_interface::Device;

		let (acceleration_structure_handle, acceleration_structure) = self.get_top_level_acceleration_structure(acceleration_structure_build.acceleration_structure);

		let (as_geometries, offsets) = match acceleration_structure_build.description {
			graphics_hardware_interface::TopLevelAccelerationStructureBuildDescriptions::Instance { instances_buffer, instance_count } => {
				(vec![
					vk::AccelerationStructureGeometryKHR::default()
						.geometry_type(vk::GeometryTypeKHR::INSTANCES)
						.geometry(vk::AccelerationStructureGeometryDataKHR{ instances: vk::AccelerationStructureGeometryInstancesDataKHR::default()
							.array_of_pointers(false)
							.data(vk::DeviceOrHostAddressConstKHR { device_address: self.ghi.get_buffer_address(instances_buffer), })
						})
						.flags(vk::GeometryFlagsKHR::OPAQUE)
				], vec![
					vk::AccelerationStructureBuildRangeInfoKHR::default()
						.primitive_count(instance_count)
						.primitive_offset(0)
						.first_vertex(0)
						.transform_offset(0)
				])
			}
		};

		let scratch_buffer_address = unsafe {
			let buffer = self.get_buffer(self.get_internal_buffer_handle(acceleration_structure_build.scratch_buffer.buffer));
			self.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + acceleration_structure_build.scratch_buffer.offset as u64
		};

		let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
			.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
			.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
			.ty(vk::AccelerationStructureTypeKHR::TOP_LEVEL)
			.dst_acceleration_structure(acceleration_structure.acceleration_structure)
			.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch_buffer_address, })
		;

		self.states.insert(Handle::TopLevelAccelerationStructure(self.get_internal_top_level_acceleration_structure_handle(acceleration_structure_handle)), TransitionState {
			stage: vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
			access: vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
			layout: vk::ImageLayout::UNDEFINED,
		});

		let infos = vec![build_geometry_info];
		let build_range_infos = vec![offsets];
		let geometries = vec![as_geometries];

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let infos = infos.iter().zip(geometries.iter()).map(|(info, geos)| info.geometries(geos)).collect::<Vec<_>>();

		let build_range_infos = build_range_infos.iter().map(|build_range_info| build_range_info.as_slice()).collect::<Vec<_>>();

		self.stages |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR;

		unsafe {
			self.ghi.acceleration_structure.cmd_build_acceleration_structures(vk_command_buffer, &infos, &build_range_infos)
		}
	}

	fn build_bottom_level_acceleration_structures(&mut self, acceleration_structure_builds: &[graphics_hardware_interface::BottomLevelAccelerationStructureBuild]) {
		if acceleration_structure_builds.is_empty() { return; }

		fn visit(this: &mut CommandBufferRecording, acceleration_structure_builds: &[graphics_hardware_interface::BottomLevelAccelerationStructureBuild], mut infos: Vec<vk::AccelerationStructureBuildGeometryInfoKHR>, mut geometries: Vec<Vec<vk::AccelerationStructureGeometryKHR>>, mut build_range_infos: Vec<Vec<vk::AccelerationStructureBuildRangeInfoKHR>>,) {
			if let Some(build) = acceleration_structure_builds.first() {
				let (acceleration_structure_handle, acceleration_structure) = this.get_bottom_level_acceleration_structure(build.acceleration_structure);

				let (as_geometries, offsets) = match &build.description {
					graphics_hardware_interface::BottomLevelAccelerationStructureBuildDescriptions::AABB { .. } => {
						(vec![], vec![])
					}
					graphics_hardware_interface::BottomLevelAccelerationStructureBuildDescriptions::Mesh { vertex_buffer, index_buffer, vertex_position_encoding, index_format, triangle_count, vertex_count } => {
						let vertex_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(vertex_buffer.buffer_offset.buffer));
							this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + vertex_buffer.buffer_offset.offset as u64
						};

						let index_data_address = unsafe {
							let buffer = this.get_buffer(this.get_internal_buffer_handle(index_buffer.buffer_offset.buffer));
							this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + index_buffer.buffer_offset.offset as u64
						};

						let triangles = vk::AccelerationStructureGeometryTrianglesDataKHR::default()
							.vertex_data(vk::DeviceOrHostAddressConstKHR { device_address: vertex_data_address, })
							.index_data(vk::DeviceOrHostAddressConstKHR { device_address: index_data_address, })
							.max_vertex(vertex_count - 1)
							.vertex_format(match vertex_position_encoding {
								graphics_hardware_interface::Encodings::FloatingPoint => vk::Format::R32G32B32_SFLOAT,
								_ => panic!("Invalid vertex position encoding"),
							})
							.index_type(match index_format {
								graphics_hardware_interface::DataTypes::U8 => vk::IndexType::UINT8_EXT,
								graphics_hardware_interface::DataTypes::U16 => vk::IndexType::UINT16,
								graphics_hardware_interface::DataTypes::U32 => vk::IndexType::UINT32,
								_ => panic!("Invalid index format"),
							})
							.vertex_stride(vertex_buffer.stride as vk::DeviceSize);

						let build_range_info = vec![vk::AccelerationStructureBuildRangeInfoKHR::default()
							.primitive_count(*triangle_count)
							.primitive_offset(0)
							.first_vertex(0)
							.transform_offset(0)
						];

						(vec![vk::AccelerationStructureGeometryKHR::default()
							.flags(vk::GeometryFlagsKHR::OPAQUE)
							.geometry_type(vk::GeometryTypeKHR::TRIANGLES)
							.geometry(vk::AccelerationStructureGeometryDataKHR{ triangles })],
						build_range_info)
					}
				};

				let scratch_buffer_address = unsafe {
					let buffer = this.get_buffer(this.get_internal_buffer_handle(build.scratch_buffer.buffer));
					this.ghi.device.get_buffer_device_address(&vk::BufferDeviceAddressInfo::default().buffer(buffer.buffer)) + build.scratch_buffer.offset as u64
				};

				let build_geometry_info = vk::AccelerationStructureBuildGeometryInfoKHR::default()
					.flags(vk::BuildAccelerationStructureFlagsKHR::PREFER_FAST_TRACE)
					.mode(vk::BuildAccelerationStructureModeKHR::BUILD)
					.ty(vk::AccelerationStructureTypeKHR::BOTTOM_LEVEL)
					.dst_acceleration_structure(acceleration_structure.acceleration_structure)
					.scratch_data(vk::DeviceOrHostAddressKHR { device_address: scratch_buffer_address, })
				;

				this.states.insert(Handle::BottomLevelAccelerationStructure(this.get_internal_bottom_level_acceleration_structure_handle(acceleration_structure_handle)), TransitionState {
					stage: vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR,
					access: vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR,
					layout: vk::ImageLayout::UNDEFINED,
				});

				infos.push(build_geometry_info);
				build_range_infos.push(offsets);
				geometries.push(as_geometries);

				visit(this, &acceleration_structure_builds[1..], infos, geometries, build_range_infos,);
			} else {
				let command_buffer = this.get_command_buffer();

				let infos = infos.iter().zip(geometries.iter()).map(|(info, geos)| info.geometries(geos)).collect::<Vec<_>>();

				let build_range_infos = build_range_infos.iter().map(|build_range_info| build_range_info.as_slice()).collect::<Vec<_>>();

				unsafe {
					this.ghi.acceleration_structure.cmd_build_acceleration_structures(command_buffer.command_buffer, &infos, &build_range_infos)
				}
			}
		}

		visit(self, acceleration_structure_builds, Vec::new(), Vec::new(), Vec::new(),);

		self.stages |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR;
	}

	fn bind_shader(&self, _: graphics_hardware_interface::ShaderHandle) {
		panic!("Not implemented");
	}

	fn bind_compute_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundComputePipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::COMPUTE, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::COMPUTE;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn bind_ray_tracing_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundRayTracingPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::RAY_TRACING_KHR, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::RAY_TRACING_KHR;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn blit_image(&mut self, source_image: crate::ImageHandle, source_layout: crate::Layouts, destination_image: crate::ImageHandle, destination_layout: crate::Layouts) {
		unsafe {
			self.consume_resources(&[
				Consumption {
					handle: Handle::Image(self.get_internal_image_handle(source_image)),
					stages: graphics_hardware_interface::Stages::TRANSFER,
					access: graphics_hardware_interface::AccessPolicies::READ,
					layout: source_layout,
				},
				Consumption {
					handle: Handle::Image(self.get_internal_image_handle(destination_image)),
					stages: graphics_hardware_interface::Stages::TRANSFER,
					access: graphics_hardware_interface::AccessPolicies::WRITE,
					layout: destination_layout,
				}
			]);
		}

		let command_buffer = self.get_command_buffer();
		let source_image = self.get_image(self.get_internal_image_handle(source_image));
		let destination_image = self.get_image(self.get_internal_image_handle(destination_image));
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
				vk::Offset3D { x: source_image.extent.width as i32, y: source_image.extent.height as i32, z: 1 },
			])
			.dst_subresource(vk::ImageSubresourceLayers {
				aspect_mask: vk::ImageAspectFlags::COLOR,
				mip_level: 0,
				base_array_layer: 0,
				layer_count: 1,
			})
			.dst_offsets([
				vk::Offset3D { x: 0, y: 0, z: 0 },
				vk::Offset3D { x: destination_image.extent.width as i32, y: destination_image.extent.height as i32, z: 1 },
			]);

			let blits = [blit];

			let blit_info = vk::BlitImageInfo2::default()
				.src_image(source_image.image).src_image_layout(texture_format_and_resource_use_to_image_layout(source_image.format_, source_layout, Some(crate::AccessPolicies::READ)))
				.dst_image(destination_image.image).dst_image_layout(texture_format_and_resource_use_to_image_layout(destination_image.format_, destination_layout, Some(crate::AccessPolicies::WRITE)))
				.regions(&blits)
				.filter(vk::Filter::LINEAR);
			self.ghi.device.cmd_blit_image2(command_buffer.command_buffer, &blit_info);
		}
	}

	fn write_to_push_constant(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, offset: u32, data: &[u8]) {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout;
		unsafe { self.ghi.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE, offset, data); }
	}

	fn write_push_constant<T: Copy + 'static>(&mut self, pipeline_layout_handle: &crate::PipelineLayoutHandle, offset: u32, data: T) where [(); std::mem::size_of::<T>()]: Sized {
		let command_buffer = self.get_command_buffer();
		let pipeline_layout = self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize].pipeline_layout;
		unsafe { self.ghi.device.cmd_push_constants(command_buffer.command_buffer, pipeline_layout, vk::ShaderStageFlags::VERTEX | vk::ShaderStageFlags::MESH_EXT | vk::ShaderStageFlags::FRAGMENT | vk::ShaderStageFlags::COMPUTE, offset, std::slice::from_raw_parts(&data as *const T as *const u8, std::mem::size_of::<T>())); }
	}

	fn clear_images(&mut self, textures: &[(graphics_hardware_interface::ImageHandle, graphics_hardware_interface::ClearValue)]) {
		unsafe { self.consume_resources(textures.iter().map(|(image_handle, _)| Consumption {
			handle: Handle::Image(self.get_internal_image_handle(*image_handle)),
			stages: graphics_hardware_interface::Stages::TRANSFER,
			access: graphics_hardware_interface::AccessPolicies::WRITE,
			layout: graphics_hardware_interface::Layouts::Transfer,
		}).collect::<Vec<_>>().as_slice()) };

		for (image_handle, clear_value) in textures {
			let image = self.get_image(self.get_internal_image_handle(*image_handle));

			if image.image.is_null() { continue; } // Skip unset textures

			if image.format_ != graphics_hardware_interface::Formats::Depth32 {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearColorValue{ float32: [0.0, 0.0, 0.0, 0.0] },
					graphics_hardware_interface::ClearValue::Color(color) => vk::ClearColorValue{ float32: [color.r, color.g, color.b, color.a] },
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearColorValue{ float32: [*depth, 0.0, 0.0, 0.0] },
					graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearColorValue{ uint32: [*r, *g, *b, *a] },
				};

				unsafe {
					self.ghi.device.cmd_clear_color_image(self.get_command_buffer().command_buffer, image.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear_value, &[vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::COLOR,
						base_mip_level: 0,
						level_count: vk::REMAINING_MIP_LEVELS,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					}]);
				}
			} else {
				let clear_value = match clear_value {
					graphics_hardware_interface::ClearValue::None => vk::ClearDepthStencilValue{ depth: 0.0, stencil: 0 },
					graphics_hardware_interface::ClearValue::Color(_) => panic!("Color clear value for depth texture"),
					graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearDepthStencilValue{ depth: *depth, stencil: 0 },
					graphics_hardware_interface::ClearValue::Integer(_, _, _, _) => panic!("Integer clear value for depth texture"),
				};

				unsafe {
					self.ghi.device.cmd_clear_depth_stencil_image(self.get_command_buffer().command_buffer, image.image, vk::ImageLayout::TRANSFER_DST_OPTIMAL, &clear_value, &[vk::ImageSubresourceRange {
						aspect_mask: vk::ImageAspectFlags::DEPTH,
						base_mip_level: 0,
						level_count: vk::REMAINING_MIP_LEVELS,
						base_array_layer: 0,
						layer_count: vk::REMAINING_ARRAY_LAYERS,
					}]);
				}
			}
		}
	}

	unsafe fn consume_resources(&mut self, consumptions: &[graphics_hardware_interface::Consumption]) {
		let consumptions = consumptions.iter().map(|c| {
			Consumption {
				access: c.access,
				handle: self.get_internal_handle(c.handle.clone()),
				stages: c.stages,
				layout: c.layout,
			}
		}).collect::<Vec<_>>();

		self.consume_resources(consumptions.as_slice());
	}

	fn clear_buffers(&mut self, buffer_handles: &[graphics_hardware_interface::BaseBufferHandle]) {
		unsafe { self.consume_resources(&buffer_handles.iter().map(|buffer_handle|
			Consumption{
				handle: Handle::Buffer(self.get_internal_buffer_handle(*buffer_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}
		).collect::<Vec<_>>()) };

		for buffer_handle in buffer_handles {
			let internal_buffer_handle = self.get_internal_buffer_handle(*buffer_handle);
			let buffer = self.get_buffer(internal_buffer_handle);

			if buffer.buffer.is_null() { continue; }

			unsafe {
				self.ghi.device.cmd_fill_buffer(self.get_command_buffer().command_buffer, buffer.buffer, 0, vk::WHOLE_SIZE, 0);
			}

			self.states.insert(Handle::Buffer(internal_buffer_handle), TransitionState {
				stage: vk::PipelineStageFlags2::TRANSFER,
				access: vk::AccessFlags2::TRANSFER_WRITE,
				layout: vk::ImageLayout::UNDEFINED,
			});
		}
	}

	fn write_image_data(&mut self, image_handle: graphics_hardware_interface::ImageHandle, data: &[graphics_hardware_interface::RGBAu8]) {
		let internal_image_handle = self.get_internal_image_handle(image_handle);

		unsafe { self.consume_resources(
			&[Consumption{
				handle: Handle::Image(self.get_internal_image_handle(image_handle)),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::WRITE,
				layout: graphics_hardware_interface::Layouts::Transfer,
			}]
		) };

		let texture = self.get_image(internal_image_handle);

		let staging_buffer_handle = texture.staging_buffer.expect("No staging buffer");

		let buffer = &self.ghi.buffers[staging_buffer_handle.0 as usize];

		let pointer = buffer.pointer;

		let subresource_layout = self.ghi.get_image_subresource_layout(&image_handle, 0);

		if pointer.is_null() {
			for i in data.len()..texture.extent.width as usize * texture.extent.height as usize * texture.extent.depth as usize {
				unsafe {
					std::ptr::write(pointer.offset(i as isize), if i % 4 == 0 { 255 } else { 0 });
				}
			}
		} else {
			let pointer = unsafe { pointer.offset(subresource_layout.offset as isize) };

			for i in 0..texture.extent.height {
				let pointer = unsafe { pointer.offset(subresource_layout.row_pitch as isize * i as isize) };

				unsafe {
					std::ptr::copy_nonoverlapping((data.as_ptr().add(i as usize * texture.extent.width as usize)) as *mut u8, pointer, texture.extent.width as usize * 4);
				}
			}
		}

		let regions = [vk::BufferImageCopy2::default()
			.buffer_offset(0)
			.buffer_row_length(0)
			.buffer_image_height(0)
			.image_subresource(vk::ImageSubresourceLayers::default()
				.aspect_mask(vk::ImageAspectFlags::COLOR)
				.mip_level(0)
				.base_array_layer(0)
				.layer_count(1)
			)
			.image_offset(vk::Offset3D::default().x(0).y(0).z(0))
			.image_extent(vk::Extent3D::default().width(texture.extent.width).height(texture.extent.height).depth(texture.extent.depth))];

		// Copy to images from staging buffer
		let buffer_image_copy = vk::CopyBufferToImageInfo2::default()
			.src_buffer(buffer.buffer)
			.dst_image(texture.image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&regions);

		let command_buffer = self.get_command_buffer();

		unsafe {
			self.ghi.device.cmd_copy_buffer_to_image2(command_buffer.command_buffer, &buffer_image_copy);
		}

		unsafe { self.consume_resources(
			&[Consumption{
				handle: Handle::Image(internal_image_handle),
				stages: graphics_hardware_interface::Stages::FRAGMENT,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Read,
			}]
		) };
	}

	fn copy_to_swapchain(&mut self, source_image_handle: graphics_hardware_interface::ImageHandle, present_key: graphics_hardware_interface::PresentKey, swapchain_handle: graphics_hardware_interface::SwapchainHandle) {
		let source_image_internal_handle = self.get_internal_image_handle(source_image_handle);

		unsafe { self.consume_resources(&[
			Consumption {
				handle: Handle::Image(source_image_internal_handle),
				stages: graphics_hardware_interface::Stages::TRANSFER,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Transfer,
			},
		]) };

		let source_texture = self.get_image(source_image_internal_handle);
		let swapchain = &self.ghi.swapchains[swapchain_handle.0 as usize];

		let swapchain_images = unsafe {
			self.ghi.swapchain.get_swapchain_images(swapchain.swapchain).expect("No swapchain images found.")
		};

		let swapchain_image = swapchain_images[present_key.image_index as usize];

		// Transition source texture to transfer read layout and swapchain image to transfer write layout

		let vk_command_buffer = self.get_command_buffer().command_buffer;

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::UNDEFINED)
				.src_stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE) // This is needed to correctly synchronize presentation when submitting the command buffer.
				.src_access_mask(vk::AccessFlags2KHR::NONE)
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.dst_stage_mask(vk::PipelineStageFlags2::BLIT)
				.dst_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(swapchain_image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				}),
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
		;

		unsafe {
			self.ghi.device.cmd_pipeline_barrier2(vk_command_buffer, &dependency_info);
		}

		// Copy texture to swapchain image

		let image_blits = [vk::ImageBlit2::default()
			.src_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
			.src_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1),
			])
			.dst_subresource(vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).mip_level(0).base_array_layer(0).layer_count(1))
			.dst_offsets([
				vk::Offset3D::default().x(0).y(0).z(0),
				vk::Offset3D::default().x(source_texture.extent.width as i32).y(source_texture.extent.height as i32).z(1),
			])
		];

		let copy_image_info = vk::BlitImageInfo2::default()
			.src_image(source_texture.image)
			.src_image_layout(vk::ImageLayout::TRANSFER_SRC_OPTIMAL)
			.dst_image(swapchain_image)
			.dst_image_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
			.regions(&image_blits)
		;

		self.stages |= vk::PipelineStageFlags2::BLIT;

		unsafe { self.ghi.device.cmd_blit_image2(vk_command_buffer, &copy_image_info); }

		// Transition swapchain image to present layout

		let image_memory_barriers = [
			vk::ImageMemoryBarrier2KHR::default()
				.old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
				.src_stage_mask(vk::PipelineStageFlags2::BLIT)
				.src_access_mask(vk::AccessFlags2KHR::TRANSFER_WRITE)
				.src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
				.dst_stage_mask(vk::PipelineStageFlags2::BOTTOM_OF_PIPE)
				.dst_access_mask(vk::AccessFlags2KHR::NONE)
				.dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
				.image(swapchain_image)
				.subresource_range(vk::ImageSubresourceRange {
					aspect_mask: vk::ImageAspectFlags::COLOR,
					base_mip_level: 0,
					level_count: vk::REMAINING_MIP_LEVELS,
					base_array_layer: 0,
					layer_count: vk::REMAINING_ARRAY_LAYERS,
				})
		];

		let dependency_info = vk::DependencyInfo::default()
			.image_memory_barriers(&image_memory_barriers)
			.dependency_flags(vk::DependencyFlags::BY_REGION)
		;

		unsafe {
			self.ghi.device.cmd_pipeline_barrier2(vk_command_buffer, &dependency_info);
		}

		self.stages |= vk::PipelineStageFlags2::BOTTOM_OF_PIPE; // This is needed to correctly synchronize presentation when submitting the command buffer.
	}

	fn end(&mut self) {
		let command_buffer = self.get_command_buffer();

		if self.in_render_pass {
			unsafe {
				self.ghi.device.cmd_end_render_pass(command_buffer.command_buffer);
			}
		}

		unsafe {
			self.ghi.device.end_command_buffer(command_buffer.command_buffer).expect("Failed to end command buffer.");
		}
	}

	fn bind_descriptor_sets(&mut self, pipeline_layout_handle: &graphics_hardware_interface::PipelineLayoutHandle, sets: &[graphics_hardware_interface::DescriptorSetHandle]) -> &mut impl graphics_hardware_interface::CommandBufferRecordable {
		if sets.is_empty() { return self; }

		let pipeline_layout = &self.ghi.pipeline_layouts[pipeline_layout_handle.0 as usize];

		let s = sets.iter().map(|descriptor_set_handle| {
			let internal_descriptor_set_handle = self.get_internal_descriptor_set_handle(*descriptor_set_handle);
			let descriptor_set = self.get_descriptor_set(&internal_descriptor_set_handle);
			let index_in_layout = pipeline_layout.descriptor_set_template_indices.get(&descriptor_set.descriptor_set_layout).unwrap();
			(*index_in_layout, internal_descriptor_set_handle, descriptor_set.descriptor_set)
		}).collect::<Vec<_>>();

		let vulkan_pipeline_layout_handle = pipeline_layout.pipeline_layout;

		for (descriptor_set_index, descriptor_set_handle, _) in s {
			if (descriptor_set_index as usize) < self.bound_descriptor_set_handles.len() {
				self.bound_descriptor_set_handles[descriptor_set_index as usize] = (descriptor_set_index, descriptor_set_handle);
				self.bound_descriptor_set_handles.truncate(descriptor_set_index as usize + 1);
			} else {
				assert_eq!(descriptor_set_index as usize, self.bound_descriptor_set_handles.len());
				self.bound_descriptor_set_handles.push((descriptor_set_index, descriptor_set_handle));
			}
		}

		let command_buffer = self.get_command_buffer();

		let partitions = partition(&self.bound_descriptor_set_handles, |e| e.0 as usize);

		// Always rebind all descriptor sets set by the user as previously bound descriptor sets might have been invalidated by a pipeline layout change
		for (base_index, descriptor_sets) in partitions {
			let base_index = base_index as u32;

			let descriptor_sets = descriptor_sets.iter().map(|(_, descriptor_set)| self.get_descriptor_set(descriptor_set).descriptor_set).collect::<Vec<_>>();

			unsafe {
				for bp in [vk::PipelineBindPoint::GRAPHICS, vk::PipelineBindPoint::COMPUTE] { // TODO: do this for all needed bind points
					self.ghi.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, bp, vulkan_pipeline_layout_handle, base_index, &descriptor_sets, &[]);
				}

				if self.pipeline_bind_point == vk::PipelineBindPoint::RAY_TRACING_KHR {
					self.ghi.device.cmd_bind_descriptor_sets(command_buffer.command_buffer, vk::PipelineBindPoint::RAY_TRACING_KHR, vulkan_pipeline_layout_handle, base_index, &descriptor_sets, &[]);
				}
			}
		}

		self
	}

	fn execute(mut self, wait_for_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle], signal_synchronizer_handles: &[graphics_hardware_interface::SynchronizerHandle], presentations: &[graphics_hardware_interface::PresentKey], execution_synchronizer_handle: graphics_hardware_interface::SynchronizerHandle) {
		self.end();

		let command_buffer = self.get_command_buffer();

		let command_buffers = [command_buffer.command_buffer];

		let command_buffer_infos = [
			vk::CommandBufferSubmitInfo::default().command_buffer(command_buffers[0])
		];

		// TODO: Take actual stage masks

		let wait_semaphores = wait_for_synchronizer_handles.iter().map(|synchronizer| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.get_synchronizer(*synchronizer).semaphore)
				.stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE | vk::PipelineStageFlags2::TRANSFER)
		}).chain(
			presentations.iter().map(|presentation| {
				vk::SemaphoreSubmitInfo::default()
					.semaphore(self.get_synchronizer(self.get_swapchain(presentation.swapchain).synchronizer).semaphore)
					.stage_mask(vk::PipelineStageFlags2::TOP_OF_PIPE)
			})
		).collect::<Vec<_>>();

		let signal_semaphores = signal_synchronizer_handles.iter().map(|synchronizer| {
			vk::SemaphoreSubmitInfo::default()
				.semaphore(self.get_synchronizer(*synchronizer).semaphore)
				.stage_mask(self.stages)
		}).collect::<Vec<_>>();

		let submit_info = vk::SubmitInfo2::default()
			.command_buffer_infos(&command_buffer_infos)
			.wait_semaphore_infos(&wait_semaphores)
			.signal_semaphore_infos(&signal_semaphores)
		;

		let execution_completion_synchronizer = &self.get_synchronizer(execution_synchronizer_handle);

		unsafe { self.ghi.device.queue_submit2(self.ghi.queue, &[submit_info], execution_completion_synchronizer.fence).expect("Failed to submit command buffer."); }

		for (&k, v) in &self.states {
			self.ghi.states.insert(k, *v);
		}

		for presentation in presentations {
			let swapchain = self.get_swapchain(presentation.swapchain);
			let wait_semaphores = signal_semaphores.iter().map(|signal| signal.semaphore).collect::<Vec<_>>();

			let index = presentation.image_index;

			let swapchains = [swapchain.swapchain];
			let image_indices = [presentation.image_index as u32];

			let mut results = [vk::Result::default()];

			let present_fences = [self.get_synchronizer(swapchain.synchronizer).fence];

			let mut present_fence_info = vk::SwapchainPresentFenceInfoEXT::default().fences(&present_fences);

  			let present_info = vk::PresentInfoKHR::default()
     			.push_next(&mut present_fence_info)
				.results(&mut results)
				.swapchains(&swapchains)
				.wait_semaphores(&wait_semaphores)
				.image_indices(&image_indices)
			;

			let _ = unsafe { self.ghi.swapchain.queue_present(self.ghi.queue, &present_info).expect("No present") };

			if !results.iter().all(|result| *result == vk::Result::SUCCESS) {
				dbg!("Some error occurred during presentation");
			}
		}
	}

	fn start_region(&self, name: &str) {
		let command_buffer = self.get_command_buffer();

		let name = std::ffi::CString::new(name).unwrap();

		let marker_info = vk::DebugUtilsLabelEXT::default()
			.label_name(name.as_c_str());

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.ghi.debug_utils {
				debug_utils.cmd_begin_debug_utils_label(command_buffer.command_buffer, &marker_info);
			}
		}
	}

	fn region(&mut self, name: &str, f: impl FnOnce(&mut Self)) {
		self.start_region(name);
		f(self);
		self.end_region();
	}

	fn end_region(&self) {
		let command_buffer = self.get_command_buffer();

		#[cfg(debug_assertions)]
		unsafe {
			if let Some(debug_utils) = &self.ghi.debug_utils {
				debug_utils.cmd_end_debug_utils_label(command_buffer.command_buffer);
			}
		}
	}
}

impl graphics_hardware_interface::RasterizationRenderPassMode for CommandBufferRecording<'_> {
	/// Binds a pipeline to the GPU.
	fn bind_raster_pipeline(&mut self, pipeline_handle: &graphics_hardware_interface::PipelineHandle) -> &mut impl graphics_hardware_interface::BoundRasterizationPipelineMode {
		let command_buffer = self.get_command_buffer();
		let pipeline = self.ghi.pipelines[pipeline_handle.0 as usize].pipeline;
		unsafe { self.ghi.device.cmd_bind_pipeline(command_buffer.command_buffer, vk::PipelineBindPoint::GRAPHICS, pipeline); }

		self.pipeline_bind_point = vk::PipelineBindPoint::GRAPHICS;
		self.bound_pipeline = Some(*pipeline_handle);

		self
	}

	fn bind_vertex_buffers(&mut self, buffer_descriptors: &[graphics_hardware_interface::BufferDescriptor]) {
		let consumptions = buffer_descriptors.iter().map(|buffer_descriptor| {
			VulkanConsumption {
				handle: Handle::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
				stages: vk::PipelineStageFlags2::VERTEX_INPUT,
				access: vk::AccessFlags2::VERTEX_ATTRIBUTE_READ,
				layout: vk::ImageLayout::UNDEFINED,
			}
		}).collect::<Vec<_>>();

		unsafe {
			self.vulkan_consume_resources(&consumptions);
		}

		let command_buffer = self.get_command_buffer();

		let buffers = buffer_descriptors.iter().map(|buffer_descriptor| self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer)).buffer).collect::<Vec<_>>();
		let offsets = buffer_descriptors.iter().map(|buffer_descriptor| buffer_descriptor.offset).collect::<Vec<_>>();

		// TODO: implent slot splitting
		unsafe { self.ghi.device.cmd_bind_vertex_buffers(command_buffer.command_buffer, 0, &buffers, &offsets.iter().map(|&e| e as _).collect::<Vec<_>>()); }
	}

	fn bind_index_buffer(&mut self, buffer_descriptor: &graphics_hardware_interface::BufferDescriptor) {
		unsafe {
			self.vulkan_consume_resources(&[VulkanConsumption {
				handle: Handle::Buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer.into())),
				stages: vk::PipelineStageFlags2::INDEX_INPUT,
				access: vk::AccessFlags2::INDEX_READ,
				layout: vk::ImageLayout::UNDEFINED,
			}]);
		}

		let command_buffer = self.get_command_buffer();

		let buffer = self.get_buffer(self.get_internal_buffer_handle(buffer_descriptor.buffer));

		unsafe { self.ghi.device.cmd_bind_index_buffer(command_buffer.command_buffer, buffer.buffer, buffer_descriptor.offset as _, vk::IndexType::UINT16); }
	}

	/// Ends a render pass on the GPU.
	fn end_render_pass(&mut self) {
		let command_buffer = self.get_command_buffer();
		unsafe { self.ghi.device.cmd_end_rendering(command_buffer.command_buffer); }
		self.in_render_pass = false;
	}
}

impl graphics_hardware_interface::BoundRasterizationPipelineMode for CommandBufferRecording<'_> {
	/// Draws a render system mesh.
	fn draw_mesh(&mut self, mesh_handle: &graphics_hardware_interface::MeshHandle) {
		let command_buffer = self.get_command_buffer();

		let mesh = &self.ghi.meshes[mesh_handle.0 as usize];

		let buffers = [mesh.buffer];
		let offsets = [0];

		let index_data_offset = (mesh.vertex_count * mesh.vertex_size as u32).next_multiple_of(16) as u64;
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe { self.ghi.device.cmd_bind_vertex_buffers(command_buffer_handle, 0, &buffers, &offsets); }
		unsafe { self.ghi.device.cmd_bind_index_buffer(command_buffer_handle, mesh.buffer, index_data_offset, vk::IndexType::UINT16); }

		unsafe { self.ghi.device.cmd_draw_indexed(command_buffer_handle, mesh.index_count, 1, 0, 0, 0); }
	}

	fn dispatch_meshes(&mut self, x: u32, y: u32, z: u32) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		self.stages |= vk::PipelineStageFlags2::MESH_SHADER_EXT;

		unsafe {
			self.ghi.mesh_shading.cmd_draw_mesh_tasks(command_buffer_handle, x, y, z);
		}
	}

	fn draw_indexed(&mut self, index_count: u32, instance_count: u32, first_index: u32, vertex_offset: i32, first_instance: u32) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		unsafe {
			self.ghi.device.cmd_draw_indexed(command_buffer_handle, index_count, instance_count, first_index, vertex_offset, first_instance);
		}
	}
}

impl graphics_hardware_interface::BoundComputePipelineMode for CommandBufferRecording<'_> {
	fn dispatch(&mut self, dispatch: graphics_hardware_interface::DispatchExtent) {
		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		let (x, y, z) = dispatch.get_extent().as_tuple();

		self.consume_resources_current(&[]);

		self.stages |= vk::PipelineStageFlags2::COMPUTE_SHADER;

		unsafe {
			self.ghi.device.cmd_dispatch(command_buffer_handle, x, y, z);
		}
	}

	fn indirect_dispatch<const N: usize>(&mut self, buffer_handle: &graphics_hardware_interface::BufferHandle<[(u32, u32, u32); N]>, entry_index: usize) {
		let buffer = self.ghi.buffers[buffer_handle.0 as usize];

		let command_buffer = self.get_command_buffer();
		let command_buffer_handle = command_buffer.command_buffer;

		self.stages |= vk::PipelineStageFlags2::COMPUTE_SHADER;

		self.consume_resources_current(&[
			graphics_hardware_interface::Consumption{
				handle: graphics_hardware_interface::Handle::Buffer(buffer_handle.clone().into()),
				stages: graphics_hardware_interface::Stages::COMPUTE,
				access: graphics_hardware_interface::AccessPolicies::READ,
				layout: graphics_hardware_interface::Layouts::Indirect,
			}
		]);

		unsafe {
			self.ghi.device.cmd_dispatch_indirect(command_buffer_handle, buffer.buffer, entry_index as u64 * (3 * 4));
		}
	}
}

impl graphics_hardware_interface::BoundRayTracingPipelineMode for CommandBufferRecording<'_> {
	fn trace_rays(&mut self, binding_tables: graphics_hardware_interface::BindingTables, x: u32, y: u32, z: u32) {
		use graphics_hardware_interface::Device;

		let command_buffer = self.get_command_buffer();
		let comamand_buffer_handle = command_buffer.command_buffer;

		let make_strided_range = |range: graphics_hardware_interface::BufferStridedRange| -> vk::StridedDeviceAddressRegionKHR {
			vk::StridedDeviceAddressRegionKHR::default()
				.device_address(self.ghi.get_buffer_address(range.buffer_offset.buffer) as vk::DeviceSize + range.buffer_offset.offset as vk::DeviceSize)
				.stride(range.stride as vk::DeviceSize)
				.size(range.size as vk::DeviceSize)
		};

		let raygen_shader_binding_tables = make_strided_range(binding_tables.raygen);
		let miss_shader_binding_tables = make_strided_range(binding_tables.miss);
		let hit_shader_binding_tables = make_strided_range(binding_tables.hit);
		let callable_shader_binding_tables = if let Some(binding_table) = binding_tables.callable { make_strided_range(binding_table) } else { vk::StridedDeviceAddressRegionKHR::default() };

		self.consume_resources_current(&[]);

		unsafe {
			self.ghi.ray_tracing_pipeline.cmd_trace_rays(comamand_buffer_handle, &raygen_shader_binding_tables, &miss_shader_binding_tables, &hit_shader_binding_tables, &callable_shader_binding_tables, x, y, z)
		}
	}
}

pub(crate) struct BufferCopy {
	pub src_buffer: BufferHandle,
	pub src_offset: vk::DeviceSize,
	pub dst_buffer: BufferHandle,
	pub dst_offset: vk::DeviceSize,
	pub size: usize,
}

impl BufferCopy {
	pub fn new(src_buffer: BufferHandle, src_offset: vk::DeviceSize, dst_buffer: BufferHandle, dst_offset: vk::DeviceSize, size: usize) -> Self {
		Self {
			src_buffer,
			src_offset,
			dst_buffer,
			dst_offset,
			size,
		}
	}
}
