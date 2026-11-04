//! Lays out the descriptor heaps and the push data that Vulkan shaders read their descriptor sets through.
//!
//! Every descriptor set owns a range of the resource heap, one 64-byte slot per descriptor, and a range of the
//! sampler heap, one sampler descriptor per sampler it holds. A pipeline finds its sets through two base indices per
//! set in the push data, and each shader stage finds its sets' pipeline positions through specialization constants.
//! The GLSL generator in `resource-management/src/shader/besl/backends/glsl/generator.rs` writes shaders to this ABI.

use ash::vk;
use smallvec::SmallVec;

use crate::shader::{DescriptorSetLayout, ResourceKind, ShaderDescriptorSet};

/// The specialization constant id that holds the pipeline index of a shader's set 0. Set position `s` reads its
/// pipeline index from id `SET_SPECIALIZATION_BASE + s`.
///
/// The GLSL generator declares the constants with the same base, `SET_SPECIALIZATION_BASE` in
/// `resource-management/src/shader/besl/backends/glsl/generator.rs`.
pub(crate) const SET_SPECIALIZATION_BASE: u32 = 1984;

/// The most descriptor sets one pipeline can bind: the push data holds two base indices per set in its first 120 bytes.
pub(crate) use crate::shader::MAX_DESCRIPTOR_SETS;

/// The byte of the push data where a pipeline's own push constants start, after the sets' base indices.
pub(crate) const PUSH_CONSTANT_OFFSET: u32 = 128;

/// The bytes of one resource heap slot. Every image, buffer, and acceleration structure descriptor takes one, so a
/// descriptor's heap index is its slot.
pub(crate) const RESOURCE_DESCRIPTOR_SIZE: u64 = 64;

/// How many fixed samplers the sampler heap holds before the sets' own samplers: every combination of filter, mip
/// filter, reduction, and address mode.
pub(crate) const FIXED_SAMPLER_COUNT: u32 = 36;

/// Returns the fixed sampler at sampler heap index `index`, as `docs/reference/ghi.mdx` lays them out:
/// `filter + 2 × mip filter + 4 × reduction + 12 × address`.
pub(crate) fn fixed_sampler(index: u32) -> crate::sampler::StaticSampler {
	let filter = |bit: u32| match bit {
		0 => crate::FilteringModes::Closest,
		_ => crate::FilteringModes::Linear,
	};
	crate::sampler::StaticSampler {
		filtering_mode: filter(index % 2),
		mip_map_mode: filter(index / 2 % 2),
		reduction_mode: match index / 4 % 3 {
			0 => crate::SamplingReductionModes::WeightedAverage,
			1 => crate::SamplingReductionModes::Min,
			_ => crate::SamplingReductionModes::Max,
		},
		addressing_mode: match index / 12 {
			0 => crate::SamplerAddressingModes::Clamp,
			1 => crate::SamplerAddressingModes::Repeat,
			_ => crate::SamplerAddressingModes::Mirror,
		},
	}
}

/// Returns whether a resource of `kind` holds samplers in its set's sampler range, one per array element.
pub(crate) fn holds_samplers(kind: ResourceKind) -> bool {
	matches!(kind, ResourceKind::CombinedImageSampler | ResourceKind::Sampler)
}

/// The `PipelineLayout` struct describes the descriptor sets a pipeline's shaders read and its push constants, so
/// binding sets knows where each one goes in the push data and which resources each one makes commands access.
///
/// Build one with [`build_pipeline_layout`]. Next, give each stage its set positions with
/// [`push_set_specializations`].
#[derive(Clone)]
pub(crate) struct PipelineLayout {
	/// The sets the stages declare, matched by layout identity, in the order the stages first declare them. Set `p`'s
	/// base indices are words `2p` and `2p + 1` of the push data.
	pub(crate) sets: SmallVec<[PipelineSet; 4]>,
	/// The bytes of push constants the pipeline declares, which start at [`PUSH_CONSTANT_OFFSET`] of the push data.
	pub(crate) push_constant_size: u32,
}

/// The `PipelineSet` struct is one descriptor set a pipeline binds, with the stages that read each of its resources.
#[derive(Clone)]
pub(crate) struct PipelineSet {
	pub(crate) layout: DescriptorSetLayout,
	/// The stages that use each resource of the layout, by resource index. A resource no stage uses has none.
	pub(crate) stages: SmallVec<[crate::Stages; 8]>,
}

impl PipelineSet {
	/// Returns whether any stage uses a resource of the set, which then has to be bound before shader work.
	pub(crate) fn used(&self) -> bool {
		self.stages.iter().any(|stages| !stages.is_empty())
	}
}

