use ash::vk;

use crate::{graphics_hardware_interface, Size};

pub(super) fn uses_to_vk_usage_flags(usage: graphics_hardware_interface::Uses) -> vk::BufferUsageFlags {
	let mut flags = vk::BufferUsageFlags::empty();
	flags |= if usage.contains(graphics_hardware_interface::Uses::Vertex) { vk::BufferUsageFlags::VERTEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Index) { vk::BufferUsageFlags::INDEX_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Uniform) { vk::BufferUsageFlags::UNIFORM_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Storage) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::TransferSource) { vk::BufferUsageFlags::TRANSFER_SRC } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::TransferDestination) { vk::BufferUsageFlags::TRANSFER_DST } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructure) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::Indirect) { vk::BufferUsageFlags::INDIRECT_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::ShaderBindingTable) { vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructureBuildScratch) { vk::BufferUsageFlags::STORAGE_BUFFER } else { vk::BufferUsageFlags::empty() };
	flags |= if usage.contains(graphics_hardware_interface::Uses::AccelerationStructureBuild) { vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR } else { vk::BufferUsageFlags::empty() };
	flags
}

pub(super) fn to_clear_value(clear: graphics_hardware_interface::ClearValue) -> vk::ClearValue {
	match clear {
		graphics_hardware_interface::ClearValue::None => vk::ClearValue::default(),
		graphics_hardware_interface::ClearValue::Color(clear) => vk::ClearValue { color: vk::ClearColorValue { float32: [clear.r, clear.g, clear.b, clear.a], }, },
		graphics_hardware_interface::ClearValue::Depth(clear) => vk::ClearValue { depth_stencil: vk::ClearDepthStencilValue { depth: clear, stencil: 0, }, },
		graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearValue { color: vk::ClearColorValue { uint32: [r, g, b, a], }, },
	}
}

pub(super) fn texture_format_and_resource_use_to_image_layout(texture_format: graphics_hardware_interface::Formats, layout: graphics_hardware_interface::Layouts, access: Option<graphics_hardware_interface::AccessPolicies>) -> vk::ImageLayout {
	match layout {
		graphics_hardware_interface::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		graphics_hardware_interface::Layouts::RenderTarget => if texture_format != graphics_hardware_interface::Formats::Depth32 { vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL } else { vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL },
		graphics_hardware_interface::Layouts::Transfer => {
			match access {
				Some(a) => {
					if a.intersects(graphics_hardware_interface::AccessPolicies::READ) {
						vk::ImageLayout::TRANSFER_SRC_OPTIMAL
					} else if a.intersects(graphics_hardware_interface::AccessPolicies::WRITE) {
						vk::ImageLayout::TRANSFER_DST_OPTIMAL
					} else {
						vk::ImageLayout::UNDEFINED
					}
				}
				None => vk::ImageLayout::UNDEFINED
			}
		}
		graphics_hardware_interface::Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		graphics_hardware_interface::Layouts::Read => {
			if texture_format != graphics_hardware_interface::Formats::Depth32 { vk::ImageLayout::READ_ONLY_OPTIMAL } else { vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL }
		},
		graphics_hardware_interface::Layouts::General => vk::ImageLayout::GENERAL,
		graphics_hardware_interface::Layouts::ShaderBindingTable => vk::ImageLayout::UNDEFINED,
		graphics_hardware_interface::Layouts::Indirect => vk::ImageLayout::UNDEFINED,
	}
}

pub(super) fn to_load_operation(value: bool) -> vk::AttachmentLoadOp {	if value { vk::AttachmentLoadOp::LOAD } else { vk::AttachmentLoadOp::CLEAR } }

pub(super) fn to_store_operation(value: bool) -> vk::AttachmentStoreOp { if value { vk::AttachmentStoreOp::STORE } else { vk::AttachmentStoreOp::DONT_CARE } }

