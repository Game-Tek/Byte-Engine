use ash::vk;

use crate::{Layouts, Size, Stages, Uses, graphics_hardware_interface};

/// Folds the Vulkan flags of every `(ghi, vulkan)` table entry whose GHI flags satisfy `matches`.
fn fold_flags<G: Copy, V: Copy + Default + std::ops::BitOr<Output = V>>(table: &[(G, V)], matches: impl Fn(G) -> bool) -> V {
	table
		.iter()
		.filter(|(ghi, _)| matches(*ghi))
		.fold(V::default(), |flags, &(_, vulkan)| flags | vulkan)
}

pub(super) fn uses_to_vk_usage_flags(usage: Uses) -> vk::BufferUsageFlags {
	use vk::BufferUsageFlags as Flags;

	fold_flags(
		&[
			(Uses::Vertex, Flags::VERTEX_BUFFER),
			(Uses::Index, Flags::INDEX_BUFFER),
			(Uses::Uniform, Flags::UNIFORM_BUFFER),
			(Uses::Storage, Flags::STORAGE_BUFFER),
			(Uses::TransferSource, Flags::TRANSFER_SRC),
			(Uses::TransferDestination, Flags::TRANSFER_DST),
			(Uses::AccelerationStructure, Flags::ACCELERATION_STRUCTURE_STORAGE_KHR),
			(Uses::Indirect, Flags::INDIRECT_BUFFER),
			(Uses::ShaderBindingTable, Flags::SHADER_BINDING_TABLE_KHR),
			(Uses::AccelerationStructureBuildScratch, Flags::STORAGE_BUFFER),
			(
				Uses::AccelerationStructureBuild,
				Flags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
			),
		],
		|uses| usage.contains(uses),
	)
}

pub(super) fn to_clear_value(clear: graphics_hardware_interface::ClearValue) -> vk::ClearValue {
	match clear {
		graphics_hardware_interface::ClearValue::None => vk::ClearValue::default(),
		graphics_hardware_interface::ClearValue::Color(clear) => vk::ClearValue {
			color: vk::ClearColorValue {
				float32: [clear.r, clear.g, clear.b, clear.a],
			},
		},
		graphics_hardware_interface::ClearValue::Depth(depth) => vk::ClearValue {
			depth_stencil: vk::ClearDepthStencilValue { depth, stencil: 0 },
		},
		graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearValue {
			color: vk::ClearColorValue { uint32: [r, g, b, a] },
		},
	}
}

pub(super) fn texture_format_and_resource_use_to_image_layout(
	texture_format: crate::Formats,
	layout: Layouts,
	access: Option<crate::AccessPolicies>,
) -> vk::ImageLayout {
	match layout {
		Layouts::Undefined | Layouts::ShaderBindingTable | Layouts::Indirect => vk::ImageLayout::UNDEFINED,
		Layouts::RenderTarget if texture_format.is_depth() => vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
		Layouts::RenderTarget => vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL,
		Layouts::Transfer => match access {
			Some(access) if access.intersects(crate::AccessPolicies::READ) => vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
			Some(access) if access.intersects(crate::AccessPolicies::WRITE) => vk::ImageLayout::TRANSFER_DST_OPTIMAL,
			_ => vk::ImageLayout::UNDEFINED,
		},
		Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		Layouts::Read if texture_format.is_depth() => vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL,
		Layouts::Read => vk::ImageLayout::READ_ONLY_OPTIMAL,
		Layouts::General => vk::ImageLayout::GENERAL,
	}
}

pub(super) fn to_load_operation(value: crate::LoadOp) -> vk::AttachmentLoadOp {
	match value {
		crate::LoadOp::Load => vk::AttachmentLoadOp::LOAD,
		crate::LoadOp::Clear(_) => vk::AttachmentLoadOp::CLEAR,
		crate::LoadOp::Discard => vk::AttachmentLoadOp::DONT_CARE,
	}
}

pub(super) fn to_store_operation(value: crate::StoreOp) -> vk::AttachmentStoreOp {
	match value {
		crate::StoreOp::Store => vk::AttachmentStoreOp::STORE,
		crate::StoreOp::Discard => vk::AttachmentStoreOp::DONT_CARE,
	}
}

/// Selects the aspects a barrier or copy must name for every subresource of an image with `format`.
pub(super) fn image_aspect_mask(format: vk::Format) -> vk::ImageAspectFlags {
	match format {
		vk::Format::D16_UNORM | vk::Format::X8_D24_UNORM_PACK32 | vk::Format::D32_SFLOAT => vk::ImageAspectFlags::DEPTH,
		vk::Format::D16_UNORM_S8_UINT | vk::Format::D24_UNORM_S8_UINT | vk::Format::D32_SFLOAT_S8_UINT => {
			vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
		}
		vk::Format::S8_UINT => vk::ImageAspectFlags::STENCIL,
		_ => vk::ImageAspectFlags::COLOR,
	}
}

