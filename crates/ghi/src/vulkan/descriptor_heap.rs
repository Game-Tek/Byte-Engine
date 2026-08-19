use ash::vk;

/// The `PipelineResourceDescriptor` struct retains one merged flat resource and its descriptor-heap locations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PipelineResourceDescriptor {
	pub(crate) descriptor: crate::shader::ShaderResourceDescriptor,
	pub(crate) stages: crate::Stages,
	pub(crate) resource_heap_offset: Option<u32>,
	pub(crate) sampler_heap_offset: Option<u32>,
	pub(crate) resource_stride: u32,
	pub(crate) sampler_stride: u32,
}

/// The `PipelineLayout` struct retains the descriptor mappings and push-data contract derived from a pipeline's shaders.
#[derive(Clone)]
pub(crate) struct PipelineLayout {
	pub(crate) resources: Vec<PipelineResourceDescriptor>,
	pub(crate) push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
	pub(crate) push_constant_size: u32,
	pub(crate) heap_push_data_offset: u32,
	pub(crate) resource_heap_size: u32,
	pub(crate) sampler_heap_size: u32,
}

/// The `PipelineLayoutKey` struct identifies a reusable flat descriptor-heap layout.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(crate) struct PipelineLayoutKey {
	resources: Vec<PipelineResourceDescriptor>,
	push_constant_ranges: Vec<crate::pipelines::PushConstantRange>,
}

impl PipelineLayoutKey {
	pub(crate) fn new(layout: &PipelineLayout) -> Self {
		Self {
			resources: layout.resources.clone(),
			push_constant_ranges: layout.push_constant_ranges.clone(),
		}
	}
}

#[inline]
pub(crate) fn align_up(value: u64, alignment: u64) -> u64 {
	assert!(
		alignment.is_power_of_two(),
		"Invalid Vulkan descriptor alignment. The most likely cause is that the device reported a non-power-of-two descriptor-heap alignment.",
	);
	(value + alignment - 1) & !(alignment - 1)
}

pub(crate) fn resource_range_end(descriptor: crate::shader::ShaderResourceDescriptor) -> u32 {
	descriptor.slot().index().checked_add(descriptor.count()).expect(
		"Vulkan shader resource range overflowed. The most likely cause is an invalid flat resource slot or descriptor count.",
	)
}

pub(crate) fn resource_ranges_overlap(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	let left_start = left.slot().index();
	let right_start = right.slot().index();
	left_start < resource_range_end(right) && right_start < resource_range_end(left)
}

pub(crate) fn resource_accepts_retained_slot_key(
	descriptor: crate::shader::ShaderResourceDescriptor,
	stored_slot: crate::shader::ResourceSlot,
) -> bool {
	let base = descriptor.slot().index();
	let stored = stored_slot.index();
	stored <= base || stored >= resource_range_end(descriptor)
}

fn resource_representations_match(
	left: crate::shader::ShaderResourceDescriptor,
	right: crate::shader::ShaderResourceDescriptor,
) -> bool {
	left.slot() == right.slot()
		&& left.kind() == right.kind()
		&& left.count() == right.count()
		&& left.texture_view() == right.texture_view()
		&& left.buffer_element_stride() == right.buffer_element_stride()
}

/// Canonicalizes one shader stage so declaration order cannot change pipeline mappings.
fn canonicalize_stage_resources(
	resources: &[crate::shader::ShaderResourceDescriptor],
) -> Vec<crate::shader::ShaderResourceDescriptor> {
	let mut sorted = resources.to_vec();
	sorted.sort_by_key(|descriptor| descriptor.slot());

	let mut canonical = Vec::<crate::shader::ShaderResourceDescriptor>::with_capacity(sorted.len());
	for descriptor in sorted {
		if let Some(previous) = canonical.last_mut() {
			if previous.slot() == descriptor.slot() {
				assert!(
					resource_representations_match(*previous, descriptor),
					"Conflicting Vulkan shader resources. The most likely cause is that one stage declared the same flat slot with incompatible representations.",
				);
				*previous = crate::shader::ShaderResourceDescriptor::new(
					previous.slot(),
					previous.kind(),
					previous.count(),
					previous.access() | descriptor.access(),
				)
				.texture_view_type(previous.texture_view())
				.buffer_stride(previous.buffer_element_stride());
				continue;
			}

			assert!(
				!resource_ranges_overlap(*previous, descriptor),
				"Overlapping Vulkan shader resources. The most likely cause is that one stage declared intersecting flat resource ranges.",
			);
		}
		canonical.push(descriptor);
	}

	canonical
}