pub(super) fn to_format(format: graphics_hardware_interface::Formats) -> vk::Format {
	match format {
		graphics_hardware_interface::Formats::R8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::R8_SRGB }
			}
		}
		graphics_hardware_interface::Formats::R16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::UNDEFINED }
			}
		}
		graphics_hardware_interface::Formats::R32(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R32_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R32_UINT }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R32_SINT }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::UNDEFINED }
			}
		}
		graphics_hardware_interface::Formats::RG8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::R8G8_SRGB }
			}
		}
		graphics_hardware_interface::Formats::RG16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::UNDEFINED }
			}
		}
		graphics_hardware_interface::Formats::RGB8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8B8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8B8_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::R8G8B8_SRGB }
			}
		}
		graphics_hardware_interface::Formats::RGB16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16B16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16B16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16B16_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::UNDEFINED }
			}
		}
		graphics_hardware_interface::Formats::RGBA8(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::UNDEFINED }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R8G8B8A8_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R8G8B8A8_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::R8G8B8A8_SRGB }
			}
		}
		graphics_hardware_interface::Formats::RGBA16(encoding) => {
			match encoding {
				graphics_hardware_interface::Encodings::FloatingPoint => { vk::Format::R16G16B16A16_SFLOAT }
				graphics_hardware_interface::Encodings::UnsignedNormalized => { vk::Format::R16G16B16A16_UNORM }
				graphics_hardware_interface::Encodings::SignedNormalized => { vk::Format::R16G16B16A16_SNORM }
				graphics_hardware_interface::Encodings::sRGB => { vk::Format::UNDEFINED }
			}
		}
		graphics_hardware_interface::Formats::RGBu11u11u10 => vk::Format::B10G11R11_UFLOAT_PACK32,
		graphics_hardware_interface::Formats::BGRAu8 => vk::Format::B8G8R8A8_SRGB,
		graphics_hardware_interface::Formats::Depth32 => vk::Format::D32_SFLOAT,
		graphics_hardware_interface::Formats::U32 => vk::Format::R32_UINT,
		graphics_hardware_interface::Formats::BC5 => vk::Format::BC5_UNORM_BLOCK,
		graphics_hardware_interface::Formats::BC7 => vk::Format::BC7_SRGB_BLOCK,
	}
}

pub(super) fn to_shader_stage_flags(shader_type: graphics_hardware_interface::ShaderTypes) -> vk::ShaderStageFlags {
	match shader_type {
		graphics_hardware_interface::ShaderTypes::Vertex => vk::ShaderStageFlags::VERTEX,
		graphics_hardware_interface::ShaderTypes::Fragment => vk::ShaderStageFlags::FRAGMENT,
		graphics_hardware_interface::ShaderTypes::Compute => vk::ShaderStageFlags::COMPUTE,
		graphics_hardware_interface::ShaderTypes::Task => vk::ShaderStageFlags::TASK_EXT,
		graphics_hardware_interface::ShaderTypes::Mesh => vk::ShaderStageFlags::MESH_EXT,
		graphics_hardware_interface::ShaderTypes::RayGen => vk::ShaderStageFlags::RAYGEN_KHR,
		graphics_hardware_interface::ShaderTypes::ClosestHit => vk::ShaderStageFlags::CLOSEST_HIT_KHR,
		graphics_hardware_interface::ShaderTypes::AnyHit => vk::ShaderStageFlags::ANY_HIT_KHR,
		graphics_hardware_interface::ShaderTypes::Intersection => vk::ShaderStageFlags::INTERSECTION_KHR,
		graphics_hardware_interface::ShaderTypes::Miss => vk::ShaderStageFlags::MISS_KHR,
		graphics_hardware_interface::ShaderTypes::Callable => vk::ShaderStageFlags::CALLABLE_KHR,
	}
}

