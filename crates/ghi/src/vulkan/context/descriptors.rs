use super::*;

impl Context {
	pub(crate) fn create_descriptor_heap_arena(
		&mut self,
		name: &str,
		size: u64,
		heap_alignment: u64,
		descriptor_alignment: u64,
		reserved_size: u64,
	) -> DescriptorHeapArena {
		assert!(
			size <= u32::MAX as u64,
			"Vulkan descriptor heap exceeds the 32-bit push-index address space. The most likely cause is an implementation reservation larger than the mapping interface supports.",
		);
		let buffer_size = size
			.checked_add(heap_alignment)
			.expect("Vulkan descriptor heap size overflowed. The most likely cause is invalid descriptor-heap properties.");
		let creation = self.create_vulkan_buffer(
			Some(name),
			usize::try_from(buffer_size).expect(
				"Vulkan descriptor heap exceeds addressable host memory. The most likely cause is invalid device limits.",
			),
			vk::BufferUsageFlags::DESCRIPTOR_HEAP_EXT | vk::BufferUsageFlags::SHADER_DEVICE_ADDRESS,
		);
		let host_device_local = self
			.memory_properties
			.memory_types
			.iter()
			.enumerate()
			.take(self.memory_properties.memory_type_count as usize)
			.any(|(index, memory_type)| {
				creation.memory_flags & (1 << index) != 0
					&& memory_type.property_flags.contains(
						vk::MemoryPropertyFlags::HOST_VISIBLE
							| vk::MemoryPropertyFlags::HOST_COHERENT
							| vk::MemoryPropertyFlags::DEVICE_LOCAL,
					)
			});
		let device_accesses = crate::DeviceAccesses::CpuWrite
			| if host_device_local {
				crate::DeviceAccesses::GpuRead
			} else {
				crate::DeviceAccesses::empty()
			};
		let (allocation, _) = self.create_allocation_internal(creation.size, creation.memory_flags.into(), device_accesses);
		let (device_address, pointer) = self.bind_vulkan_buffer_memory(&creation, allocation, 0);
		let aligned_address = crate::vulkan::align_up(device_address, heap_alignment);
		let base_offset = aligned_address - device_address;
		assert!(base_offset + size <= buffer_size);

		let application_offset = crate::vulkan::align_up(reserved_size, descriptor_alignment);
		assert!(
			application_offset < size,
			"Vulkan descriptor heap has no application-owned range. The most likely cause is that the implementation reservation consumes the device's maximum heap size.",
		);
		DescriptorHeapArena {
			buffer: creation.resource,
			pointer: unsafe { pointer.add(base_offset as usize) },
			device_address: aligned_address,
			size,
			reserved_size,
			free_ranges: vec![crate::vulkan::DescriptorHeapRange {
				offset: application_offset,
				size: size - application_offset,
			}],
		}
	}

	/// Allocates the long-lived resource and sampler heaps once for the context.
	pub(crate) fn create_descriptor_heaps(&mut self) -> DescriptorHeaps {
		const RESOURCE_HEAP_TARGET_SIZE: u64 = 64 * 1024 * 1024;
		const SAMPLER_HEAP_TARGET_SIZE: u64 = 1024 * 1024;
		let properties = self.device.descriptor_heap_properties;
		let resource_descriptor_alignment = properties
			.buffer_descriptor_alignment
			.max(properties.image_descriptor_alignment);
		let resource_minimum =
			crate::vulkan::align_up(properties.min_resource_heap_reserved_range, resource_descriptor_alignment)
				.checked_add(properties.buffer_descriptor_size.max(properties.image_descriptor_size))
				.unwrap();
		let sampler_minimum = crate::vulkan::align_up(
			properties.min_sampler_heap_reserved_range,
			properties.sampler_descriptor_alignment,
		)
		.checked_add(properties.sampler_descriptor_size)
		.unwrap();
		assert!(
			resource_minimum <= properties.max_resource_heap_size,
			"Vulkan resource heap limits are inconsistent. The most likely cause is that the implementation reservation leaves no room for one resource descriptor.",
		);
		assert!(
			sampler_minimum <= properties.max_sampler_heap_size,
			"Vulkan sampler heap limits are inconsistent. The most likely cause is that the implementation reservation leaves no room for one sampler descriptor.",
		);
		let resource_size = RESOURCE_HEAP_TARGET_SIZE
			.max(resource_minimum)
			.min(properties.max_resource_heap_size);
		let sampler_size = SAMPLER_HEAP_TARGET_SIZE
			.max(sampler_minimum)
			.min(properties.max_sampler_heap_size);

		DescriptorHeaps {
			resource: self.create_descriptor_heap_arena(
				"GHI Resource Descriptor Heap",
				resource_size,
				properties.resource_heap_alignment,
				resource_descriptor_alignment,
				properties.min_resource_heap_reserved_range,
			),
			sampler: self.create_descriptor_heap_arena(
				"GHI Sampler Descriptor Heap",
				sampler_size,
				properties.sampler_heap_alignment,
				properties.sampler_descriptor_alignment,
				properties.min_sampler_heap_reserved_range,
			),
		}
	}