fn descriptor_heap_representation(
	descriptor: crate::shader::ShaderResourceDescriptor,
	properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
) -> (Option<(u64, u64)>, Option<(u64, u64)>) {
	let image = (properties.image_descriptor_size, properties.image_descriptor_alignment);
	let buffer = (properties.buffer_descriptor_size, properties.buffer_descriptor_alignment);
	let sampler = (properties.sampler_descriptor_size, properties.sampler_descriptor_alignment);

	match descriptor.kind() {
		crate::shader::ResourceKind::UniformBuffer
		| crate::shader::ResourceKind::StorageBuffer
		| crate::shader::ResourceKind::AccelerationStructure => (Some(buffer), None),
		crate::shader::ResourceKind::SampledImage
		| crate::shader::ResourceKind::StorageImage
		| crate::shader::ResourceKind::InputAttachment => (Some(image), None),
		crate::shader::ResourceKind::CombinedImageSampler => (Some(image), Some(sampler)),
		crate::shader::ResourceKind::Sampler => (None, Some(sampler)),
	}
}

fn reserve_descriptor_range(cursor: &mut u64, count: u32, size: u64, alignment: u64) -> (u32, u32) {
	assert!(
		size > 0,
		"Invalid Vulkan descriptor size. The most likely cause is incomplete descriptor-heap properties."
	);
	*cursor = align_up(*cursor, alignment);
	let offset = u32::try_from(*cursor).expect(
		"Vulkan descriptor-heap offset exceeded 32 bits. The most likely cause is a pipeline resource layout larger than the extension mapping API supports.",
	);
	let stride = u32::try_from(size)
		.expect("Vulkan descriptor size exceeded 32 bits. The most likely cause is invalid descriptor-heap properties.");
	*cursor = cursor
		.checked_add(
			size.checked_mul(count as u64)
				.expect("Vulkan descriptor array size overflowed. The most likely cause is an invalid shader resource count."),
		)
		.expect("Vulkan descriptor heap size overflowed. The most likely cause is an invalid pipeline resource interface.");
	(offset, stride)
}

/// Builds a descriptor-heap layout by merging every shader stage's flat resource interface.
pub(crate) fn build_pipeline_layout(
	stage_resources: &[(crate::Stages, Vec<crate::shader::ShaderResourceDescriptor>)],
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
	properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
) -> PipelineLayout {
	let mut merged = Vec::<(crate::shader::ShaderResourceDescriptor, crate::Stages)>::new();

	for (stage, resources) in stage_resources {
		for descriptor in canonicalize_stage_resources(resources) {
			if let Some((existing, existing_stages)) =
				merged.iter_mut().find(|(existing, _)| existing.slot() == descriptor.slot())
			{
				assert!(
					resource_representations_match(*existing, descriptor),
					"Conflicting Vulkan pipeline resources. The most likely cause is that shader stages declared incompatible resources at the same flat slot.",
				);
				*existing_stages |= *stage;
				*existing = crate::shader::ShaderResourceDescriptor::new(
					descriptor.slot(),
					descriptor.kind(),
					descriptor.count(),
					existing.access() | descriptor.access(),
				)
				.texture_view_type(descriptor.texture_view())
				.buffer_stride(descriptor.buffer_element_stride());
				continue;
			}

			assert!(
				merged
					.iter()
					.all(|(existing, _)| !resource_ranges_overlap(*existing, descriptor)),
				"Overlapping Vulkan pipeline resources. The most likely cause is that shader resource arrays reserve intersecting flat slot ranges.",
			);
			merged.push((descriptor, *stage));
		}
	}
	merged.sort_by_key(|(descriptor, _)| descriptor.slot());

	let mut resource_cursor = 0u64;
	let mut sampler_cursor = 0u64;
	let resources = merged
		.into_iter()
		.map(|(descriptor, stages)| {
			let (resource, sampler) = descriptor_heap_representation(descriptor, properties);
			let (resource_heap_offset, resource_stride) = resource
				.map(|(size, alignment)| reserve_descriptor_range(&mut resource_cursor, descriptor.count(), size, alignment))
				.map_or((None, 0), |(offset, stride)| (Some(offset), stride));
			let (sampler_heap_offset, sampler_stride) = sampler
				.map(|(size, alignment)| reserve_descriptor_range(&mut sampler_cursor, descriptor.count(), size, alignment))
				.map_or((None, 0), |(offset, stride)| (Some(offset), stride));

			PipelineResourceDescriptor {
				descriptor,
				stages,
				resource_heap_offset,
				sampler_heap_offset,
				resource_stride,
				sampler_stride,
			}
		})
		.collect::<Vec<_>>();

	let push_constant_size = push_constant_ranges
		.iter()
		.map(|range| {
			range
				.offset
				.checked_add(range.size)
				.expect("Vulkan push-data range overflowed. The most likely cause is an invalid push-constant range.")
		})
		.max()
		.unwrap_or(0);
	let heap_push_data_offset = u32::try_from(align_up(push_constant_size as u64, 4)).unwrap();

	assert!(
		heap_push_data_offset as u64 + 8 <= properties.max_push_data_size,
		"Vulkan push data is exhausted. The most likely cause is that declared push constants leave no room for descriptor-heap base offsets.",
	);

	PipelineLayout {
		resources,
		push_constant_ranges: push_constant_ranges.to_vec(),
		push_constant_size,
		heap_push_data_offset,
		resource_heap_size: u32::try_from(resource_cursor)
			.expect("Vulkan resource layout exceeded 32 bits. The most likely cause is an oversized descriptor array."),
		sampler_heap_size: u32::try_from(sampler_cursor)
			.expect("Vulkan sampler layout exceeded 32 bits. The most likely cause is an oversized descriptor array."),
	}
}