pub(super) fn to_pipeline_stage_flags(stages: graphics_hardware_interface::Stages, layout: Option<graphics_hardware_interface::Layouts>, format: Option<graphics_hardware_interface::Formats>) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(graphics_hardware_interface::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT;
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER;
	}

	if stages.contains(graphics_hardware_interface::Stages::INDEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT;
		pipeline_stage_flags |= vk::PipelineStageFlags2::INDEX_INPUT;
	}

	if stages.contains(graphics_hardware_interface::Stages::MESH) { pipeline_stage_flags |= vk::PipelineStageFlags2::MESH_SHADER_EXT; }

	if stages.contains(graphics_hardware_interface::Stages::FRAGMENT) {
		if let Some(layout) = layout {
			if layout == graphics_hardware_interface::Layouts::Read {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}

			if layout == graphics_hardware_interface::Layouts::RenderTarget {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
			}

			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
				} else {
					pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
					pipeline_stage_flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
				}
			}
		} else {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
				} else {
					pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
					pipeline_stage_flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
				}
			} else {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}
		}
	}

	if stages.contains(graphics_hardware_interface::Stages::COMPUTE) {
		if let Some(layout) = layout {
			if layout == graphics_hardware_interface::Layouts::Indirect {
				pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT
			} else {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
			}
		} else {
			pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
		}
	}

	if stages.contains(graphics_hardware_interface::Stages::TRANSFER) { pipeline_stage_flags |= vk::PipelineStageFlags2::TRANSFER 	}
	if stages.contains(graphics_hardware_interface::Stages::PRESENTATION) { pipeline_stage_flags |= vk::PipelineStageFlags2::TOP_OF_PIPE }
	if stages.contains(graphics_hardware_interface::Stages::RAYGEN) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::CLOSEST_HIT) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::ANY_HIT) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::INTERSECTION) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::MISS) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::CALLABLE) { pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR; }
	if stages.contains(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) { pipeline_stage_flags |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR; }

	pipeline_stage_flags
}

pub(super) fn to_access_flags(accesses: graphics_hardware_interface::AccessPolicies, stages: graphics_hardware_interface::Stages, layout: graphics_hardware_interface::Layouts, format: Option<graphics_hardware_interface::Formats>) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::empty();

	if accesses.contains(graphics_hardware_interface::AccessPolicies::READ) {
		if stages.intersects(graphics_hardware_interface::Stages::VERTEX) {
			access_flags |= vk::AccessFlags2::VERTEX_ATTRIBUTE_READ;
		}
		if stages.intersects(graphics_hardware_interface::Stages::INDEX) {
			access_flags |= vk::AccessFlags2::VERTEX_ATTRIBUTE_READ;
			access_flags |= vk::AccessFlags2::INDEX_READ;
		}
		if stages.intersects(graphics_hardware_interface::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(graphics_hardware_interface::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(graphics_hardware_interface::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				} else {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::COMPUTE) {
			if layout == graphics_hardware_interface::Layouts::Indirect {
				access_flags |= vk::AccessFlags2::INDIRECT_COMMAND_READ
			} else {
				access_flags |= vk::AccessFlags2::SHADER_READ
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::RAYGEN) {
			if layout == graphics_hardware_interface::Layouts::ShaderBindingTable {
				access_flags |= vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR
			} else {
				access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
		}
	}

	if accesses.contains(graphics_hardware_interface::AccessPolicies::WRITE) {
		if stages.intersects(graphics_hardware_interface::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != graphics_hardware_interface::Formats::Depth32 {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				} else {
					if layout == graphics_hardware_interface::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
			}
		}
		if stages.intersects(graphics_hardware_interface::Stages::RAYGEN) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR
		}
	}

	access_flags
}

pub(super) fn image_type_from_extent(extent: vk::Extent3D) -> Option<vk::ImageType> {
	match extent {
		vk::Extent3D { width: 1.., height: 1, depth: 1 } => { Some(vk::ImageType::TYPE_1D) }
		vk::Extent3D { width: 1.., height: 1.., depth: 1 } => { Some(vk::ImageType::TYPE_2D) }
		vk::Extent3D { width: 1.., height: 1.., depth: 1.. } => { Some(vk::ImageType::TYPE_3D) }
		_ => { None }
	}
}

pub(super) fn into_vk_image_usage_flags(uses: graphics_hardware_interface::Uses, format: graphics_hardware_interface::Formats) -> vk::ImageUsageFlags {
	vk::ImageUsageFlags::empty()
	|
	if uses.intersects(graphics_hardware_interface::Uses::Image) { vk::ImageUsageFlags::SAMPLED } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::Clear) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::Storage) { vk::ImageUsageFlags::STORAGE } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::RenderTarget) && format != graphics_hardware_interface::Formats::Depth32 { vk::ImageUsageFlags::COLOR_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::DepthStencil) || format == graphics_hardware_interface::Formats::Depth32 { vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::TransferSource) { vk::ImageUsageFlags::TRANSFER_SRC } else { vk::ImageUsageFlags::empty() }
	|
	if uses.intersects(graphics_hardware_interface::Uses::TransferDestination) { vk::ImageUsageFlags::TRANSFER_DST } else { vk::ImageUsageFlags::empty() }
}