/// Builds the layout of a pipeline whose stages declare `stage_sets`: each stage with the sets its shader declares,
/// in the shader's set order.
///
/// Sets of equal layout identity are one pipeline set, so stages that share a layer read one set.
pub(crate) fn build_pipeline_layout<'a>(
	stage_sets: impl IntoIterator<Item = (crate::Stages, &'a [ShaderDescriptorSet])>,
	push_constant_ranges: &[crate::pipelines::PushConstantRange],
	properties: &vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'_>,
) -> PipelineLayout {
	let mut sets = SmallVec::<[PipelineSet; 4]>::new();
	for (stage, shader_sets) in stage_sets {
		for shader_set in shader_sets {
			let identity = shader_set.layout().identity();
			let position = match sets.iter().position(|set| set.layout.identity() == identity) {
				Some(position) => position,
				None => {
					sets.push(PipelineSet {
						layout: shader_set.layout().clone(),
						stages: SmallVec::from_elem(crate::Stages::empty(), shader_set.layout().resources().len()),
					});
					sets.len() - 1
				}
			};
			for (index, _) in shader_set.used_resources() {
				sets[position].stages[index] |= stage;
			}
		}
	}
	assert!(
		sets.len() <= MAX_DESCRIPTOR_SETS,
		"Vulkan pipeline creation failed because its shaders declare {} distinct descriptor sets. The most likely cause is that the stages declare different layers, which add up past the limit. Declare at most {MAX_DESCRIPTOR_SETS} sets across a pipeline's stages.",
		sets.len(),
	);

	let push_constant_size = push_constant_ranges
		.iter()
		.map(|range| {
			range.offset.checked_add(range.size).expect(
				"Vulkan push constant range overflowed. The most likely cause is an invalid push constant range. Pass ranges whose offset and size fit in 32 bits.",
			)
		})
		.max()
		.unwrap_or(0);
	assert!(
		u64::from(PUSH_CONSTANT_OFFSET + push_constant_size) <= properties.max_push_data_size,
		"Vulkan push data is exhausted. The most likely cause is that the pipeline declares {push_constant_size} bytes of push constants, which together with the {PUSH_CONSTANT_OFFSET} bytes of set base indices exceed the device's {} bytes of push data. Move push constants into a buffer.",
		properties.max_push_data_size,
	);

	PipelineLayout {
		sets,
		push_constant_size,
	}
}

/// Returns the position among the pipeline's sets of `shader_set`, which a stage's shader declares.
fn pipeline_set_index(layout: &PipelineLayout, shader_set: &ShaderDescriptorSet) -> u32 {
	let index = layout
		.sets
		.iter()
		.position(|set| set.layout.identity() == shader_set.layout().identity())
		.expect("Missing Vulkan pipeline set. The most likely cause is that the layout was built from other shaders than the stages it specializes.");
	index as u32
}

/// Appends to a stage's specialization constants the pipeline index of each set the stage's shader declares, at
/// [`SET_SPECIALIZATION_BASE`] plus the set's position in `shader_sets`.
pub(crate) fn push_set_specializations(
	layout: &PipelineLayout,
	shader_sets: &[ShaderDescriptorSet],
	data: &mut SmallVec<[u8; 64]>,
	entries: &mut SmallVec<[vk::SpecializationMapEntry; 16]>,
) {
	for (position, shader_set) in (0u32..).zip(shader_sets) {
		entries.push(
			vk::SpecializationMapEntry::default()
				.constant_id(SET_SPECIALIZATION_BASE + position)
				.offset(data.len() as u32)
				.size(4),
		);
		data.extend_from_slice(&pipeline_set_index(layout, shader_set).to_ne_bytes());
	}
}