/// Packs specialization constants into one data blob with a 4-byte map entry per scalar component.
pub(super) fn build_specialization_entries(
	specialization_map: &[crate::pipelines::SpecializationMapEntry],
) -> (Vec<u8>, Vec<vk::SpecializationMapEntry>) {
	let mut data = Vec::<u8>::with_capacity(256);
	let mut entries = Vec::with_capacity(48);

	for specialization_map_entry in specialization_map {
		let value = specialization_map_entry.get_data();
		let offset = data.len() as u32;
		let constant_type = specialization_map_entry.get_type();
		let scalar_count = match constant_type.as_str() {
			"bool" | "u32" | "f32" => 1,
			"vec2f" => 2,
			"vec3f" => 3,
			"vec4f" => 4,
			_ => panic!(
				"Unsupported Vulkan specialization constant type. The most likely cause is that the Vulkan backend was not updated for a new specialization entry type."
			),
		};
		if constant_type == "bool" {
			// SPIR-V boolean constants are read as a 4-byte VkBool32, but Rust bools are one byte.
			data.extend_from_slice(&vk::Bool32::from(value.iter().any(|byte| *byte != 0)).to_ne_bytes());
		} else {
			assert!(
				value.len() >= scalar_count as usize * 4,
				"Vulkan specialization constant data is smaller than its type. The most likely cause is that the value's Rust type differs from the declared constant type."
			);
			data.extend_from_slice(value);
		}
		for i in 0..scalar_count {
			entries.push(
				vk::SpecializationMapEntry::default()
					.constant_id(specialization_map_entry.get_constant_id() + i)
					.offset(offset + i * 4)
					.size(4),
			);
		}
	}

	(data, entries)
}

pub(super) fn to_format(format: crate::Formats) -> vk::Format {
	match format {
		crate::Formats::R8F
		| crate::Formats::R16sRGB
		| crate::Formats::R32sRGB
		| crate::Formats::RG8F
		| crate::Formats::RG16sRGB
		| crate::Formats::RGB8F
		| crate::Formats::RGB16sRGB
		| crate::Formats::RGBA8F
		| crate::Formats::RGBA16sRGB => vk::Format::UNDEFINED,
		crate::Formats::R8UNORM => vk::Format::R8_UNORM,
		crate::Formats::R8SNORM => vk::Format::R8_SNORM,
		crate::Formats::R8sRGB => vk::Format::R8_SRGB,
		crate::Formats::R16F => vk::Format::R16_SFLOAT,
		crate::Formats::R16UNORM => vk::Format::R16_UNORM,
		crate::Formats::R16SNORM => vk::Format::R16_SNORM,
		crate::Formats::R32F => vk::Format::R32_SFLOAT,
		crate::Formats::R32UNORM | crate::Formats::U32 => vk::Format::R32_UINT,
		crate::Formats::R32SNORM => vk::Format::R32_SINT,
		crate::Formats::RG8UNORM => vk::Format::R8G8_UNORM,
		crate::Formats::RG8SNORM => vk::Format::R8G8_SNORM,
		crate::Formats::RG8sRGB => vk::Format::R8G8_SRGB,
		crate::Formats::RG16F => vk::Format::R16G16_SFLOAT,
		crate::Formats::RG16UNORM => vk::Format::R16G16_UNORM,
		crate::Formats::RG16SNORM => vk::Format::R16G16_SNORM,
		crate::Formats::RGB8UNORM => vk::Format::R8G8B8_UNORM,
		crate::Formats::RGB8SNORM => vk::Format::R8G8B8_SNORM,
		crate::Formats::RGB8sRGB => vk::Format::R8G8B8_SRGB,
		crate::Formats::RGB16F => vk::Format::R16G16B16_SFLOAT,
		crate::Formats::RGB16UNORM => vk::Format::R16G16B16_UNORM,
		crate::Formats::RGB16SNORM => vk::Format::R16G16B16_SNORM,
		crate::Formats::RGBA8UNORM => vk::Format::R8G8B8A8_UNORM,
		crate::Formats::RGBA8SNORM => vk::Format::R8G8B8A8_SNORM,
		crate::Formats::RGBA8sRGB => vk::Format::R8G8B8A8_SRGB,
		crate::Formats::RGBA16F => vk::Format::R16G16B16A16_SFLOAT,
		crate::Formats::RGBA16UNORM => vk::Format::R16G16B16A16_UNORM,
		crate::Formats::RGBA16SNORM => vk::Format::R16G16B16A16_SNORM,
		crate::Formats::RGBu11u11u10 => vk::Format::B10G11R11_UFLOAT_PACK32,
		crate::Formats::BGRAu8 => vk::Format::B8G8R8A8_UNORM,
		crate::Formats::BGRAsRGB => vk::Format::B8G8R8A8_SRGB,
		crate::Formats::Depth16 => vk::Format::D16_UNORM,
		crate::Formats::Depth32 => vk::Format::D32_SFLOAT,
		crate::Formats::BC5 => vk::Format::BC5_UNORM_BLOCK,
		crate::Formats::BC5SNORM => vk::Format::BC5_SNORM_BLOCK,
		crate::Formats::BC7 => vk::Format::BC7_UNORM_BLOCK,
		crate::Formats::BC7SRGB => vk::Format::BC7_SRGB_BLOCK,
	}
}

pub(super) fn to_shader_stage_flags(shader_type: crate::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		crate::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		crate::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		crate::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,
		crate::ShaderTypes::Task => vk::ShaderStageFlags::TASK_EXT,
		crate::ShaderTypes::Mesh => vk::ShaderStageFlags::MESH_EXT,
		crate::ShaderTypes::RayGen => vk::ShaderStageFlags::RAYGEN_KHR,
		crate::ShaderTypes::ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
		crate::ShaderTypes::AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
		crate::ShaderTypes::Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
		crate::ShaderTypes::Miss => vk::ShaderStageFlags::MISS_KHR,
		crate::ShaderTypes::Callable => vk::ShaderStageFlags::CALLABLE_KHR,
	}
}