impl Into<vk::ShaderStageFlags> for graphics_hardware_interface::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::VERTEX) { vk::ShaderStageFlags::VERTEX } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::FRAGMENT) { vk::ShaderStageFlags::FRAGMENT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::COMPUTE) { vk::ShaderStageFlags::COMPUTE } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::MESH) { vk::ShaderStageFlags::MESH_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::TASK) { vk::ShaderStageFlags::TASK_EXT } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::RAYGEN) { vk::ShaderStageFlags::RAYGEN_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::CLOSEST_HIT) { vk::ShaderStageFlags::CLOSEST_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::ANY_HIT) { vk::ShaderStageFlags::ANY_HIT_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::INTERSECTION) { vk::ShaderStageFlags::INTERSECTION_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::MISS) { vk::ShaderStageFlags::MISS_KHR } else { vk::ShaderStageFlags::default() };
		shader_stage_flags |= if self.intersects(graphics_hardware_interface::Stages::CALLABLE) { vk::ShaderStageFlags::CALLABLE_KHR } else { vk::ShaderStageFlags::default() };

		shader_stage_flags
	}
}

impl Into<vk::Format> for graphics_hardware_interface::DataTypes {
	fn into(self) -> vk::Format {
		match self {
			graphics_hardware_interface::DataTypes::Float => vk::Format::R32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float2 => vk::Format::R32G32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float3 => vk::Format::R32G32B32_SFLOAT,
			graphics_hardware_interface::DataTypes::Float4 => vk::Format::R32G32B32A32_SFLOAT,
			graphics_hardware_interface::DataTypes::U8 => vk::Format::R8_UINT,
			graphics_hardware_interface::DataTypes::U16 => vk::Format::R16_UINT,
			graphics_hardware_interface::DataTypes::Int => vk::Format::R32_SINT,
			graphics_hardware_interface::DataTypes::U32 => vk::Format::R32_UINT,
			graphics_hardware_interface::DataTypes::Int2 => vk::Format::R32G32_SINT,
			graphics_hardware_interface::DataTypes::Int3 => vk::Format::R32G32B32_SINT,
			graphics_hardware_interface::DataTypes::Int4 => vk::Format::R32G32B32A32_SINT,
			graphics_hardware_interface::DataTypes::UInt => vk::Format::R32_UINT,
			graphics_hardware_interface::DataTypes::UInt2 => vk::Format::R32G32_UINT,
			graphics_hardware_interface::DataTypes::UInt3 => vk::Format::R32G32B32_UINT,
			graphics_hardware_interface::DataTypes::UInt4 => vk::Format::R32G32B32A32_UINT,
		}
	}
}