pub(crate) fn descriptor_type(kind: crate::shader::ResourceKind) -> Option<vk::DescriptorType> {
	match kind {
		crate::shader::ResourceKind::UniformBuffer => Some(vk::DescriptorType::UNIFORM_BUFFER),
		crate::shader::ResourceKind::StorageBuffer => Some(vk::DescriptorType::STORAGE_BUFFER),
		crate::shader::ResourceKind::SampledImage | crate::shader::ResourceKind::CombinedImageSampler => {
			Some(vk::DescriptorType::SAMPLED_IMAGE)
		}
		crate::shader::ResourceKind::StorageImage => Some(vk::DescriptorType::STORAGE_IMAGE),
		crate::shader::ResourceKind::InputAttachment => Some(vk::DescriptorType::INPUT_ATTACHMENT),
		crate::shader::ResourceKind::Sampler => None,
		crate::shader::ResourceKind::AccelerationStructure => Some(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR),
	}
}

fn spirv_resource_mask(descriptor: crate::shader::ShaderResourceDescriptor) -> vk::SpirvResourceTypeFlagsEXT {
	match descriptor.kind() {
		crate::shader::ResourceKind::UniformBuffer => vk::SpirvResourceTypeFlagsEXT::UNIFORM_BUFFER,
		crate::shader::ResourceKind::StorageBuffer if descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
			vk::SpirvResourceTypeFlagsEXT::READ_WRITE_STORAGE_BUFFER
		}
		crate::shader::ResourceKind::StorageBuffer => vk::SpirvResourceTypeFlagsEXT::READ_ONLY_STORAGE_BUFFER,
		crate::shader::ResourceKind::SampledImage => vk::SpirvResourceTypeFlagsEXT::SAMPLED_IMAGE,
		crate::shader::ResourceKind::CombinedImageSampler => vk::SpirvResourceTypeFlagsEXT::COMBINED_SAMPLED_IMAGE,
		crate::shader::ResourceKind::StorageImage if descriptor.access().intersects(crate::AccessPolicies::WRITE) => {
			vk::SpirvResourceTypeFlagsEXT::READ_WRITE_IMAGE
		}
		crate::shader::ResourceKind::StorageImage | crate::shader::ResourceKind::InputAttachment => {
			vk::SpirvResourceTypeFlagsEXT::READ_ONLY_IMAGE
		}
		crate::shader::ResourceKind::Sampler => vk::SpirvResourceTypeFlagsEXT::SAMPLER,
		crate::shader::ResourceKind::AccelerationStructure => vk::SpirvResourceTypeFlagsEXT::ACCELERATION_STRUCTURE,
	}
}