/// Appends to a stage's descriptor mappings one for each set the stage's shader declares that holds an acceleration
/// structure or a buffer. Chain them to the stage with a `VkShaderDescriptorSetAndBindingMappingInfoEXT`.
///
/// Shaders declare acceleration structures and buffers at `layout(set = s, binding = b)` instead of reading them from
/// the heap alias. Through the heap alias, NVIDIA drivers lose the device when a ray query traverses a structure, and
/// read a buffer's struct members from the buffer's start instead of their offset. `s` is the set's position in
/// `shader_sets` and `b` the binding's first descriptor in the set, so the mapping reads the pipeline set's resource
/// base from the push data and finds the descriptor where every other one is: at that base plus `b` plus the array
/// element, in 64-byte slots.
pub(crate) fn push_binding_mappings(
	layout: &PipelineLayout,
	shader_sets: &[ShaderDescriptorSet],
	mappings: &mut SmallVec<[vk::DescriptorSetAndBindingMappingEXT<'static>; 2]>,
) {
	for (position, shader_set) in (0u32..).zip(shader_sets) {
		let set_layout = shader_set.layout();
		if !set_layout.resources().iter().any(|resource| {
			matches!(
				resource.kind(),
				ResourceKind::AccelerationStructure | ResourceKind::StorageBuffer | ResourceKind::UniformBuffer
			)
		}) {
			continue;
		}
		let mut push_index = vk::DescriptorMappingSourcePushIndexEXT::default();
		// The resource base is the first word of the set's two in the push data.
		push_index.push_offset = pipeline_set_index(layout, shader_set) * 8;
		push_index.heap_index_stride = RESOURCE_DESCRIPTOR_SIZE as u32;
		push_index.heap_array_stride = RESOURCE_DESCRIPTOR_SIZE as u32;
		mappings.push(
			vk::DescriptorSetAndBindingMappingEXT::default()
				.descriptor_set(position)
				.first_binding(0)
				.binding_count(set_layout.element_count())
				.resource_mask(
					vk::SpirvResourceTypeFlagsEXT::ACCELERATION_STRUCTURE
						| vk::SpirvResourceTypeFlagsEXT::READ_ONLY_STORAGE_BUFFER
						| vk::SpirvResourceTypeFlagsEXT::READ_WRITE_STORAGE_BUFFER,
				)
				.source(vk::DescriptorMappingSourceEXT::HEAP_WITH_PUSH_INDEX)
				.source_data(vk::DescriptorMappingSourceDataEXT { push_index }),
		);
	}
}

pub(crate) fn descriptor_type(kind: ResourceKind) -> Option<vk::DescriptorType> {
	match kind {
		ResourceKind::UniformBuffer => Some(vk::DescriptorType::UNIFORM_BUFFER),
		ResourceKind::StorageBuffer => Some(vk::DescriptorType::STORAGE_BUFFER),
		ResourceKind::SampledImage | ResourceKind::CombinedImageSampler => Some(vk::DescriptorType::SAMPLED_IMAGE),
		ResourceKind::StorageImage => Some(vk::DescriptorType::STORAGE_IMAGE),
		ResourceKind::InputAttachment => Some(vk::DescriptorType::INPUT_ATTACHMENT),
		ResourceKind::Sampler => None,
		ResourceKind::AccelerationStructure => Some(vk::DescriptorType::ACCELERATION_STRUCTURE_KHR),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::shader::{ResourceSlot, ShaderResourceDescriptor};

	fn properties() -> vk::PhysicalDeviceDescriptorHeapPropertiesEXT<'static> {
		vk::PhysicalDeviceDescriptorHeapPropertiesEXT::default().max_push_data_size(256)
	}

	fn layout(name: &str, kinds: &[ResourceKind]) -> DescriptorSetLayout {
		DescriptorSetLayout::new(
			name,
			(0u32..).zip(kinds).map(|(slot, kind)| {
				ShaderResourceDescriptor::single(ResourceSlot::new(slot), *kind, crate::AccessPolicies::READ)
			}),
		)
	}

	#[test]
	fn stages_share_the_sets_of_one_layout_in_the_order_they_first_declare_them() {
		let scene = layout("scene", &[ResourceKind::StorageBuffer, ResourceKind::CombinedImageSampler]);
		let pass = layout("pass", &[ResourceKind::StorageImage]);
		let vertex = [ShaderDescriptorSet::with_used(scene.clone(), 0b01)];
		let fragment = [
			ShaderDescriptorSet::new(pass.clone()),
			ShaderDescriptorSet::with_used(scene.clone(), 0b10),
		];
		let layout = build_pipeline_layout(
			[(crate::Stages::VERTEX, &vertex[..]), (crate::Stages::FRAGMENT, &fragment[..])],
			&[],
			&properties(),
		);

		assert_eq!(layout.sets.len(), 2);
		assert_eq!(layout.sets[0].layout.identity(), scene.identity());
		assert_eq!(&layout.sets[0].stages[..], [crate::Stages::VERTEX, crate::Stages::FRAGMENT]);
		assert_eq!(&layout.sets[1].stages[..], [crate::Stages::FRAGMENT]);

		// The fragment shader's set 0 is the pipeline's set 1, and its set 1 the pipeline's set 0.
		let (mut data, mut entries) = (SmallVec::new(), SmallVec::new());
		push_set_specializations(&layout, &fragment, &mut data, &mut entries);
		let ids = entries.iter().map(|entry| entry.constant_id).collect::<Vec<_>>();
		assert_eq!(ids, [SET_SPECIALIZATION_BASE, SET_SPECIALIZATION_BASE + 1]);
		assert_eq!(&data[..], [1u32.to_ne_bytes(), 0u32.to_ne_bytes()].concat());
	}

	#[test]
	fn fixed_samplers_follow_the_documented_index() {
		let sampler = fixed_sampler(1 + 2 + 4 * 2 + 12 * 2);
		assert_eq!(sampler.filtering_mode, crate::FilteringModes::Linear);
		assert_eq!(sampler.mip_map_mode, crate::FilteringModes::Linear);
		assert_eq!(sampler.reduction_mode, crate::SamplingReductionModes::Max);
		assert_eq!(sampler.addressing_mode, crate::SamplerAddressingModes::Mirror);
	}
}