impl Size for graphics_hardware_interface::DataTypes {
	fn size(&self) -> usize {
		match self {
			graphics_hardware_interface::DataTypes::Float => std::mem::size_of::<f32>(),
			graphics_hardware_interface::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			graphics_hardware_interface::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			graphics_hardware_interface::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			graphics_hardware_interface::DataTypes::U8 => std::mem::size_of::<u8>(),
			graphics_hardware_interface::DataTypes::U16 => std::mem::size_of::<u16>(),
			graphics_hardware_interface::DataTypes::U32 => std::mem::size_of::<u32>(),
			graphics_hardware_interface::DataTypes::Int => std::mem::size_of::<i32>(),
			graphics_hardware_interface::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			graphics_hardware_interface::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			graphics_hardware_interface::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			graphics_hardware_interface::DataTypes::UInt => std::mem::size_of::<u32>(),
			graphics_hardware_interface::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			graphics_hardware_interface::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			graphics_hardware_interface::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}
}

impl Size for &[graphics_hardware_interface::VertexElement] {
	fn size(&self) -> usize {
		let mut size = 0;

		for element in *self {
			size += element.format.size();
		}

		size
	}
}

impl Into<graphics_hardware_interface::Stages> for graphics_hardware_interface::ShaderTypes {
	fn into(self) -> graphics_hardware_interface::Stages {
		match self {
			graphics_hardware_interface::ShaderTypes::Vertex => graphics_hardware_interface::Stages::VERTEX,
			graphics_hardware_interface::ShaderTypes::Fragment => graphics_hardware_interface::Stages::FRAGMENT,
			graphics_hardware_interface::ShaderTypes::Compute => graphics_hardware_interface::Stages::COMPUTE,
			graphics_hardware_interface::ShaderTypes::Task => graphics_hardware_interface::Stages::TASK,
			graphics_hardware_interface::ShaderTypes::Mesh => graphics_hardware_interface::Stages::MESH,
			graphics_hardware_interface::ShaderTypes::RayGen => graphics_hardware_interface::Stages::RAYGEN,
			graphics_hardware_interface::ShaderTypes::ClosestHit => graphics_hardware_interface::Stages::CLOSEST_HIT,
			graphics_hardware_interface::ShaderTypes::AnyHit => graphics_hardware_interface::Stages::ANY_HIT,
			graphics_hardware_interface::ShaderTypes::Intersection => graphics_hardware_interface::Stages::INTERSECTION,
			graphics_hardware_interface::ShaderTypes::Miss => graphics_hardware_interface::Stages::MISS,
			graphics_hardware_interface::ShaderTypes::Callable => graphics_hardware_interface::Stages::CALLABLE,
		}
	}
}

#[cfg(test)]
mod tests {
	use utils::RGBA;

	use super::*;