pub(super) fn to_pipeline_stage_flags(
	stages: Stages,
	layout: Option<Layouts>,
	format: Option<crate::Formats>,
) -> vk::PipelineStageFlags2 {
	use vk::PipelineStageFlags2 as Flags;

	let fragment = match layout {
		Some(Layouts::Read) => Flags::FRAGMENT_SHADER,
		Some(Layouts::RenderTarget) => Flags::COLOR_ATTACHMENT_OUTPUT,
		_ => Flags::NONE,
	} | match format {
		Some(format) if format.is_depth() => Flags::EARLY_FRAGMENT_TESTS | Flags::LATE_FRAGMENT_TESTS,
		Some(_) => Flags::FRAGMENT_SHADER,
		None if layout.is_none() => Flags::FRAGMENT_SHADER,
		None => Flags::NONE,
	};
	let compute = if layout == Some(Layouts::Indirect) {
		Flags::DRAW_INDIRECT
	} else {
		Flags::COMPUTE_SHADER
	};

	fold_flags(
		&[
			(Stages::VERTEX, Flags::VERTEX_ATTRIBUTE_INPUT | Flags::VERTEX_SHADER),
			(Stages::INDEX, Flags::VERTEX_ATTRIBUTE_INPUT | Flags::INDEX_INPUT),
			(Stages::MESH, Flags::MESH_SHADER_EXT),
			(Stages::FRAGMENT, fragment),
			(Stages::COMPUTE, compute),
			(Stages::TRANSFER, Flags::TRANSFER),
			// Presentation is external to the pipeline; TOP_OF_PIPE would be NONE in a first scope, leaving the pre-present
			// barrier and semaphore signal unordered with the frame's last write, whichever stage made it.
			(Stages::PRESENTATION, Flags::ALL_COMMANDS),
			(Stages::RAYGEN, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::CLOSEST_HIT, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::ANY_HIT, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::INTERSECTION, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::MISS, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::CALLABLE, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::ACCELERATION_STRUCTURE_BUILD, Flags::ACCELERATION_STRUCTURE_BUILD_KHR),
			(Stages::LAST, Flags::BOTTOM_OF_PIPE),
		],
		|stage| stages.contains(stage),
	)
}

pub(super) fn to_access_flags(
	accesses: crate::AccessPolicies,
	stages: Stages,
	layout: Layouts,
	format: Option<crate::Formats>,
) -> vk::AccessFlags2 {
	use vk::AccessFlags2 as Flags;

	let depth = format.map(|format| format.is_depth());
	let render_target = layout == Layouts::RenderTarget;
	let mut access_flags = Flags::NONE;

	if accesses.contains(crate::AccessPolicies::READ) {
		let fragment = match (depth, render_target) {
			(Some(false), true) => Flags::COLOR_ATTACHMENT_READ,
			(Some(true), true) => Flags::DEPTH_STENCIL_ATTACHMENT_READ,
			_ => Flags::SHADER_SAMPLED_READ,
		};
		let compute = if layout == Layouts::Indirect {
			Flags::INDIRECT_COMMAND_READ
		} else {
			Flags::SHADER_READ
		};
		let raygen = if layout == Layouts::ShaderBindingTable {
			Flags::SHADER_BINDING_TABLE_READ_KHR
		} else {
			Flags::ACCELERATION_STRUCTURE_READ_KHR
		};
		access_flags |= fold_flags(
			&[
				(Stages::VERTEX, Flags::VERTEX_ATTRIBUTE_READ),
				(Stages::INDEX, Flags::VERTEX_ATTRIBUTE_READ | Flags::INDEX_READ),
				(Stages::TRANSFER, Flags::TRANSFER_READ),
				(Stages::FRAGMENT, fragment),
				(Stages::COMPUTE, compute),
				(Stages::RAYGEN, raygen),
				(Stages::ACCELERATION_STRUCTURE_BUILD, Flags::ACCELERATION_STRUCTURE_READ_KHR),
			],
			|stage| stages.intersects(stage),
		);
	}

	if accesses.contains(crate::AccessPolicies::WRITE) {
		let fragment = match (depth, render_target) {
			(None, _) | (Some(false), true) => Flags::COLOR_ATTACHMENT_WRITE,
			(Some(true), true) => Flags::DEPTH_STENCIL_ATTACHMENT_WRITE,
			(Some(_), false) => Flags::SHADER_WRITE,
		};
		access_flags |= fold_flags(
			&[
				(Stages::TRANSFER, Flags::TRANSFER_WRITE),
				(Stages::COMPUTE, Flags::SHADER_WRITE),
				(Stages::FRAGMENT, fragment),
				(Stages::RAYGEN, Flags::SHADER_WRITE),
				(Stages::ACCELERATION_STRUCTURE_BUILD, Flags::ACCELERATION_STRUCTURE_WRITE_KHR),
			],
			|stage| stages.intersects(stage),
		);
	}

	access_flags
}

/// A depth of 0 or 1 is one slice, so such extents make 2D images; descriptor views make the same distinction.
pub(super) fn image_type_from_extent(extent: utils::Extent) -> Option<vk::ImageType> {
	if extent.width() == 0 {
		None
	} else if extent.height() == 0 {
		Some(vk::ImageType::TYPE_1D)
	} else if extent.depth() <= 1 {
		Some(vk::ImageType::TYPE_2D)
	} else {
		Some(vk::ImageType::TYPE_3D)
	}
}