	pub(crate) fn descriptors_at_slot<'a>(
		&'a self,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		slot: crate::shader::ResourceSlot,
	) -> Option<&'a HashMap<u32, crate::vulkan::descriptor_set::RetainedDescriptor>> {
		sets.iter()
			.find_map(|set| self.descriptor_sets[set.0 as usize].descriptors.get(&slot))
	}

	pub(crate) fn descriptor_matches_kind(
		descriptor: crate::descriptors::WriteData,
		kind: crate::shader::ResourceKind,
	) -> bool {
		match descriptor {
			crate::descriptors::WriteData::Buffer { .. } => matches!(
				kind,
				crate::shader::ResourceKind::UniformBuffer | crate::shader::ResourceKind::StorageBuffer
			),
			crate::descriptors::WriteData::Image { .. } | crate::descriptors::WriteData::Swapchain(_) => matches!(
				kind,
				crate::shader::ResourceKind::SampledImage
					| crate::shader::ResourceKind::StorageImage
					| crate::shader::ResourceKind::InputAttachment
			),
			crate::descriptors::WriteData::CombinedImageSampler { .. } => {
				kind == crate::shader::ResourceKind::CombinedImageSampler
			}
			crate::descriptors::WriteData::Sampler(_) => kind == crate::shader::ResourceKind::Sampler,
			crate::descriptors::WriteData::AccelerationStructure { .. } => {
				kind == crate::shader::ResourceKind::AccelerationStructure
			}
			crate::descriptors::WriteData::StaticSamplers | crate::descriptors::WriteData::CombinedImageSamplerArray => false,
		}
	}

	/// Compares the native resource and image layout consumed by two materialized descriptors.
	pub(crate) fn descriptors_consume_same_resource(left: Descriptor, right: Descriptor) -> bool {
		match (left, right) {
			(Descriptor::Buffer { buffer: left, .. }, Descriptor::Buffer { buffer: right, .. }) => left == right,
			(
				Descriptor::Image {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::Image {
					image: right,
					layout: right_layout,
					..
				},
			)
			| (
				Descriptor::Image {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::CombinedImageSampler {
					image: right,
					layout: right_layout,
					..
				},
			)
			| (
				Descriptor::CombinedImageSampler {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::Image {
					image: right,
					layout: right_layout,
					..
				},
			)
			| (
				Descriptor::CombinedImageSampler {
					image: left,
					layout: left_layout,
					..
				},
				Descriptor::CombinedImageSampler {
					image: right,
					layout: right_layout,
					..
				},
			) => left == right && left_layout == right_layout,
			(Descriptor::AccelerationStructure { handle: left }, Descriptor::AccelerationStructure { handle: right }) => {
				left == right
			}
			_ => false,
		}
	}

	/// Validates one complete logical set union against the active flat pipeline layout.
	pub(crate) fn validate_descriptor_sets(
		&self,
		layout: &PipelineLayout,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
	) {
		for set in sets {
			assert!(
				(set.0 as usize) < self.descriptor_sets.len(),
				"Invalid Vulkan descriptor set. The most likely cause is that a bound handle came from another context.",
			);
		}

		for resource in &layout.resources {
			let descriptor = resource.descriptor;
			for set in sets {
				assert!(
					self.descriptor_sets[set.0 as usize]
						.descriptors
						.keys()
						.all(|slot| crate::vulkan::resource_accepts_retained_slot_key(descriptor, *slot)),
					"Invalid retained Vulkan descriptor slot. The most likely cause is that an array element was written as an interior flat slot instead of using array_element at the array base.",
				);
			}

			let owners = sets
				.iter()
				.filter(|set| {
					self.descriptor_sets[set.0 as usize]
						.descriptors
						.contains_key(&descriptor.slot())
				})
				.count();
			assert!(
				owners <= 1,
				"Overlapping retained Vulkan descriptor sets. The most likely cause is that two bound sets write the same active flat resource slot.",
			);

			let elements = self.descriptors_at_slot(sets, descriptor.slot());
			if descriptor.count() == 1 {
				assert!(
					elements.is_some_and(|elements| elements.contains_key(&0)),
					"Missing retained Vulkan descriptor at resource slot {}. The most likely cause is that a scalar pipeline resource was not written before rendering.",
					descriptor.slot().index(),
				);
			}
			if let Some(elements) = elements {
				for (&array_element, retained) in elements {
					assert!(
						array_element < descriptor.count(),
						"Vulkan descriptor array element is out of range. The most likely cause is that a retained write exceeded the shader resource count.",
					);
					assert!(
						Self::descriptor_matches_kind(retained.descriptor, descriptor.kind()),
						"Vulkan descriptor kind mismatch. The most likely cause is that a retained write does not match the active shader resource interface.",
					);
				}
			}
		}
	}

	pub(crate) fn swapchain_key_for_image(
		&self,
		handle: graphics_hardware_interface::BaseImageHandle,
		sequence_index: u8,
	) -> Option<(graphics_hardware_interface::SwapchainHandle, u8)> {
		self.swapchains.iter().enumerate().find_map(|(index, swapchain)| {
			(swapchain.images[0].0 == handle.0 || swapchain.native_images[0].0 == handle.0).then_some((
				graphics_hardware_interface::SwapchainHandle(index as u64),
				swapchain.acquired_image_indices[sequence_index as usize],
			))
		})
	}

	pub(crate) fn materialization_key(
		&self,
		layout_handle: graphics_hardware_interface::PipelineLayoutHandle,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		sequence_index: u8,
	) -> MaterializationKey {
		let layout = &self.pipeline_layouts[layout_handle.0 as usize];
		let mut descriptor_sets = sets
			.iter()
			.map(|set| {
				let descriptor_set = &self.descriptor_sets[set.0 as usize];
				(
					*set,
					descriptor_set.version,
					descriptor_set.sequence_versions[sequence_index as usize],
				)
			})
			.collect::<SmallVec<[_; 4]>>();
		descriptor_sets.sort_unstable_by_key(|(set, ..)| set.0);
		let mut resource_epochs = SmallVec::<[_; MAX_FRAMES_IN_FLIGHT]>::new();
		let mut swapchain_images = SmallVec::<[_; 4]>::new();
		for resource in &layout.resources {
			let Some(elements) = self.descriptors_at_slot(sets, resource.descriptor.slot()) else {
				continue;
			};
			for retained in elements.values() {
				let target_sequence = self.frame_index_with_offset(sequence_index as usize, retained.frame_offset) as u8;
				let key = match retained.descriptor {
					crate::descriptors::WriteData::Buffer { .. } => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						None
					}
					crate::descriptors::WriteData::Swapchain(handle) => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						Some((
							handle,
							self.swapchains[handle.0 as usize].acquired_image_indices[target_sequence as usize],
						))
					}
					crate::descriptors::WriteData::Image { handle, .. }
					| crate::descriptors::WriteData::CombinedImageSampler {
						image_handle: handle, ..
					} => {
						resource_epochs.push((target_sequence, self.descriptor_sequence_epochs[target_sequence as usize]));
						self.swapchain_key_for_image(handle, target_sequence)
					}
					_ => None,
				};
				if let Some(key) = key {
					swapchain_images.push(key);
				}
			}
		}
		resource_epochs.sort_unstable_by_key(|(sequence, _)| *sequence);
		resource_epochs.dedup();
		swapchain_images.sort_unstable_by_key(|(handle, image)| (handle.0, *image));
		swapchain_images.dedup();

		MaterializationKey {
			layout: layout_handle,
			descriptor_sets,
			sequence_index,
			resource_epochs,
			swapchain_images,
		}
	}

	pub(crate) fn resolve_retained_descriptor(
		&self,
		retained: crate::vulkan::descriptor_set::RetainedDescriptor,
		sequence_index: u8,
	) -> Descriptor {
		let resource_sequence = self.frame_index_with_offset(sequence_index as usize, retained.frame_offset);
		match retained.descriptor {
			crate::descriptors::WriteData::Buffer { handle, size } => Descriptor::Buffer {
				buffer: self.buffers.nth_handle(handle, resource_sequence).expect(
					"Missing deferred Vulkan buffer. The most likely cause is that frame resource tasks were not processed before descriptor materialization.",
				),
				size,
			},
			crate::descriptors::WriteData::Image {
				handle,
				layout,
				mip_level,
			} => Descriptor::Image {
				image: self.resolve_descriptor_image_handle(
					graphics_hardware_interface::ImageHandle(handle),
					sequence_index as usize,
					retained.frame_offset,
				),
				layout,
				mip_level,
			},
			crate::descriptors::WriteData::CombinedImageSampler {
				image_handle,
				sampler_handle,
				layout,
				layer,
			} => Descriptor::CombinedImageSampler {
				image: self.resolve_descriptor_image_handle(
					graphics_hardware_interface::ImageHandle(image_handle),
					sequence_index as usize,
					retained.frame_offset,
				),
				sampler: sampler_handle,
				layout,
				layer,
			},
			crate::descriptors::WriteData::Sampler(sampler) => Descriptor::Sampler { sampler },
			crate::descriptors::WriteData::AccelerationStructure { handle } => Descriptor::AccelerationStructure {
				handle: TopLevelAccelerationStructureHandle(handle.0),
			},
			crate::descriptors::WriteData::Swapchain(handle) => {
				let swapchain = &self.swapchains[handle.0 as usize];
				let image_index = swapchain.acquired_image_indices[resource_sequence] as usize;
				Descriptor::Image {
					image: swapchain.images[image_index],
					layout: crate::Layouts::General,
					mip_level: None,
				}
			}
			crate::descriptors::WriteData::StaticSamplers
			| crate::descriptors::WriteData::CombinedImageSamplerArray => unreachable!(
				"Legacy Vulkan descriptor write reached materialization. The most likely cause is that write validation was bypassed."
			),
		}
	}

	pub(crate) fn descriptor_image_view_create_info(
		&self,
		image: &Image,
		view_type: crate::TextureViewTypes,
		layer: Option<u32>,
		base_mip_level: u32,
		level_count: u32,
	) -> vk::ImageViewCreateInfo<'static> {
		assert!(
			base_mip_level < image.mip_levels,
			"Vulkan image descriptor mip level is out of range. The most likely cause is that the selected mip exceeds the image mip count. mip_level={base_mip_level}, mip_levels={}",
			image.mip_levels,
		);
		assert!(
			level_count > 0 && level_count <= image.mip_levels - base_mip_level,
			"Vulkan image descriptor mip range is invalid. The most likely cause is that the descriptor view exceeds the image mip count. base_mip_level={base_mip_level}, level_count={level_count}, mip_levels={}",
			image.mip_levels,
		);
		let array_layer_count = image.layers.map_or(1, NonZeroU32::get);
		let (vk_view_type, base_array_layer, layer_count) = match view_type {
			crate::TextureViewTypes::Texture2D => {
				assert!(
					layer.is_none() && image.layers.is_none() && image.extent.depth().max(1) == 1,
					"Vulkan 2D descriptor view mismatch. The most likely cause is that a layered or 3D image was written to a Texture2D shader resource."
				);
				(vk::ImageViewType::TYPE_2D, 0, 1)
			}
			crate::TextureViewTypes::Texture2DArray => {
				let layers = image.layers.map(|layers| layers.get()).expect(
					"Vulkan array descriptor view mismatch. The most likely cause is that a non-array image was written to a Texture2DArray shader resource.",
				);
				let base = layer.unwrap_or(0);
				assert!(
					base < layers,
					"Vulkan image layer is out of range. The most likely cause is an invalid combined-image-sampler layer."
				);
				(
					vk::ImageViewType::TYPE_2D_ARRAY,
					base,
					if layer.is_some() { 1 } else { layers },
				)
			}
			crate::TextureViewTypes::TextureCube => {
				assert!(
					layer.is_none() && image.cube_compatible && array_layer_count == 6,
					"Vulkan cubemap descriptor view mismatch. The most likely cause is that the image is not a six-layer cube-compatible image."
				);
				(vk::ImageViewType::CUBE, 0, 6)
			}
			crate::TextureViewTypes::TextureCubeArray => {
				assert!(
					layer.is_none() && image.cube_array_compatible && array_layer_count.is_multiple_of(6),
					"Vulkan cube-array descriptor view mismatch. The most likely cause is that the image is not a cube-array-compatible image."
				);
				(vk::ImageViewType::CUBE_ARRAY, 0, array_layer_count)
			}
			crate::TextureViewTypes::Texture3D => {
				assert!(
					layer.is_none() && image.layers.is_none() && image.extent.depth() > 1,
					"Vulkan 3D descriptor view mismatch. The most likely cause is that a 2D image was written to a Texture3D shader resource."
				);
				(vk::ImageViewType::TYPE_3D, 0, 1)
			}
		};
		vk::ImageViewCreateInfo::default()
			.image(image.image)
			.view_type(vk_view_type)
			.format(image.format)
			.components(vk::ComponentMapping::default())
			.subresource_range(vk::ImageSubresourceRange {
				aspect_mask: if image.format_.is_depth() {
					vk::ImageAspectFlags::DEPTH
				} else {
					vk::ImageAspectFlags::COLOR
				},
				base_mip_level,
				level_count,
				base_array_layer,
				layer_count,
			})
	}

	/// Writes one immutable logical-set union into previously unused descriptor-heap memory and caches it.
	pub(crate) fn materialize_descriptor_sets(
		&mut self,
		layout_handle: graphics_hardware_interface::PipelineLayoutHandle,
		sets: &[graphics_hardware_interface::DescriptorSetHandle],
		sequence_index: u8,
	) -> DescriptorMaterializationHandle {
		let key = self.materialization_key(layout_handle, sets, sequence_index);
		if let Some(handle) = self.materialization_indices.get(&key) {
			return *handle;
		}

		let layout = self.pipeline_layouts[layout_handle.0 as usize].clone();
		self.validate_descriptor_sets(&layout, sets);
		let mut resolved = Vec::<(PipelineResourceDescriptor, u32, Descriptor)>::new();
		for resource in &layout.resources {
			let Some(elements) = self.descriptors_at_slot(sets, resource.descriptor.slot()) else {
				continue;
			};
			let mut elements = elements.iter().collect::<SmallVec<[_; 16]>>();
			elements.sort_unstable_by_key(|(array_element, _)| **array_element);
			for (&array_element, &retained) in elements {
				resolved.push((
					*resource,
					array_element,
					self.resolve_retained_descriptor(retained, sequence_index),
				));
			}
		}

		let properties = self.device.descriptor_heap_properties;
		let resource_alignment = properties
			.buffer_descriptor_alignment
			.max(properties.image_descriptor_alignment);
		let (resource_heap_offset, sampler_heap_offset) = {
			let heaps = self.descriptor_heaps.as_mut().unwrap();
			let resource_offset = (layout.resource_heap_size > 0)
				.then(|| heaps.resource_mut().allocate(layout.resource_heap_size, resource_alignment))
				.unwrap_or(0);
			let sampler_offset = (layout.sampler_heap_size > 0)
				.then(|| {
					heaps
						.sampler_mut()
						.allocate(layout.sampler_heap_size, properties.sampler_descriptor_alignment)
				})
				.unwrap_or(0);
			(resource_offset, sampler_offset)
		};

		let mut address_writes = Vec::<(vk::DescriptorType, vk::DeviceAddressRangeEXT, u32, u64)>::new();
		let mut image_writes = Vec::<(vk::DescriptorType, vk::ImageViewCreateInfo, vk::ImageLayout, u32, u64)>::new();
		let mut sampler_writes = Vec::<(graphics_hardware_interface::SamplerHandle, u32, u64)>::new();
		let mut materialized_resources = SmallVec::<[ResolvedPipelineDescriptor; 128]>::new();
		for (resource, array_element, descriptor) in resolved {
			let resource_offset = resource
				.resource_heap_offset
				.map(|offset| resource_heap_offset + offset + array_element * resource.resource_stride);
			let sampler_offset = resource
				.sampler_heap_offset
				.map(|offset| sampler_heap_offset + offset + array_element * resource.sampler_stride);
			match descriptor {
				Descriptor::Buffer { buffer, size } => {
					let buffer = self.buffers.resource(buffer);
					let size = match size {
						graphics_hardware_interface::Ranges::Whole => buffer.size as u64,
						graphics_hardware_interface::Ranges::Size(size) => size as u64,
					};
					assert!(
						size > 0 && size <= buffer.size as u64,
						"Invalid Vulkan buffer descriptor range. The most likely cause is that a descriptor exceeds its backing buffer."
					);
					address_writes.push((
						crate::vulkan::descriptor_type(resource.descriptor.kind()).unwrap(),
						vk::DeviceAddressRangeEXT::default().address(buffer.device_address).size(size),
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
				Descriptor::Image {
					image,
					layout: image_layout,
					mip_level,
				} => {
					let image = &self.images[image.0 as usize];
					let vk_layout = texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None);
					image_writes.push((
						crate::vulkan::descriptor_type(resource.descriptor.kind()).unwrap(),
						self.descriptor_image_view_create_info(
							image,
							resource.descriptor.texture_view(),
							None,
							mip_level.unwrap_or(0),
							1,
						),
						vk_layout,
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
				Descriptor::CombinedImageSampler {
					image,
					sampler,
					layout: image_layout,
					layer,
				} => {
					let image = &self.images[image.0 as usize];
					let vk_layout = texture_format_and_resource_use_to_image_layout(image.format_, image_layout, None);
					image_writes.push((
						vk::DescriptorType::SAMPLED_IMAGE,
						self.descriptor_image_view_create_info(
							image,
							resource.descriptor.texture_view(),
							layer,
							0,
							image.mip_levels,
						),
						vk_layout,
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
					sampler_writes.push((sampler, sampler_offset.unwrap(), resource.sampler_stride as u64));
				}
				Descriptor::Sampler { sampler } => {
					sampler_writes.push((sampler, sampler_offset.unwrap(), resource.sampler_stride as u64));
				}
				Descriptor::AccelerationStructure { handle } => {
					let acceleration_structure = &self.acceleration_structures[handle.0 as usize];
					let address = unsafe {
						self.acceleration_structure.get_acceleration_structure_device_address(
							&vk::AccelerationStructureDeviceAddressInfoKHR::default()
								.acceleration_structure(acceleration_structure.acceleration_structure),
						)
					};
					address_writes.push((
						vk::DescriptorType::ACCELERATION_STRUCTURE_KHR,
						vk::DeviceAddressRangeEXT::default().address(address).size(0),
						resource_offset.unwrap(),
						resource.resource_stride as u64,
					));
				}
			}
			if !matches!(descriptor, Descriptor::Sampler { .. }) {
				if let Some(existing) = materialized_resources
					.iter_mut()
					.find(|existing| Self::descriptors_consume_same_resource(existing.descriptor, descriptor))
				{
					existing.stages |= resource.stages;
					existing.access |= resource.descriptor.access();
				} else {
					materialized_resources.push(ResolvedPipelineDescriptor {
						descriptor,
						stages: resource.stages,
						access: resource.descriptor.access(),
					});
				}
			}
		}

		let heaps = self.descriptor_heaps.as_ref().unwrap();
		if !address_writes.is_empty() {
			let ranges = address_writes.iter().map(|(_, range, ..)| *range).collect::<Box<[_]>>();
			let infos = address_writes
				.iter()
				.enumerate()
				.map(|(index, (ty, ..))| {
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT {
							p_address_range: &ranges[index],
						})
				})
				.collect::<Box<[_]>>();
			let destinations = address_writes
				.iter()
				.map(|(_, _, offset, size)| heaps.resource().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_resource_descriptors(&infos, &destinations)
					.expect("Vulkan buffer descriptor write failed. The most likely cause is an invalid device address range.");
			}
		}
		if !image_writes.is_empty() {
			let views = image_writes.iter().map(|(_, view, ..)| *view).collect::<Box<[_]>>();
			let images = image_writes
				.iter()
				.enumerate()
				.map(|(index, (_, _, layout, ..))| vk::ImageDescriptorInfoEXT::default().view(&views[index]).layout(*layout))
				.collect::<Box<[_]>>();
			let infos = image_writes
				.iter()
				.enumerate()
				.map(|(index, (ty, ..))| {
					vk::ResourceDescriptorInfoEXT::default()
						.ty(*ty)
						.data(vk::ResourceDescriptorDataEXT { p_image: &images[index] })
				})
				.collect::<Box<[_]>>();
			let destinations = image_writes
				.iter()
				.map(|(_, _, _, offset, size)| heaps.resource().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_resource_descriptors(&infos, &destinations)
					.expect(
						"Vulkan image descriptor write failed. The most likely cause is an invalid image view description or layout.",
					);
			}
		}
		if !sampler_writes.is_empty() {
			let reductions = sampler_writes
				.iter()
				.map(|(handle, ..)| {
					vk::SamplerReductionModeCreateInfo::default()
						.reduction_mode(self.samplers[handle.0 as usize].reduction_mode)
				})
				.collect::<Box<[_]>>();
			let mut samplers = sampler_writes
				.iter()
				.map(|(handle, ..)| self.samplers[handle.0 as usize].create_info())
				.collect::<Box<[_]>>();
			for (sampler, reduction) in samplers.iter_mut().zip(reductions.iter()) {
				sampler.p_next = (reduction as *const vk::SamplerReductionModeCreateInfo).cast();
			}
			let destinations = sampler_writes
				.iter()
				.map(|(_, offset, size)| heaps.sampler().host_range(*offset, *size))
				.collect::<Box<[_]>>();
			unsafe {
				self.device
					.descriptor_heap
					.write_sampler_descriptors(&samplers, &destinations)
					.expect(
						"Vulkan sampler descriptor write failed. The most likely cause is an unsupported sampler description.",
					);
			}
		}

		let materialization = DescriptorMaterialization {
			resource_heap_offset,
			resource_heap_size: layout.resource_heap_size,
			sampler_heap_offset,
			sampler_heap_size: layout.sampler_heap_size,
			resources: materialized_resources,
		};
		let handle = self
			.free_materialization_handles
			.pop()
			.unwrap_or_else(|| DescriptorMaterializationHandle(self.descriptor_materializations.len() as u64));
		if handle.0 as usize == self.descriptor_materializations.len() {
			self.descriptor_materializations.push(Some(materialization));
		} else {
			self.descriptor_materializations[handle.0 as usize] = Some(materialization);
		}
		self.materialization_indices.insert(key, handle);
		handle
	}

	pub(crate) fn descriptor_materialization(&self, handle: DescriptorMaterializationHandle) -> &DescriptorMaterialization {
		self.descriptor_materializations[handle.0 as usize].as_ref().expect(
			"Retired Vulkan descriptor materialization was reused. The most likely cause is that a command buffer outlived its frame sequence fence.",
		)
	}
	#[inline]
	pub(crate) fn set_object_debug_name(&mut self, name: Option<&str>, handle: graphics_hardware_interface::Handles) {
		#[cfg(debug_assertions)]
		if let Some(name) = name {
			self.names.insert(handle, name.to_string());
		}
	}

	#[inline]
	pub(crate) fn get_object_debug_name(&self, handle: graphics_hardware_interface::Handles) -> Option<String> {
		#[cfg(debug_assertions)]
		let name = self.names.get(&handle).map(|e| e.clone());

		#[cfg(not(debug_assertions))]
		let name: Option<String> = None;

		name
	}
}