	#[test]
	fn test_uses_to_vk_usage_flags() {
		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Vertex);
		assert!(value.intersects(vk::BufferUsageFlags::VERTEX_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Index);
		assert!(value.intersects(vk::BufferUsageFlags::INDEX_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Uniform);
		assert!(value.intersects(vk::BufferUsageFlags::UNIFORM_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Storage);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::TransferSource);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_SRC));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::TransferDestination);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_DST));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructure);
		assert!(value.intersects(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::Indirect);
		assert!(value.intersects(vk::BufferUsageFlags::INDIRECT_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::ShaderBindingTable);
		assert!(value.intersects(vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructureBuildScratch);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(graphics_hardware_interface::Uses::AccelerationStructureBuild);
		assert!(value.intersects(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR));
	}

	#[test]
	fn test_to_clear_value() {
		let value = to_clear_value(graphics_hardware_interface::ClearValue::Color(RGBA::new(0.0, 1.0, 2.0, 3.0)));
		assert_eq!(unsafe { value.color.float32 }, [0.0, 1.0, 2.0, 3.0]);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Depth(0.0));
		assert_eq!(unsafe { value.depth_stencil.depth }, 0.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Depth(1.0));
		assert_eq!(unsafe { value.depth_stencil.depth }, 1.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::Integer(1, 2, 3, 4));
		assert_eq!(unsafe { value.color.int32 }, [1, 2, 3, 4]);

		let value = to_clear_value(graphics_hardware_interface::ClearValue::None);
		assert_eq!(unsafe { value.color.float32 }, [0.0, 0.0, 0.0, 0.0]);
		assert_eq!(unsafe { value.depth_stencil.depth }, 0.0);
		assert_eq!(unsafe { value.depth_stencil.stencil }, 0);
	}

	#[test]
	fn test_to_load_operation() {
		let value = to_load_operation(true);
		assert_eq!(value, vk::AttachmentLoadOp::LOAD);

		let value = to_load_operation(false);
		assert_eq!(value, vk::AttachmentLoadOp::CLEAR);
	}

	#[test]
	fn test_to_store_operation() {
		let value = to_store_operation(true);
		assert_eq!(value, vk::AttachmentStoreOp::STORE);

		let value = to_store_operation(false);
		assert_eq!(value, vk::AttachmentStoreOp::DONT_CARE);
	}

	#[test]
	fn test_texture_format_and_resource_use_to_image_layout() {
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, Some(graphics_hardware_interface::AccessPolicies::READ));
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Undefined, Some(graphics_hardware_interface::AccessPolicies::WRITE));
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::Depth32, graphics_hardware_interface::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, Some(graphics_hardware_interface::AccessPolicies::READ));
		assert_eq!(value, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Transfer, Some(graphics_hardware_interface::AccessPolicies::WRITE));
		assert_eq!(value, vk::ImageLayout::TRANSFER_DST_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Present, None);
		assert_eq!(value, vk::ImageLayout::PRESENT_SRC_KHR);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::READ_ONLY_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::Depth32, graphics_hardware_interface::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::ImageLayout::GENERAL);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::ShaderBindingTable, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value = texture_format_and_resource_use_to_image_layout(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized), graphics_hardware_interface::Layouts::Indirect, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
	}

	#[test]
	fn test_to_format() {
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::R8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::R16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R32_UINT);
		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R32_SINT);
		let value = to_format(graphics_hardware_interface::Formats::R32(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8G8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::BC5);
		assert_eq!(value, vk::Format::BC5_UNORM_BLOCK);
		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8G8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RG16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGB16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16B16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R8G8B8A8_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::BC7);
		assert_eq!(value, vk::Format::BC7_SRGB_BLOCK);
		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R8G8B8A8_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::UnsignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16A16_UNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::SignedNormalized));
		assert_eq!(value, vk::Format::R16G16B16A16_SNORM);
		let value = to_format(graphics_hardware_interface::Formats::RGBA16(graphics_hardware_interface::Encodings::FloatingPoint));
		assert_eq!(value, vk::Format::R16G16B16A16_SFLOAT);

		let value = to_format(graphics_hardware_interface::Formats::BGRAu8);
		assert_eq!(value, vk::Format::B8G8R8A8_SRGB);

		let value = to_format(graphics_hardware_interface::Formats::RGBu11u11u10);
		assert_eq!(value, vk::Format::B10G11R11_UFLOAT_PACK32);

		let value = to_format(graphics_hardware_interface::Formats::Depth32);
		assert_eq!(value, vk::Format::D32_SFLOAT);
	}

	#[test]
	fn test_to_shader_stage_flags() {
		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Vertex);
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Fragment);
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Compute);
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Task);
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Mesh);
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::RayGen);
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::ClosestHit);
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::AnyHit);
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Intersection);
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Miss);
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value = to_shader_stage_flags(graphics_hardware_interface::ShaderTypes::Callable);
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);
	}

	#[test]
	fn test_to_pipeline_stage_flags() {
		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::NONE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::NONE);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::VERTEX, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::VERTEX_SHADER | vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::MESH, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::MESH_SHADER_EXT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::FRAGMENT_SHADER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, Some(graphics_hardware_interface::Layouts::RenderTarget), None);
		assert_eq!(value, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::FRAGMENT, None, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::COMPUTE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::COMPUTE_SHADER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::COMPUTE, Some(graphics_hardware_interface::Layouts::Indirect), None);
		assert_eq!(value, vk::PipelineStageFlags2::DRAW_INDIRECT);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::TRANSFER, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TRANSFER);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::PRESENTATION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TOP_OF_PIPE);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::RAYGEN, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::CLOSEST_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::ANY_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::INTERSECTION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::MISS, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::CALLABLE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR);
	}

	#[test]
	fn test_to_access_flags() {
		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::VERTEX, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::VERTEX_ATTRIBUTE_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::TRANSFER, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::PRESENTATION, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::NONE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::Read, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::Read, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::Indirect, None);
		assert_eq!(value, vk::AccessFlags2::INDIRECT_COMMAND_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_READ);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::ShaderBindingTable, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::READ, graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::TRANSFER, graphics_hardware_interface::Layouts::Undefined, None);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::COMPUTE, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::RenderTarget, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::General, Some(graphics_hardware_interface::Formats::RGBA8(graphics_hardware_interface::Encodings::UnsignedNormalized)));
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::FRAGMENT, graphics_hardware_interface::Layouts::General, Some(graphics_hardware_interface::Formats::Depth32));
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::RAYGEN, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(graphics_hardware_interface::AccessPolicies::WRITE, graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD, graphics_hardware_interface::Layouts::General, None);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR);
	}

	#[test]
	fn stages_to_vk_shader_stage_flags() {
		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::VERTEX.into();
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::FRAGMENT.into();
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::COMPUTE.into();
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::MESH.into();
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::TASK.into();
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::RAYGEN.into();
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::CLOSEST_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::ANY_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::INTERSECTION.into();
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::MISS.into();
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::CALLABLE.into();
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::ACCELERATION_STRUCTURE_BUILD.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::TRANSFER.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::PRESENTATION.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = graphics_hardware_interface::Stages::NONE.into();
		assert_eq!(value, vk::ShaderStageFlags::default());
	}

	#[test]
	fn datatype_to_vk_format() {
		let value: vk::Format = graphics_hardware_interface::DataTypes::U8.into();
		assert_eq!(value, vk::Format::R8_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::U16.into();
		assert_eq!(value, vk::Format::R16_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::U32.into();
		assert_eq!(value, vk::Format::R32_UINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int.into();
		assert_eq!(value, vk::Format::R32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int2.into();
		assert_eq!(value, vk::Format::R32G32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int3.into();
		assert_eq!(value, vk::Format::R32G32B32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Int4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SINT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float.into();
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float2.into();
		assert_eq!(value, vk::Format::R32G32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float3.into();
		assert_eq!(value, vk::Format::R32G32B32_SFLOAT);

		let value: vk::Format = graphics_hardware_interface::DataTypes::Float4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SFLOAT);
	}

	#[test]
	fn datatype_size() {
		let value = graphics_hardware_interface::DataTypes::U8.size();
		assert_eq!(value, 1);

		let value = graphics_hardware_interface::DataTypes::U16.size();
		assert_eq!(value, 2);

		let value = graphics_hardware_interface::DataTypes::U32.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Int.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Int2.size();
		assert_eq!(value, 8);

		let value = graphics_hardware_interface::DataTypes::Int3.size();
		assert_eq!(value, 12);

		let value = graphics_hardware_interface::DataTypes::Int4.size();
		assert_eq!(value, 16);

		let value = graphics_hardware_interface::DataTypes::Float.size();
		assert_eq!(value, 4);

		let value = graphics_hardware_interface::DataTypes::Float2.size();
		assert_eq!(value, 8);

		let value = graphics_hardware_interface::DataTypes::Float3.size();
		assert_eq!(value, 12);

		let value = graphics_hardware_interface::DataTypes::Float4.size();
		assert_eq!(value, 16);
	}

	#[test]
	fn shader_types_to_stages() {
		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Vertex.into();
		assert_eq!(value, graphics_hardware_interface::Stages::VERTEX);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Fragment.into();
		assert_eq!(value, graphics_hardware_interface::Stages::FRAGMENT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Compute.into();
		assert_eq!(value, graphics_hardware_interface::Stages::COMPUTE);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Task.into();
		assert_eq!(value, graphics_hardware_interface::Stages::TASK);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Mesh.into();
		assert_eq!(value, graphics_hardware_interface::Stages::MESH);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::RayGen.into();
		assert_eq!(value, graphics_hardware_interface::Stages::RAYGEN);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::ClosestHit.into();
		assert_eq!(value, graphics_hardware_interface::Stages::CLOSEST_HIT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::AnyHit.into();
		assert_eq!(value, graphics_hardware_interface::Stages::ANY_HIT);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Intersection.into();
		assert_eq!(value, graphics_hardware_interface::Stages::INTERSECTION);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Miss.into();
		assert_eq!(value, graphics_hardware_interface::Stages::MISS);

		let value: graphics_hardware_interface::Stages = graphics_hardware_interface::ShaderTypes::Callable.into();
		assert_eq!(value, graphics_hardware_interface::Stages::CALLABLE);
	}

	#[test]
	fn test_image_type_from_extent() {
		let value = image_type_from_extent(vk::Extent3D { width: 1, height: 1, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 1, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 2, depth: 1 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_2D);

		let value = image_type_from_extent(vk::Extent3D { width: 2, height: 2, depth: 2 }).expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_3D);
	}
}