/// Selects the view type that matches an image's dimensionality. 3D images cannot be arrayed, so `arrayed` is ignored for them.
pub(super) fn image_view_type(image_type: vk::ImageType, arrayed: bool) -> vk::ImageViewType {
	match (image_type, arrayed) {
		(vk::ImageType::TYPE_1D, false) => vk::ImageViewType::TYPE_1D,
		(vk::ImageType::TYPE_1D, true) => vk::ImageViewType::TYPE_1D_ARRAY,
		(vk::ImageType::TYPE_3D, _) => vk::ImageViewType::TYPE_3D,
		(_, false) => vk::ImageViewType::TYPE_2D,
		(_, true) => vk::ImageViewType::TYPE_2D_ARRAY,
	}
}

pub(super) fn extent_into_vk_extent(extent: utils::Extent) -> vk::Extent3D {
	vk::Extent3D {
		width: extent.width(),
		height: extent.height().max(1),
		depth: extent.depth().max(1),
	}
}

pub(super) fn into_vk_image_usage_flags(uses: Uses, format: crate::Formats) -> vk::ImageUsageFlags {
	use vk::ImageUsageFlags as Flags;

	let mut flags = fold_flags(
		&[
			(Uses::Image, Flags::SAMPLED),
			(Uses::InputAttachment, Flags::INPUT_ATTACHMENT),
			(Uses::Clear, Flags::TRANSFER_DST),
			(Uses::Storage, Flags::STORAGE),
			(Uses::TransferSource, Flags::TRANSFER_SRC),
			(Uses::TransferDestination, Flags::TRANSFER_DST),
		],
		|flag| uses.intersects(flag),
	);
	if uses.intersects(Uses::RenderTarget) && !format.is_depth() {
		flags |= Flags::COLOR_ATTACHMENT;
	}
	if uses.intersects(Uses::DepthStencil) || format.is_depth() {
		flags |= Flags::DEPTH_STENCIL_ATTACHMENT;
	}
	flags
}

impl From<Stages> for vk::ShaderStageFlags {
	fn from(stages: Stages) -> Self {
		fold_flags(
			&[
				(Stages::VERTEX, Self::VERTEX),
				(Stages::FRAGMENT, Self::FRAGMENT),
				(Stages::COMPUTE, Self::COMPUTE),
				(Stages::MESH, Self::MESH_EXT),
				(Stages::TASK, Self::TASK_EXT),
				(Stages::RAYGEN, Self::RAYGEN_KHR),
				(Stages::CLOSEST_HIT, Self::CLOSEST_HIT_KHR),
				(Stages::ANY_HIT, Self::ANY_HIT_KHR),
				(Stages::INTERSECTION, Self::INTERSECTION_KHR),
				(Stages::MISS, Self::MISS_KHR),
				(Stages::CALLABLE, Self::CALLABLE_KHR),
			],
			|stage| stages.intersects(stage),
		)
	}
}

impl From<crate::DataTypes> for vk::Format {
	fn from(data_type: crate::DataTypes) -> Self {
		match data_type {
			crate::DataTypes::Float => Self::R32_SFLOAT,
			crate::DataTypes::Float2 => Self::R32G32_SFLOAT,
			crate::DataTypes::Float3 => Self::R32G32B32_SFLOAT,
			crate::DataTypes::Float4 => Self::R32G32B32A32_SFLOAT,
			crate::DataTypes::U8 => Self::R8_UINT,
			crate::DataTypes::U16 => Self::R16_UINT,
			crate::DataTypes::Int => Self::R32_SINT,
			crate::DataTypes::U32 | crate::DataTypes::UInt => Self::R32_UINT,
			crate::DataTypes::Int2 => Self::R32G32_SINT,
			crate::DataTypes::Int3 => Self::R32G32B32_SINT,
			crate::DataTypes::Int4 => Self::R32G32B32A32_SINT,
			crate::DataTypes::UInt2 => Self::R32G32_UINT,
			crate::DataTypes::UInt3 => Self::R32G32B32_UINT,
			crate::DataTypes::UInt4 => Self::R32G32B32A32_UINT,
		}
	}
}

impl Size for &[crate::pipelines::VertexElement<'_>] {
	fn size(&self) -> usize {
		self.iter().map(|element| element.format.size()).sum()
	}
}

impl From<crate::ShaderTypes> for vk::ShaderStageFlags {
	fn from(value: crate::ShaderTypes) -> Self {
		to_shader_stage_flags(value)
	}
}

/// Orders the memory types that can back an allocation with `device_accesses`, best first.
///
/// Host access requires mapped coherent memory, so host writes need no flush and host reads no invalidate. GPU access
/// prefers device-local memory and host reads prefer cached memory. A preference is dropped when no type offers it or
/// its heap is exhausted, such as CPU-writable GPU buffers on devices without resizable BAR.
pub(super) fn memory_type_candidates(
	memory_properties: &vk::PhysicalDeviceMemoryProperties,
	memory_type_bits: u32,
	device_accesses: crate::DeviceAccesses,
) -> Vec<u32> {
	let host_access = device_accesses.intersects(crate::DeviceAccesses::CpuRead | crate::DeviceAccesses::CpuWrite);
	let required = if host_access {
		vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT
	} else {
		vk::MemoryPropertyFlags::empty()
	};
	// Protected and AMD device-coherent memory need device features this backend does not enable.
	let unsupported = vk::MemoryPropertyFlags::PROTECTED
		| vk::MemoryPropertyFlags::DEVICE_COHERENT_AMD
		| vk::MemoryPropertyFlags::DEVICE_UNCACHED_AMD;
	// Uncached host reads are far slower than a GPU reading host memory, so caching outranks device locality.
	let score = |flags: vk::MemoryPropertyFlags| {
		let cached =
			device_accesses.contains(crate::DeviceAccesses::CpuRead) && flags.contains(vk::MemoryPropertyFlags::HOST_CACHED);
		let device_local = device_accesses.intersects(crate::DeviceAccesses::GpuRead | crate::DeviceAccesses::GpuWrite)
			&& flags.contains(vk::MemoryPropertyFlags::DEVICE_LOCAL);
		u8::from(cached) * 2 + u8::from(device_local)
	};

	let mut candidates = memory_properties.memory_types[..memory_properties.memory_type_count as usize]
		.iter()
		.enumerate()
		.filter(|(index, memory_type)| {
			memory_type_bits & (1 << index) != 0
				&& memory_type.property_flags.contains(required)
				&& !memory_type.property_flags.intersects(unsupported)
		})
		.map(|(index, memory_type)| (index as u32, score(memory_type.property_flags)))
		.collect::<Vec<_>>();
	// The stable sort keeps the implementation's order among equal scores, which lists types with fewer extra properties first.
	candidates.sort_by_key(|&(_, score)| std::cmp::Reverse(score));
	candidates.into_iter().map(|(index, _)| index).collect()
}