/// Builds the set-zero binding mappings consumed by one pipeline shader stage.
pub(crate) fn build_shader_mappings(
	layout: &PipelineLayout,
	shader_resources: &[crate::shader::ShaderResourceDescriptor],
) -> Vec<vk::DescriptorSetAndBindingMappingEXT<'static>> {
	canonicalize_stage_resources(shader_resources)
		.into_iter()
		.map(|descriptor| {
			let resource = layout
				.resources
				.iter()
				.find(|resource| resource.descriptor.slot() == descriptor.slot())
				.expect("Missing Vulkan pipeline resource mapping. The most likely cause is inconsistent shader metadata.");
			let is_sampler = descriptor.kind() == crate::shader::ResourceKind::Sampler;
			let mut push_index = vk::DescriptorMappingSourcePushIndexEXT::default()
				.heap_offset(if is_sampler {
					resource.sampler_heap_offset.unwrap_or(0)
				} else {
					resource.resource_heap_offset.unwrap_or(0)
				})
				.push_offset(layout.heap_push_data_offset + u32::from(is_sampler) * 4)
				.heap_index_stride(1)
				.heap_array_stride(if is_sampler {
					resource.sampler_stride
				} else {
					resource.resource_stride
				});

			if descriptor.kind() == crate::shader::ResourceKind::CombinedImageSampler {
				push_index = push_index
					.sampler_heap_offset(resource.sampler_heap_offset.unwrap())
					.sampler_push_offset(layout.heap_push_data_offset + 4)
					.sampler_heap_index_stride(1)
					.sampler_heap_array_stride(resource.sampler_stride);
			}

			vk::DescriptorSetAndBindingMappingEXT::default()
				.descriptor_set(0)
				.first_binding(descriptor.slot().index())
				.binding_count(1)
				.resource_mask(spirv_resource_mask(descriptor))
				.source(vk::DescriptorMappingSourceEXT::HEAP_WITH_PUSH_INDEX)
				.source_data(vk::DescriptorMappingSourceDataEXT { push_index })
		})
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	fn properties() -> vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static> {
		vk::PhysicalDeviceDescriptorHeapPropertiesEXT::default()
			.image_descriptor_size(64)
			.image_descriptor_alignment(64)
			.buffer_descriptor_size(32)
			.buffer_descriptor_alignment(32)
			.sampler_descriptor_size(16)
			.sampler_descriptor_alignment(16)
			.max_push_data_size(256)
	}

	fn resource(slot: u32, kind: crate::shader::ResourceKind, count: u32) -> crate::shader::ShaderResourceDescriptor {
		crate::shader::ShaderResourceDescriptor::new(
			crate::shader::ResourceSlot::new(slot),
			kind,
			count,
			crate::AccessPolicies::READ,
		)
	}

	#[test]
	fn flat_arrays_reserve_ranges_but_one_native_mapping() {
		let resources = vec![
			resource(9, crate::shader::ResourceKind::CombinedImageSampler, 1024),
			resource(1033, crate::shader::ResourceKind::StorageBuffer, 1),
		];
		let layout = build_pipeline_layout(&[(crate::Stages::COMPUTE, resources.clone())], &[], &properties());
		let mappings = build_shader_mappings(&layout, &resources);

		assert_eq!(layout.resources.len(), 2);
		assert_eq!(mappings.len(), 2);
		assert_eq!(mappings[0].first_binding, 9);
		assert_eq!(mappings[0].binding_count, 1);
		assert_eq!(layout.resources[0].resource_stride, 64);
		assert_eq!(layout.resources[0].sampler_stride, 16);
		assert_eq!(layout.resources[1].resource_heap_offset, Some(64 * 1024));
	}

	#[test]
	#[should_panic(expected = "Overlapping Vulkan shader resources")]
	fn flat_arrays_reject_interior_resource_slots() {
		let resources = vec![
			resource(9, crate::shader::ResourceKind::CombinedImageSampler, 1024),
			resource(10, crate::shader::ResourceKind::StorageBuffer, 1),
		];
		let _ = build_pipeline_layout(&[(crate::Stages::COMPUTE, resources)], &[], &properties());
	}

	#[test]
	fn compatible_stage_resources_merge_visibility_and_access() {
		let read = resource(4, crate::shader::ResourceKind::StorageBuffer, 1);
		let write =
			crate::shader::ShaderResourceDescriptor::new(read.slot(), read.kind(), read.count(), crate::AccessPolicies::WRITE);
		let layout = build_pipeline_layout(
			&[(crate::Stages::VERTEX, vec![read]), (crate::Stages::FRAGMENT, vec![write])],
			&[],
			&properties(),
		);

		assert_eq!(layout.resources.len(), 1);
		assert_eq!(layout.resources[0].stages, crate::Stages::VERTEX | crate::Stages::FRAGMENT);
		assert_eq!(layout.resources[0].descriptor.access(), crate::AccessPolicies::READ_WRITE);
	}
}