#[cfg(test)]
mod tests {
	use utils::RGBA;

	use super::*;
	use crate::{AccessPolicies, Formats};

	#[test]
	fn depth_formats_select_depth_aspects() {
		assert!(image_aspect_mask(to_format(crate::Formats::Depth16)) == vk::ImageAspectFlags::DEPTH);
		assert!(image_aspect_mask(to_format(crate::Formats::Depth32)) == vk::ImageAspectFlags::DEPTH);
		assert!(
			image_aspect_mask(vk::Format::D24_UNORM_S8_UINT) == vk::ImageAspectFlags::DEPTH | vk::ImageAspectFlags::STENCIL
		);
		assert!(image_aspect_mask(to_format(crate::Formats::RGBA8UNORM)) == vk::ImageAspectFlags::COLOR);
	}

	fn memory_properties(types: &[vk::MemoryPropertyFlags]) -> vk::PhysicalDeviceMemoryProperties {
		let mut properties = vk::PhysicalDeviceMemoryProperties {
			memory_type_count: types.len() as u32,
			..Default::default()
		};
		for (memory_type, &property_flags) in properties.memory_types.iter_mut().zip(types) {
			memory_type.property_flags = property_flags;
		}
		properties
	}

	#[test]
	fn memory_types_fall_back_when_preferences_are_unavailable() {
		// A discrete GPU without resizable BAR.
		type F = vk::MemoryPropertyFlags;
		let properties = memory_properties(&[
			F::DEVICE_LOCAL,
			F::HOST_VISIBLE | F::HOST_COHERENT,
			F::HOST_VISIBLE | F::HOST_COHERENT | F::HOST_CACHED,
			F::DEVICE_LOCAL | F::PROTECTED,
		]);

		// Device memory comes first; host memory remains a fallback for when device memory runs out.
		assert_eq!(
			memory_type_candidates(&properties, !0, crate::DeviceAccesses::GpuRead),
			vec![0, 1, 2]
		);
		// No device-local host-visible type exists, so CPU-writable GPU buffers use host memory.
		assert_eq!(
			memory_type_candidates(
				&properties,
				!0,
				crate::DeviceAccesses::CpuWrite | crate::DeviceAccesses::GpuRead
			),
			vec![1, 2]
		);
		assert_eq!(
			memory_type_candidates(&properties, !0, crate::DeviceAccesses::CpuRead),
			vec![2, 1]
		);
		assert_eq!(
			memory_type_candidates(&properties, 0b0010, crate::DeviceAccesses::CpuRead),
			vec![1]
		);
	}

	#[test]
	fn memory_types_never_offer_non_coherent_memory_for_host_access() {
		type F = vk::MemoryPropertyFlags;
		let properties = memory_properties(&[
			F::HOST_VISIBLE | F::HOST_CACHED,
			F::DEVICE_LOCAL | F::HOST_VISIBLE | F::HOST_COHERENT,
		]);

		assert_eq!(
			memory_type_candidates(&properties, !0, crate::DeviceAccesses::CpuRead),
			vec![1]
		);
	}

	#[test]
	fn image_views_match_image_dimensionality() {
		let view_type = |extent| image_view_type(image_type_from_extent(extent).unwrap(), false);

		assert!(view_type(utils::Extent::line(64)) == vk::ImageViewType::TYPE_1D);
		assert!(view_type(utils::Extent::rectangle(64, 64)) == vk::ImageViewType::TYPE_2D);
		assert!(view_type(utils::Extent::cube(64, 64, 1)) == vk::ImageViewType::TYPE_2D);
		assert!(view_type(utils::Extent::cube(64, 64, 64)) == vk::ImageViewType::TYPE_3D);
		assert!(image_view_type(vk::ImageType::TYPE_2D, true) == vk::ImageViewType::TYPE_2D_ARRAY);
		assert!(image_view_type(vk::ImageType::TYPE_1D, true) == vk::ImageViewType::TYPE_1D_ARRAY);
	}

	#[test]
	fn specialization_constants_use_four_byte_scalars() {
		let entries = [
			crate::pipelines::SpecializationMapEntry::new(0, "bool".to_string(), true),
			crate::pipelines::SpecializationMapEntry::new(1, "vec2f".to_string(), [1.0f32, 2.0f32]),
			crate::pipelines::SpecializationMapEntry::new(3, "u32".to_string(), 7u32),
		];

		let (data, map_entries) = build_specialization_entries(&entries);

		assert_eq!(data.len(), 16);
		assert_eq!(&data[0..4], &1u32.to_ne_bytes());
		assert_eq!(&data[12..16], &7u32.to_ne_bytes());
		let layout = map_entries
			.iter()
			.map(|entry| (entry.constant_id, entry.offset, entry.size))
			.collect::<Vec<_>>();
		assert_eq!(layout, vec![(0, 0, 4), (1, 4, 4), (2, 8, 4), (3, 12, 4)]);
	}

	#[test]
	fn transfer_image_uses_request_only_transfer_usage() {
		// Blit uses alias transfer uses, and vkCmdBlitImage2 only needs transfer usage; extra bits are invalid for BC formats.
		let value = into_vk_image_usage_flags(
			crate::Uses::Image | crate::Uses::TransferSource | crate::Uses::TransferDestination,
			crate::Formats::BC7,
		);

		assert!(value == vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST);
	}

	#[test]
	fn test_uses_to_vk_usage_flags() {
		let cases = [
			(Uses::Vertex, vk::BufferUsageFlags::VERTEX_BUFFER),
			(Uses::Index, vk::BufferUsageFlags::INDEX_BUFFER),
			(Uses::Uniform, vk::BufferUsageFlags::UNIFORM_BUFFER),
			(Uses::Storage, vk::BufferUsageFlags::STORAGE_BUFFER),
			(Uses::TransferSource, vk::BufferUsageFlags::TRANSFER_SRC),
			(Uses::TransferDestination, vk::BufferUsageFlags::TRANSFER_DST),
			(
				Uses::AccelerationStructure,
				vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR,
			),
			(Uses::Indirect, vk::BufferUsageFlags::INDIRECT_BUFFER),
			(Uses::ShaderBindingTable, vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR),
			(Uses::AccelerationStructureBuildScratch, vk::BufferUsageFlags::STORAGE_BUFFER),
			(
				Uses::AccelerationStructureBuild,
				vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR,
			),
		];
		for (uses, expected) in cases {
			assert!(uses_to_vk_usage_flags(uses).intersects(expected), "{uses:?}");
		}
	}

	#[test]
	fn test_to_clear_value() {
		let value = to_clear_value(graphics_hardware_interface::ClearValue::Color(RGBA::new(0.0, 1.0, 2.0, 3.0)));
		assert_eq!(unsafe { value.color.float32 }, [0.0, 1.0, 2.0, 3.0]);

		for depth in [0.0, 1.0] {
			let value = to_clear_value(graphics_hardware_interface::ClearValue::Depth(depth));
			assert_eq!(unsafe { value.depth_stencil.depth }, depth);
			assert_eq!(unsafe { value.depth_stencil.stencil }, 0);
		}

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Integer(1, 2, 3, 4));
		assert_eq!(unsafe { value.color.int32 }, [1, 2, 3, 4]);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::None);
		assert_eq!(unsafe { value.color.float32 }, [0.0, 0.0, 0.0, 0.0]);
		assert_eq!(unsafe { value.depth_stencil.depth }, 0.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);
	}

	#[test]
	fn test_to_load_and_store_operations() {
		assert_eq!(to_load_operation(crate::LoadOp::Load), vk::AttachmentLoadOp::LOAD);
		assert_eq!(
			to_load_operation(crate::LoadOp::Clear(crate::ClearValue::None)),
			vk::AttachmentLoadOp::CLEAR
		);
		assert_eq!(to_load_operation(crate::LoadOp::Discard), vk::AttachmentLoadOp::DONT_CARE);
		assert_eq!(to_store_operation(crate::StoreOp::Store), vk::AttachmentStoreOp::STORE);
		assert_eq!(to_store_operation(crate::StoreOp::Discard), vk::AttachmentStoreOp::DONT_CARE);
	}

	#[test]
	fn test_texture_format_and_resource_use_to_image_layout() {
		let (color, read, write) = (Formats::RGBA8UNORM, Some(AccessPolicies::READ), Some(AccessPolicies::WRITE));
		let cases = [
			(color, Layouts::Undefined, None, vk::ImageLayout::UNDEFINED),
			(color, Layouts::Undefined, read, vk::ImageLayout::UNDEFINED),
			(color, Layouts::Undefined, write, vk::ImageLayout::UNDEFINED),
			(color, Layouts::RenderTarget, None, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL),
			(
				Formats::Depth32,
				Layouts::RenderTarget,
				None,
				vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
			),
			(
				Formats::Depth16,
				Layouts::RenderTarget,
				None,
				vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL,
			),
			(color, Layouts::Transfer, None, vk::ImageLayout::UNDEFINED),
			(color, Layouts::Transfer, read, vk::ImageLayout::TRANSFER_SRC_OPTIMAL),
			(color, Layouts::Transfer, write, vk::ImageLayout::TRANSFER_DST_OPTIMAL),
			(color, Layouts::Present, None, vk::ImageLayout::PRESENT_SRC_KHR),
			(color, Layouts::Read, None, vk::ImageLayout::READ_ONLY_OPTIMAL),
			(
				Formats::Depth32,
				Layouts::Read,
				None,
				vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL,
			),
			(color, Layouts::General, None, vk::ImageLayout::GENERAL),
			(color, Layouts::ShaderBindingTable, None, vk::ImageLayout::UNDEFINED),
			(color, Layouts::Indirect, None, vk::ImageLayout::UNDEFINED),
		];
		for (format, layout, access, expected) in cases {
			assert_eq!(
				texture_format_and_resource_use_to_image_layout(format, layout, access),
				expected,
				"{format:?} {layout:?} {access:?}"
			);
		}
	}

	#[test]
	fn test_to_format() {
		let cases = [
			(Formats::R8UNORM, vk::Format::R8_UNORM),
			(Formats::R8SNORM, vk::Format::R8_SNORM),
			(Formats::R8F, vk::Format::UNDEFINED),
			(Formats::R16UNORM, vk::Format::R16_UNORM),
			(Formats::R16SNORM, vk::Format::R16_SNORM),
			(Formats::R16F, vk::Format::R16_SFLOAT),
			(Formats::R32UNORM, vk::Format::R32_UINT),
			(Formats::R32SNORM, vk::Format::R32_SINT),
			(Formats::R32F, vk::Format::R32_SFLOAT),
			(Formats::RG8UNORM, vk::Format::R8G8_UNORM),
			(Formats::BC5, vk::Format::BC5_UNORM_BLOCK),
			(Formats::RG8SNORM, vk::Format::R8G8_SNORM),
			(Formats::RG8F, vk::Format::UNDEFINED),
			(Formats::RG16UNORM, vk::Format::R16G16_UNORM),
			(Formats::RG16SNORM, vk::Format::R16G16_SNORM),
			(Formats::RG16F, vk::Format::R16G16_SFLOAT),
			(Formats::RGB16UNORM, vk::Format::R16G16B16_UNORM),
			(Formats::RGB16SNORM, vk::Format::R16G16B16_SNORM),
			(Formats::RGB16F, vk::Format::R16G16B16_SFLOAT),
			(Formats::RGBA8UNORM, vk::Format::R8G8B8A8_UNORM),
			(Formats::BC7, vk::Format::BC7_UNORM_BLOCK),
			(Formats::BC7SRGB, vk::Format::BC7_SRGB_BLOCK),
			(Formats::RGBA8SNORM, vk::Format::R8G8B8A8_SNORM),
			(Formats::RGBA8F, vk::Format::UNDEFINED),
			(Formats::RGBA16UNORM, vk::Format::R16G16B16A16_UNORM),
			(Formats::RGBA16SNORM, vk::Format::R16G16B16A16_SNORM),
			(Formats::RGBA16F, vk::Format::R16G16B16A16_SFLOAT),
			(Formats::BGRAu8, vk::Format::B8G8R8A8_UNORM),
			(Formats::RGBu11u11u10, vk::Format::B10G11R11_UFLOAT_PACK32),
			(Formats::Depth32, vk::Format::D32_SFLOAT),
			(Formats::Depth16, vk::Format::D16_UNORM),
		];
		for (format, expected) in cases {
			assert_eq!(to_format(format), expected, "{format:?}");
		}
	}

	#[test]
	fn test_shader_stage_flags() {
		let cases = [
			(crate::ShaderTypes::Vertex, vk::ShaderStageFlags::VERTEX),
			(crate::ShaderTypes::Fragment, vk::ShaderStageFlags::FRAGMENT),
			(crate::ShaderTypes::Compute, vk::ShaderStageFlags::COMPUTE),
			(crate::ShaderTypes::Task, vk::ShaderStageFlags::TASK_EXT),
			(crate::ShaderTypes::Mesh, vk::ShaderStageFlags::MESH_EXT),
			(crate::ShaderTypes::RayGen, vk::ShaderStageFlags::RAYGEN_KHR),
			(crate::ShaderTypes::ClosestHit, vk::ShaderStageFlags::CLOSEST_HIT_KHR),
			(crate::ShaderTypes::AnyHit, vk::ShaderStageFlags::ANY_HIT_KHR),
			(crate::ShaderTypes::Intersection, vk::ShaderStageFlags::INTERSECTION_KHR),
			(crate::ShaderTypes::Miss, vk::ShaderStageFlags::MISS_KHR),
			(crate::ShaderTypes::Callable, vk::ShaderStageFlags::CALLABLE_KHR),
		];
		for (shader_type, expected) in cases {
			assert_eq!(to_shader_stage_flags(shader_type), expected, "{shader_type:?}");
			assert_eq!(vk::ShaderStageFlags::from(shader_type), expected, "{shader_type:?}");
			assert_eq!(
				vk::ShaderStageFlags::from(Stages::from(shader_type)),
				expected,
				"{shader_type:?}"
			);
		}

		for stages in [
			Stages::ACCELERATION_STRUCTURE_BUILD,
			Stages::TRANSFER,
			Stages::PRESENTATION,
			Stages::NONE,
		] {
			assert_eq!(
				vk::ShaderStageFlags::from(stages),
				vk::ShaderStageFlags::empty(),
				"{stages:?}"
			);
		}
	}

	#[test]
	fn test_to_pipeline_stage_flags() {
		use vk::PipelineStageFlags2 as Flags;

		let cases = [
			(Stages::NONE, None, None, Flags::NONE),
			(
				Stages::VERTEX,
				None,
				None,
				Flags::VERTEX_SHADER | Flags::VERTEX_ATTRIBUTE_INPUT,
			),
			(Stages::MESH, None, None, Flags::MESH_SHADER_EXT),
			(Stages::FRAGMENT, None, None, Flags::FRAGMENT_SHADER),
			(
				Stages::FRAGMENT,
				Some(Layouts::RenderTarget),
				None,
				Flags::COLOR_ATTACHMENT_OUTPUT,
			),
			(
				Stages::FRAGMENT,
				None,
				Some(Formats::Depth32),
				Flags::EARLY_FRAGMENT_TESTS | Flags::LATE_FRAGMENT_TESTS,
			),
			(Stages::COMPUTE, None, None, Flags::COMPUTE_SHADER),
			(Stages::COMPUTE, Some(Layouts::Indirect), None, Flags::DRAW_INDIRECT),
			(Stages::TRANSFER, None, None, Flags::TRANSFER),
			(Stages::PRESENTATION, None, None, Flags::ALL_COMMANDS),
			(Stages::RAYGEN, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::CLOSEST_HIT, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::ANY_HIT, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::INTERSECTION, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::MISS, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(Stages::CALLABLE, None, None, Flags::RAY_TRACING_SHADER_KHR),
			(
				Stages::ACCELERATION_STRUCTURE_BUILD,
				None,
				None,
				Flags::ACCELERATION_STRUCTURE_BUILD_KHR,
			),
		];
		for (stages, layout, format, expected) in cases {
			assert_eq!(
				to_pipeline_stage_flags(stages, layout, format),
				expected,
				"{stages:?} {layout:?} {format:?}"
			);
		}
	}

	#[test]
	fn test_to_access_flags() {
		use vk::AccessFlags2 as Flags;

		let (read, write) = (AccessPolicies::READ, AccessPolicies::WRITE);
		let (color, depth) = (Some(Formats::RGBA8UNORM), Some(Formats::Depth32));
		let cases = [
			(read, Stages::VERTEX, Layouts::Undefined, None, Flags::VERTEX_ATTRIBUTE_READ),
			(read, Stages::TRANSFER, Layouts::Undefined, None, Flags::TRANSFER_READ),
			(read, Stages::PRESENTATION, Layouts::Undefined, None, Flags::NONE),
			(
				read,
				Stages::FRAGMENT,
				Layouts::RenderTarget,
				color,
				Flags::COLOR_ATTACHMENT_READ,
			),
			(
				read,
				Stages::FRAGMENT,
				Layouts::RenderTarget,
				depth,
				Flags::DEPTH_STENCIL_ATTACHMENT_READ,
			),
			(read, Stages::FRAGMENT, Layouts::Read, color, Flags::SHADER_SAMPLED_READ),
			(read, Stages::FRAGMENT, Layouts::Read, depth, Flags::SHADER_SAMPLED_READ),
			(read, Stages::COMPUTE, Layouts::Indirect, None, Flags::INDIRECT_COMMAND_READ),
			(read, Stages::COMPUTE, Layouts::General, None, Flags::SHADER_READ),
			(
				read,
				Stages::RAYGEN,
				Layouts::ShaderBindingTable,
				None,
				Flags::SHADER_BINDING_TABLE_READ_KHR,
			),
			(
				read,
				Stages::RAYGEN,
				Layouts::General,
				None,
				Flags::ACCELERATION_STRUCTURE_READ_KHR,
			),
			(
				read,
				Stages::ACCELERATION_STRUCTURE_BUILD,
				Layouts::General,
				None,
				Flags::ACCELERATION_STRUCTURE_READ_KHR,
			),
			(write, Stages::TRANSFER, Layouts::Undefined, None, Flags::TRANSFER_WRITE),
			(write, Stages::COMPUTE, Layouts::General, None, Flags::SHADER_WRITE),
			(
				write,
				Stages::FRAGMENT,
				Layouts::RenderTarget,
				color,
				Flags::COLOR_ATTACHMENT_WRITE,
			),
			(
				AccessPolicies::READ_WRITE,
				Stages::FRAGMENT,
				Layouts::RenderTarget,
				color,
				Flags::COLOR_ATTACHMENT_READ | Flags::COLOR_ATTACHMENT_WRITE,
			),
			(
				write,
				Stages::FRAGMENT,
				Layouts::RenderTarget,
				depth,
				Flags::DEPTH_STENCIL_ATTACHMENT_WRITE,
			),
			(write, Stages::FRAGMENT, Layouts::General, color, Flags::SHADER_WRITE),
			(write, Stages::FRAGMENT, Layouts::General, depth, Flags::SHADER_WRITE),
			(write, Stages::RAYGEN, Layouts::General, None, Flags::SHADER_WRITE),
			(
				write,
				Stages::ACCELERATION_STRUCTURE_BUILD,
				Layouts::General,
				None,
				Flags::ACCELERATION_STRUCTURE_WRITE_KHR,
			),
		];
		for (accesses, stages, layout, format, expected) in cases {
			assert_eq!(
				to_access_flags(accesses, stages, layout, format),
				expected,
				"{accesses:?} {stages:?} {layout:?} {format:?}"
			);
		}
	}

	#[test]
	fn datatype_to_vk_format() {
		let cases = [
			(crate::DataTypes::U8, vk::Format::R8_UINT),
			(crate::DataTypes::U16, vk::Format::R16_UINT),
			(crate::DataTypes::U32, vk::Format::R32_UINT),
			(crate::DataTypes::Int, vk::Format::R32_SINT),
			(crate::DataTypes::Int2, vk::Format::R32G32_SINT),
			(crate::DataTypes::Int3, vk::Format::R32G32B32_SINT),
			(crate::DataTypes::Int4, vk::Format::R32G32B32A32_SINT),
			(crate::DataTypes::Float, vk::Format::R32_SFLOAT),
			(crate::DataTypes::Float2, vk::Format::R32G32_SFLOAT),
			(crate::DataTypes::Float3, vk::Format::R32G32B32_SFLOAT),
			(crate::DataTypes::Float4, vk::Format::R32G32B32A32_SFLOAT),
		];
		for (data_type, expected) in cases {
			assert_eq!(vk::Format::from(data_type), expected);
		}
	}
}
