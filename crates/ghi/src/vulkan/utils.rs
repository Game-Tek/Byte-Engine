use ash::vk;

use crate::{graphics_hardware_interface, Size};

pub(super) fn uses_to_vk_usage_flags(usage: crate::Uses) -> vk::BufferUsageFlags {
	let mut flags = vk::BufferUsageFlags::empty();
	flags |= if usage.contains(crate::Uses::Vertex) {
		vk::BufferUsageFlags::VERTEX_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::Index) {
		vk::BufferUsageFlags::INDEX_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::Uniform) {
		vk::BufferUsageFlags::UNIFORM_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::Storage) {
		vk::BufferUsageFlags::STORAGE_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::TransferSource) {
		vk::BufferUsageFlags::TRANSFER_SRC
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::TransferDestination) {
		vk::BufferUsageFlags::TRANSFER_DST
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::AccelerationStructure) {
		vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::Indirect) {
		vk::BufferUsageFlags::INDIRECT_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::ShaderBindingTable) {
		vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::AccelerationStructureBuildScratch) {
		vk::BufferUsageFlags::STORAGE_BUFFER
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags |= if usage.contains(crate::Uses::AccelerationStructureBuild) {
		vk::BufferUsageFlags::ACCELERATION_STRUCTURE_BUILD_INPUT_READ_ONLY_KHR
	} else {
		vk::BufferUsageFlags::empty()
	};
	flags
}

pub(super) fn to_clear_value(clear: graphics_hardware_interface::ClearValue) -> vk::ClearValue {
	match clear {
		graphics_hardware_interface::ClearValue::None => vk::ClearValue::default(),
		graphics_hardware_interface::ClearValue::Color(clear) => vk::ClearValue {
			color: vk::ClearColorValue {
				float32: [clear.r, clear.g, clear.b, clear.a],
			},
		},
		graphics_hardware_interface::ClearValue::Depth(clear) => vk::ClearValue {
			depth_stencil: vk::ClearDepthStencilValue {
				depth: clear,
				stencil: 0,
			},
		},
		graphics_hardware_interface::ClearValue::Integer(r, g, b, a) => vk::ClearValue {
			color: vk::ClearColorValue { uint32: [r, g, b, a] },
		},
	}
}

pub(super) fn texture_format_and_resource_use_to_image_layout(
	texture_format: crate::Formats,
	layout: crate::Layouts,
	access: Option<crate::AccessPolicies>,
) -> vk::ImageLayout {
	match layout {
		crate::Layouts::Undefined => vk::ImageLayout::UNDEFINED,
		crate::Layouts::RenderTarget => {
			if texture_format != crate::Formats::Depth32 {
				vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL
			} else {
				vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL
			}
		}
		crate::Layouts::Transfer => match access {
			Some(a) => {
				if a.intersects(crate::AccessPolicies::READ) {
					vk::ImageLayout::TRANSFER_SRC_OPTIMAL
				} else if a.intersects(crate::AccessPolicies::WRITE) {
					vk::ImageLayout::TRANSFER_DST_OPTIMAL
				} else {
					vk::ImageLayout::UNDEFINED
				}
			}
			None => vk::ImageLayout::UNDEFINED,
		},
		crate::Layouts::Present => vk::ImageLayout::PRESENT_SRC_KHR,
		crate::Layouts::Read => {
			if texture_format != crate::Formats::Depth32 {
				vk::ImageLayout::READ_ONLY_OPTIMAL
			} else {
				vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL
			}
		}
		crate::Layouts::General => vk::ImageLayout::GENERAL,
		crate::Layouts::ShaderBindingTable => vk::ImageLayout::UNDEFINED,
		crate::Layouts::Indirect => vk::ImageLayout::UNDEFINED,
	}
}

pub(super) fn to_load_operation(value: bool) -> vk::AttachmentLoadOp {
	if value {
		vk::AttachmentLoadOp::LOAD
	} else {
		vk::AttachmentLoadOp::CLEAR
	}
}

pub(super) fn to_store_operation(value: bool) -> vk::AttachmentStoreOp {
	if value {
		vk::AttachmentStoreOp::STORE
	} else {
		vk::AttachmentStoreOp::DONT_CARE
	}
}

pub(super) fn to_format(format: crate::Formats) -> vk::Format {
	match format {
		crate::Formats::R8F => vk::Format::UNDEFINED,
		crate::Formats::R8UNORM => vk::Format::R8_UNORM,
		crate::Formats::R8SNORM => vk::Format::R8_SNORM,
		crate::Formats::R8sRGB => vk::Format::R8_SRGB,
		crate::Formats::R16F => vk::Format::R16_SFLOAT,
		crate::Formats::R16UNORM => vk::Format::R16_UNORM,
		crate::Formats::R16SNORM => vk::Format::R16_SNORM,
		crate::Formats::R16sRGB => vk::Format::UNDEFINED,
		crate::Formats::R32F => vk::Format::R32_SFLOAT,
		crate::Formats::R32UNORM => vk::Format::R32_UINT,
		crate::Formats::R32SNORM => vk::Format::R32_SINT,
		crate::Formats::R32sRGB => vk::Format::UNDEFINED,
		crate::Formats::RG8F => vk::Format::UNDEFINED,
		crate::Formats::RG8UNORM => vk::Format::R8G8_UNORM,
		crate::Formats::RG8SNORM => vk::Format::R8G8_SNORM,
		crate::Formats::RG8sRGB => vk::Format::R8G8_SRGB,
		crate::Formats::RG16F => vk::Format::R16G16_SFLOAT,
		crate::Formats::RG16UNORM => vk::Format::R16G16_UNORM,
		crate::Formats::RG16SNORM => vk::Format::R16G16_SNORM,
		crate::Formats::RG16sRGB => vk::Format::UNDEFINED,
		crate::Formats::RGB8F => vk::Format::UNDEFINED,
		crate::Formats::RGB8UNORM => vk::Format::R8G8B8_UNORM,
		crate::Formats::RGB8SNORM => vk::Format::R8G8B8_SNORM,
		crate::Formats::RGB8sRGB => vk::Format::R8G8B8_SRGB,
		crate::Formats::RGB16F => vk::Format::R16G16B16_SFLOAT,
		crate::Formats::RGB16UNORM => vk::Format::R16G16B16_UNORM,
		crate::Formats::RGB16SNORM => vk::Format::R16G16B16_SNORM,
		crate::Formats::RGB16sRGB => vk::Format::UNDEFINED,
		crate::Formats::RGBA8F => vk::Format::UNDEFINED,
		crate::Formats::RGBA8UNORM => vk::Format::R8G8B8A8_UNORM,
		crate::Formats::RGBA8SNORM => vk::Format::R8G8B8A8_SNORM,
		crate::Formats::RGBA8sRGB => vk::Format::R8G8B8A8_SRGB,
		crate::Formats::RGBA16F => vk::Format::R16G16B16A16_SFLOAT,
		crate::Formats::RGBA16UNORM => vk::Format::R16G16B16A16_UNORM,
		crate::Formats::RGBA16SNORM => vk::Format::R16G16B16A16_SNORM,
		crate::Formats::RGBA16sRGB => vk::Format::UNDEFINED,
		crate::Formats::RGBu11u11u10 => vk::Format::B10G11R11_UFLOAT_PACK32,
		crate::Formats::BGRAu8 => vk::Format::B8G8R8A8_UNORM,
		crate::Formats::BGRAsRGB => vk::Format::B8G8R8A8_SRGB,
		crate::Formats::Depth32 => vk::Format::D32_SFLOAT,
		crate::Formats::U32 => vk::Format::R32_UINT,
		crate::Formats::BC5 => vk::Format::BC5_UNORM_BLOCK,
		crate::Formats::BC7 => vk::Format::BC7_SRGB_BLOCK,
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
	stages: crate::Stages,
	layout: Option<crate::Layouts>,
	format: Option<crate::Formats>,
) -> vk::PipelineStageFlags2 {
	let mut pipeline_stage_flags = vk::PipelineStageFlags2::NONE;

	if stages.contains(crate::Stages::VERTEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT;
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_SHADER;
	}

	if stages.contains(crate::Stages::INDEX) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT;
		pipeline_stage_flags |= vk::PipelineStageFlags2::INDEX_INPUT;
	}

	if stages.contains(crate::Stages::MESH) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::MESH_SHADER_EXT;
	}

	if stages.contains(crate::Stages::FRAGMENT) {
		if let Some(layout) = layout {
			if layout == crate::Layouts::Read {
				pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
			}

			if layout == crate::Layouts::RenderTarget {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT
			}

			if let Some(format) = format {
				if format != crate::Formats::Depth32 {
					pipeline_stage_flags |= vk::PipelineStageFlags2::FRAGMENT_SHADER
				} else {
					pipeline_stage_flags |= vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS;
					pipeline_stage_flags |= vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS;
				}
			}
		} else {
			if let Some(format) = format {
				if format != crate::Formats::Depth32 {
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

	if stages.contains(crate::Stages::COMPUTE) {
		if let Some(layout) = layout {
			if layout == crate::Layouts::Indirect {
				pipeline_stage_flags |= vk::PipelineStageFlags2::DRAW_INDIRECT
			} else {
				pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
			}
		} else {
			pipeline_stage_flags |= vk::PipelineStageFlags2::COMPUTE_SHADER
		}
	}

	if stages.contains(crate::Stages::TRANSFER) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::TRANSFER
	}
	if stages.contains(crate::Stages::PRESENTATION) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::TOP_OF_PIPE
	}
	if stages.contains(crate::Stages::RAYGEN) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::CLOSEST_HIT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::ANY_HIT) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::INTERSECTION) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::MISS) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::CALLABLE) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR;
	}
	if stages.contains(crate::Stages::ACCELERATION_STRUCTURE_BUILD) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR;
	}
	if stages.contains(crate::Stages::LAST) {
		pipeline_stage_flags |= vk::PipelineStageFlags2::BOTTOM_OF_PIPE;
	}

	pipeline_stage_flags
}

pub(super) fn to_access_flags(
	accesses: crate::AccessPolicies,
	stages: crate::Stages,
	layout: crate::Layouts,
	format: Option<crate::Formats>,
) -> vk::AccessFlags2 {
	let mut access_flags = vk::AccessFlags2::NONE;

	if accesses.contains(crate::AccessPolicies::READ) {
		if stages.intersects(crate::Stages::VERTEX) {
			access_flags |= vk::AccessFlags2::VERTEX_ATTRIBUTE_READ;
		}
		if stages.intersects(crate::Stages::INDEX) {
			access_flags |= vk::AccessFlags2::VERTEX_ATTRIBUTE_READ;
			access_flags |= vk::AccessFlags2::INDEX_READ;
		}
		if stages.intersects(crate::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_READ
		}
		if stages.intersects(crate::Stages::PRESENTATION) {
			access_flags |= vk::AccessFlags2::NONE
		}
		if stages.intersects(crate::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != crate::Formats::Depth32 {
					if layout == crate::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				} else {
					if layout == crate::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ
					} else {
						access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::SHADER_SAMPLED_READ
			}
		}
		if stages.intersects(crate::Stages::COMPUTE) {
			if layout == crate::Layouts::Indirect {
				access_flags |= vk::AccessFlags2::INDIRECT_COMMAND_READ
			} else {
				access_flags |= vk::AccessFlags2::SHADER_READ
			}
		}
		if stages.intersects(crate::Stages::RAYGEN) {
			if layout == crate::Layouts::ShaderBindingTable {
				access_flags |= vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR
			} else {
				access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
			}
		}
		if stages.intersects(crate::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR
		}
	}

	if accesses.contains(crate::AccessPolicies::WRITE) {
		if stages.intersects(crate::Stages::TRANSFER) {
			access_flags |= vk::AccessFlags2::TRANSFER_WRITE
		}
		if stages.intersects(crate::Stages::COMPUTE) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(crate::Stages::FRAGMENT) {
			if let Some(format) = format {
				if format != crate::Formats::Depth32 {
					if layout == crate::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				} else {
					if layout == crate::Layouts::RenderTarget {
						access_flags |= vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE
					} else {
						access_flags |= vk::AccessFlags2::SHADER_WRITE
					}
				}
			} else {
				access_flags |= vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
			}
		}
		if stages.intersects(crate::Stages::RAYGEN) {
			access_flags |= vk::AccessFlags2::SHADER_WRITE
		}
		if stages.intersects(crate::Stages::ACCELERATION_STRUCTURE_BUILD) {
			access_flags |= vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR
		}
	}

	access_flags
}

pub(super) fn image_type_from_extent(extent: vk::Extent3D) -> Option<vk::ImageType> {
	match extent {
		vk::Extent3D {
			width: 1..,
			height: 1,
			depth: 1,
		} => Some(vk::ImageType::TYPE_1D),
		vk::Extent3D {
			width: 1..,
			height: 1..,
			depth: 1,
		} => Some(vk::ImageType::TYPE_2D),
		vk::Extent3D {
			width: 1..,
			height: 1..,
			depth: 1..,
		} => Some(vk::ImageType::TYPE_3D),
		_ => None,
	}
}

pub(super) fn into_vk_image_usage_flags(uses: crate::Uses, format: crate::Formats) -> vk::ImageUsageFlags {
	vk::ImageUsageFlags::empty()
		| if uses.intersects(crate::Uses::Image) {
			vk::ImageUsageFlags::SAMPLED
		} else {
			vk::ImageUsageFlags::empty()
		} | if uses.intersects(crate::Uses::InputAttachment) {
		vk::ImageUsageFlags::INPUT_ATTACHMENT
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::Clear) {
		vk::ImageUsageFlags::TRANSFER_DST
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::Storage) {
		vk::ImageUsageFlags::STORAGE
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::RenderTarget) && format != crate::Formats::Depth32 {
		vk::ImageUsageFlags::COLOR_ATTACHMENT
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::DepthStencil) || format == crate::Formats::Depth32 {
		vk::ImageUsageFlags::DEPTH_STENCIL_ATTACHMENT
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::TransferSource) {
		vk::ImageUsageFlags::TRANSFER_SRC
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::TransferDestination) {
		vk::ImageUsageFlags::TRANSFER_DST
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::BlitDestination) {
		vk::ImageUsageFlags::COLOR_ATTACHMENT
	} else {
		vk::ImageUsageFlags::empty()
	} | if uses.intersects(crate::Uses::BlitSource) {
		vk::ImageUsageFlags::SAMPLED
	} else {
		vk::ImageUsageFlags::empty()
	}
}

impl Into<vk::ShaderStageFlags> for crate::Stages {
	fn into(self) -> vk::ShaderStageFlags {
		let mut shader_stage_flags = vk::ShaderStageFlags::default();

		shader_stage_flags |= if self.intersects(crate::Stages::VERTEX) {
			vk::ShaderStageFlags::VERTEX
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::FRAGMENT) {
			vk::ShaderStageFlags::FRAGMENT
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::COMPUTE) {
			vk::ShaderStageFlags::COMPUTE
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::MESH) {
			vk::ShaderStageFlags::MESH_EXT
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::TASK) {
			vk::ShaderStageFlags::TASK_EXT
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::RAYGEN) {
			vk::ShaderStageFlags::RAYGEN_KHR
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::CLOSEST_HIT) {
			vk::ShaderStageFlags::CLOSEST_HIT_KHR
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::ANY_HIT) {
			vk::ShaderStageFlags::ANY_HIT_KHR
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::INTERSECTION) {
			vk::ShaderStageFlags::INTERSECTION_KHR
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::MISS) {
			vk::ShaderStageFlags::MISS_KHR
		} else {
			vk::ShaderStageFlags::default()
		};
		shader_stage_flags |= if self.intersects(crate::Stages::CALLABLE) {
			vk::ShaderStageFlags::CALLABLE_KHR
		} else {
			vk::ShaderStageFlags::default()
		};

		shader_stage_flags
	}
}

impl Into<vk::Format> for crate::DataTypes {
	fn into(self) -> vk::Format {
		match self {
			crate::DataTypes::Float => vk::Format::R32_SFLOAT,
			crate::DataTypes::Float2 => vk::Format::R32G32_SFLOAT,
			crate::DataTypes::Float3 => vk::Format::R32G32B32_SFLOAT,
			crate::DataTypes::Float4 => vk::Format::R32G32B32A32_SFLOAT,
			crate::DataTypes::U8 => vk::Format::R8_UINT,
			crate::DataTypes::U16 => vk::Format::R16_UINT,
			crate::DataTypes::Int => vk::Format::R32_SINT,
			crate::DataTypes::U32 => vk::Format::R32_UINT,
			crate::DataTypes::Int2 => vk::Format::R32G32_SINT,
			crate::DataTypes::Int3 => vk::Format::R32G32B32_SINT,
			crate::DataTypes::Int4 => vk::Format::R32G32B32A32_SINT,
			crate::DataTypes::UInt => vk::Format::R32_UINT,
			crate::DataTypes::UInt2 => vk::Format::R32G32_UINT,
			crate::DataTypes::UInt3 => vk::Format::R32G32B32_UINT,
			crate::DataTypes::UInt4 => vk::Format::R32G32B32A32_UINT,
		}
	}
}

impl Size for crate::DataTypes {
	fn size(&self) -> usize {
		match self {
			crate::DataTypes::Float => std::mem::size_of::<f32>(),
			crate::DataTypes::Float2 => std::mem::size_of::<f32>() * 2,
			crate::DataTypes::Float3 => std::mem::size_of::<f32>() * 3,
			crate::DataTypes::Float4 => std::mem::size_of::<f32>() * 4,
			crate::DataTypes::U8 => std::mem::size_of::<u8>(),
			crate::DataTypes::U16 => std::mem::size_of::<u16>(),
			crate::DataTypes::U32 => std::mem::size_of::<u32>(),
			crate::DataTypes::Int => std::mem::size_of::<i32>(),
			crate::DataTypes::Int2 => std::mem::size_of::<i32>() * 2,
			crate::DataTypes::Int3 => std::mem::size_of::<i32>() * 3,
			crate::DataTypes::Int4 => std::mem::size_of::<i32>() * 4,
			crate::DataTypes::UInt => std::mem::size_of::<u32>(),
			crate::DataTypes::UInt2 => std::mem::size_of::<u32>() * 2,
			crate::DataTypes::UInt3 => std::mem::size_of::<u32>() * 3,
			crate::DataTypes::UInt4 => std::mem::size_of::<u32>() * 4,
		}
	}
}

impl Size for &[crate::pipelines::VertexElement<'_>] {
	fn size(&self) -> usize {
		let mut size = 0;

		for element in *self {
			size += element.format.size();
		}

		size
	}
}

impl Into<crate::Stages> for crate::ShaderTypes {
	fn into(self) -> crate::Stages {
		match self {
			crate::ShaderTypes::Vertex => crate::Stages::VERTEX,
			crate::ShaderTypes::Fragment => crate::Stages::FRAGMENT,
			crate::ShaderTypes::Compute => crate::Stages::COMPUTE,
			crate::ShaderTypes::Task => crate::Stages::TASK,
			crate::ShaderTypes::Mesh => crate::Stages::MESH,
			crate::ShaderTypes::RayGen => crate::Stages::RAYGEN,
			crate::ShaderTypes::ClosestHit => crate::Stages::CLOSEST_HIT,
			crate::ShaderTypes::AnyHit => crate::Stages::ANY_HIT,
			crate::ShaderTypes::Intersection => crate::Stages::INTERSECTION,
			crate::ShaderTypes::Miss => crate::Stages::MISS,
			crate::ShaderTypes::Callable => crate::Stages::CALLABLE,
		}
	}
}

#[cfg(test)]
mod tests {
	use utils::RGBA;

	use super::*;

	#[test]
	fn test_uses_to_vk_usage_flags() {
		let value = uses_to_vk_usage_flags(crate::Uses::Vertex);
		assert!(value.intersects(vk::BufferUsageFlags::VERTEX_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::Index);
		assert!(value.intersects(vk::BufferUsageFlags::INDEX_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::Uniform);
		assert!(value.intersects(vk::BufferUsageFlags::UNIFORM_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::Storage);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::TransferSource);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_SRC));

		let value = uses_to_vk_usage_flags(crate::Uses::TransferDestination);
		assert!(value.intersects(vk::BufferUsageFlags::TRANSFER_DST));

		let value = uses_to_vk_usage_flags(crate::Uses::AccelerationStructure);
		assert!(value.intersects(vk::BufferUsageFlags::ACCELERATION_STRUCTURE_STORAGE_KHR));

		let value = uses_to_vk_usage_flags(crate::Uses::Indirect);
		assert!(value.intersects(vk::BufferUsageFlags::INDIRECT_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::ShaderBindingTable);
		assert!(value.intersects(vk::BufferUsageFlags::SHADER_BINDING_TABLE_KHR));

		let value = uses_to_vk_usage_flags(crate::Uses::AccelerationStructureBuildScratch);
		assert!(value.intersects(vk::BufferUsageFlags::STORAGE_BUFFER));

		let value = uses_to_vk_usage_flags(crate::Uses::AccelerationStructureBuild);
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
		let value =
			texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::Undefined, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(
			crate::Formats::RGBA8UNORM,
			crate::Layouts::Undefined,
			Some(crate::AccessPolicies::READ),
		);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(
			crate::Formats::RGBA8UNORM,
			crate::Layouts::Undefined,
			Some(crate::AccessPolicies::WRITE),
		);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value =
			texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL);
		let value =
			texture_format_and_resource_use_to_image_layout(crate::Formats::Depth32, crate::Layouts::RenderTarget, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_STENCIL_ATTACHMENT_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::Transfer, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
		let value = texture_format_and_resource_use_to_image_layout(
			crate::Formats::RGBA8UNORM,
			crate::Layouts::Transfer,
			Some(crate::AccessPolicies::READ),
		);
		assert_eq!(value, vk::ImageLayout::TRANSFER_SRC_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(
			crate::Formats::RGBA8UNORM,
			crate::Layouts::Transfer,
			Some(crate::AccessPolicies::WRITE),
		);
		assert_eq!(value, vk::ImageLayout::TRANSFER_DST_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::Present, None);
		assert_eq!(value, vk::ImageLayout::PRESENT_SRC_KHR);

		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::READ_ONLY_OPTIMAL);
		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::Depth32, crate::Layouts::Read, None);
		assert_eq!(value, vk::ImageLayout::DEPTH_READ_ONLY_OPTIMAL);

		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::General, None);
		assert_eq!(value, vk::ImageLayout::GENERAL);

		let value = texture_format_and_resource_use_to_image_layout(
			crate::Formats::RGBA8UNORM,
			crate::Layouts::ShaderBindingTable,
			None,
		);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);

		let value = texture_format_and_resource_use_to_image_layout(crate::Formats::RGBA8UNORM, crate::Layouts::Indirect, None);
		assert_eq!(value, vk::ImageLayout::UNDEFINED);
	}

	#[test]
	fn test_to_format() {
		let value = to_format(crate::Formats::R8UNORM);
		assert_eq!(value, vk::Format::R8_UNORM);
		let value = to_format(crate::Formats::R8SNORM);
		assert_eq!(value, vk::Format::R8_SNORM);
		let value = to_format(crate::Formats::R8F);
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(crate::Formats::R16UNORM);
		assert_eq!(value, vk::Format::R16_UNORM);
		let value = to_format(crate::Formats::R16SNORM);
		assert_eq!(value, vk::Format::R16_SNORM);
		let value = to_format(crate::Formats::R16F);
		assert_eq!(value, vk::Format::R16_SFLOAT);

		let value = to_format(crate::Formats::R32UNORM);
		assert_eq!(value, vk::Format::R32_UINT);
		let value = to_format(crate::Formats::R32SNORM);
		assert_eq!(value, vk::Format::R32_SINT);
		let value = to_format(crate::Formats::R32F);
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value = to_format(crate::Formats::RG8UNORM);
		assert_eq!(value, vk::Format::R8G8_UNORM);
		let value = to_format(crate::Formats::BC5);
		assert_eq!(value, vk::Format::BC5_UNORM_BLOCK);
		let value = to_format(crate::Formats::RG8SNORM);
		assert_eq!(value, vk::Format::R8G8_SNORM);
		let value = to_format(crate::Formats::RG8F);
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(crate::Formats::RG16UNORM);
		assert_eq!(value, vk::Format::R16G16_UNORM);
		let value = to_format(crate::Formats::RG16SNORM);
		assert_eq!(value, vk::Format::R16G16_SNORM);
		let value = to_format(crate::Formats::RG16F);
		assert_eq!(value, vk::Format::R16G16_SFLOAT);

		let value = to_format(crate::Formats::RGB16UNORM);
		assert_eq!(value, vk::Format::R16G16B16_UNORM);
		let value = to_format(crate::Formats::RGB16SNORM);
		assert_eq!(value, vk::Format::R16G16B16_SNORM);
		let value = to_format(crate::Formats::RGB16F);
		assert_eq!(value, vk::Format::R16G16B16_SFLOAT);

		let value = to_format(crate::Formats::RGBA8UNORM);
		assert_eq!(value, vk::Format::R8G8B8A8_UNORM);
		let value = to_format(crate::Formats::BC7);
		assert_eq!(value, vk::Format::BC7_SRGB_BLOCK);
		let value = to_format(crate::Formats::RGBA8SNORM);
		assert_eq!(value, vk::Format::R8G8B8A8_SNORM);
		let value = to_format(crate::Formats::RGBA8F);
		assert_eq!(value, vk::Format::UNDEFINED);

		let value = to_format(crate::Formats::RGBA16UNORM);
		assert_eq!(value, vk::Format::R16G16B16A16_UNORM);
		let value = to_format(crate::Formats::RGBA16SNORM);
		assert_eq!(value, vk::Format::R16G16B16A16_SNORM);
		let value = to_format(crate::Formats::RGBA16F);
		assert_eq!(value, vk::Format::R16G16B16A16_SFLOAT);

		let value = to_format(crate::Formats::BGRAu8);
		assert_eq!(value, vk::Format::B8G8R8A8_UNORM);

		let value = to_format(crate::Formats::RGBu11u11u10);
		assert_eq!(value, vk::Format::B10G11R11_UFLOAT_PACK32);

		let value = to_format(crate::Formats::Depth32);
		assert_eq!(value, vk::Format::D32_SFLOAT);
	}

	#[test]
	fn test_to_shader_stage_flags() {
		let value = to_shader_stage_flags(crate::ShaderTypes::Vertex);
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value = to_shader_stage_flags(crate::ShaderTypes::Fragment);
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value = to_shader_stage_flags(crate::ShaderTypes::Compute);
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value = to_shader_stage_flags(crate::ShaderTypes::Task);
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value = to_shader_stage_flags(crate::ShaderTypes::Mesh);
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value = to_shader_stage_flags(crate::ShaderTypes::RayGen);
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value = to_shader_stage_flags(crate::ShaderTypes::ClosestHit);
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value = to_shader_stage_flags(crate::ShaderTypes::AnyHit);
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value = to_shader_stage_flags(crate::ShaderTypes::Intersection);
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value = to_shader_stage_flags(crate::ShaderTypes::Miss);
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value = to_shader_stage_flags(crate::ShaderTypes::Callable);
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);
	}

	#[test]
	fn test_to_pipeline_stage_flags() {
		let value = to_pipeline_stage_flags(crate::Stages::NONE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::NONE);

		let value = to_pipeline_stage_flags(crate::Stages::VERTEX, None, None);
		assert_eq!(
			value,
			vk::PipelineStageFlags2::VERTEX_SHADER | vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT
		);

		let value = to_pipeline_stage_flags(crate::Stages::MESH, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::MESH_SHADER_EXT);

		let value = to_pipeline_stage_flags(crate::Stages::FRAGMENT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::FRAGMENT_SHADER);

		let value = to_pipeline_stage_flags(crate::Stages::FRAGMENT, Some(crate::Layouts::RenderTarget), None);
		assert_eq!(value, vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT);

		let value = to_pipeline_stage_flags(crate::Stages::FRAGMENT, None, Some(crate::Formats::Depth32));
		assert_eq!(
			value,
			vk::PipelineStageFlags2::EARLY_FRAGMENT_TESTS | vk::PipelineStageFlags2::LATE_FRAGMENT_TESTS
		);

		let value = to_pipeline_stage_flags(crate::Stages::COMPUTE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::COMPUTE_SHADER);

		let value = to_pipeline_stage_flags(crate::Stages::COMPUTE, Some(crate::Layouts::Indirect), None);
		assert_eq!(value, vk::PipelineStageFlags2::DRAW_INDIRECT);

		let value = to_pipeline_stage_flags(crate::Stages::TRANSFER, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TRANSFER);

		let value = to_pipeline_stage_flags(crate::Stages::PRESENTATION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::TOP_OF_PIPE);

		let value = to_pipeline_stage_flags(crate::Stages::RAYGEN, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::CLOSEST_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::ANY_HIT, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::INTERSECTION, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::MISS, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::CALLABLE, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::RAY_TRACING_SHADER_KHR);

		let value = to_pipeline_stage_flags(crate::Stages::ACCELERATION_STRUCTURE_BUILD, None, None);
		assert_eq!(value, vk::PipelineStageFlags2::ACCELERATION_STRUCTURE_BUILD_KHR);
	}

	#[test]
	fn test_to_access_flags() {
		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::VERTEX,
			crate::Layouts::Undefined,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::VERTEX_ATTRIBUTE_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::TRANSFER,
			crate::Layouts::Undefined,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::PRESENTATION,
			crate::Layouts::Undefined,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::NONE);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::FRAGMENT,
			crate::Layouts::RenderTarget,
			Some(crate::Formats::RGBA8UNORM),
		);
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::FRAGMENT,
			crate::Layouts::RenderTarget,
			Some(crate::Formats::Depth32),
		);
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::FRAGMENT,
			crate::Layouts::Read,
			Some(crate::Formats::RGBA8UNORM),
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::FRAGMENT,
			crate::Layouts::Read,
			Some(crate::Formats::Depth32),
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_SAMPLED_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::COMPUTE,
			crate::Layouts::Indirect,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::INDIRECT_COMMAND_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::COMPUTE,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_READ);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::RAYGEN,
			crate::Layouts::ShaderBindingTable,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_BINDING_TABLE_READ_KHR);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::RAYGEN,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(
			crate::AccessPolicies::READ,
			crate::Stages::ACCELERATION_STRUCTURE_BUILD,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_READ_KHR);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::TRANSFER,
			crate::Layouts::Undefined,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::TRANSFER_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::COMPUTE,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::FRAGMENT,
			crate::Layouts::RenderTarget,
			Some(crate::Formats::RGBA8UNORM),
		);
		assert_eq!(value, vk::AccessFlags2::COLOR_ATTACHMENT_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::READ_WRITE,
			crate::Stages::FRAGMENT,
			crate::Layouts::RenderTarget,
			Some(crate::Formats::RGBA8UNORM),
		);
		assert_eq!(
			value,
			vk::AccessFlags2::COLOR_ATTACHMENT_READ | vk::AccessFlags2::COLOR_ATTACHMENT_WRITE
		);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::FRAGMENT,
			crate::Layouts::RenderTarget,
			Some(crate::Formats::Depth32),
		);
		assert_eq!(value, vk::AccessFlags2::DEPTH_STENCIL_ATTACHMENT_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::FRAGMENT,
			crate::Layouts::General,
			Some(crate::Formats::RGBA8UNORM),
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::FRAGMENT,
			crate::Layouts::General,
			Some(crate::Formats::Depth32),
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::RAYGEN,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::SHADER_WRITE);

		let value = to_access_flags(
			crate::AccessPolicies::WRITE,
			crate::Stages::ACCELERATION_STRUCTURE_BUILD,
			crate::Layouts::General,
			None,
		);
		assert_eq!(value, vk::AccessFlags2::ACCELERATION_STRUCTURE_WRITE_KHR);
	}

	#[test]
	fn stages_to_vk_shader_stage_flags() {
		let value: vk::ShaderStageFlags = crate::Stages::VERTEX.into();
		assert_eq!(value, vk::ShaderStageFlags::VERTEX);

		let value: vk::ShaderStageFlags = crate::Stages::FRAGMENT.into();
		assert_eq!(value, vk::ShaderStageFlags::FRAGMENT);

		let value: vk::ShaderStageFlags = crate::Stages::COMPUTE.into();
		assert_eq!(value, vk::ShaderStageFlags::COMPUTE);

		let value: vk::ShaderStageFlags = crate::Stages::MESH.into();
		assert_eq!(value, vk::ShaderStageFlags::MESH_EXT);

		let value: vk::ShaderStageFlags = crate::Stages::TASK.into();
		assert_eq!(value, vk::ShaderStageFlags::TASK_EXT);

		let value: vk::ShaderStageFlags = crate::Stages::RAYGEN.into();
		assert_eq!(value, vk::ShaderStageFlags::RAYGEN_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::CLOSEST_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::CLOSEST_HIT_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::ANY_HIT.into();
		assert_eq!(value, vk::ShaderStageFlags::ANY_HIT_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::INTERSECTION.into();
		assert_eq!(value, vk::ShaderStageFlags::INTERSECTION_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::MISS.into();
		assert_eq!(value, vk::ShaderStageFlags::MISS_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::CALLABLE.into();
		assert_eq!(value, vk::ShaderStageFlags::CALLABLE_KHR);

		let value: vk::ShaderStageFlags = crate::Stages::ACCELERATION_STRUCTURE_BUILD.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = crate::Stages::TRANSFER.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = crate::Stages::PRESENTATION.into();
		assert_eq!(value, vk::ShaderStageFlags::default());

		let value: vk::ShaderStageFlags = crate::Stages::NONE.into();
		assert_eq!(value, vk::ShaderStageFlags::default());
	}

	#[test]
	fn datatype_to_vk_format() {
		let value: vk::Format = crate::DataTypes::U8.into();
		assert_eq!(value, vk::Format::R8_UINT);

		let value: vk::Format = crate::DataTypes::U16.into();
		assert_eq!(value, vk::Format::R16_UINT);

		let value: vk::Format = crate::DataTypes::U32.into();
		assert_eq!(value, vk::Format::R32_UINT);

		let value: vk::Format = crate::DataTypes::Int.into();
		assert_eq!(value, vk::Format::R32_SINT);

		let value: vk::Format = crate::DataTypes::Int2.into();
		assert_eq!(value, vk::Format::R32G32_SINT);

		let value: vk::Format = crate::DataTypes::Int3.into();
		assert_eq!(value, vk::Format::R32G32B32_SINT);

		let value: vk::Format = crate::DataTypes::Int4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SINT);

		let value: vk::Format = crate::DataTypes::Float.into();
		assert_eq!(value, vk::Format::R32_SFLOAT);

		let value: vk::Format = crate::DataTypes::Float2.into();
		assert_eq!(value, vk::Format::R32G32_SFLOAT);

		let value: vk::Format = crate::DataTypes::Float3.into();
		assert_eq!(value, vk::Format::R32G32B32_SFLOAT);

		let value: vk::Format = crate::DataTypes::Float4.into();
		assert_eq!(value, vk::Format::R32G32B32A32_SFLOAT);
	}

	#[test]
	fn datatype_size() {
		let value = crate::DataTypes::U8.size();
		assert_eq!(value, 1);

		let value = crate::DataTypes::U16.size();
		assert_eq!(value, 2);

		let value = crate::DataTypes::U32.size();
		assert_eq!(value, 4);

		let value = crate::DataTypes::Int.size();
		assert_eq!(value, 4);

		let value = crate::DataTypes::Int2.size();
		assert_eq!(value, 8);

		let value = crate::DataTypes::Int3.size();
		assert_eq!(value, 12);

		let value = crate::DataTypes::Int4.size();
		assert_eq!(value, 16);

		let value = crate::DataTypes::Float.size();
		assert_eq!(value, 4);

		let value = crate::DataTypes::Float2.size();
		assert_eq!(value, 8);

		let value = crate::DataTypes::Float3.size();
		assert_eq!(value, 12);

		let value = crate::DataTypes::Float4.size();
		assert_eq!(value, 16);
	}

	#[test]
	fn shader_types_to_stages() {
		let value: crate::Stages = crate::ShaderTypes::Vertex.into();
		assert_eq!(value, crate::Stages::VERTEX);

		let value: crate::Stages = crate::ShaderTypes::Fragment.into();
		assert_eq!(value, crate::Stages::FRAGMENT);

		let value: crate::Stages = crate::ShaderTypes::Compute.into();
		assert_eq!(value, crate::Stages::COMPUTE);

		let value: crate::Stages = crate::ShaderTypes::Task.into();
		assert_eq!(value, crate::Stages::TASK);

		let value: crate::Stages = crate::ShaderTypes::Mesh.into();
		assert_eq!(value, crate::Stages::MESH);

		let value: crate::Stages = crate::ShaderTypes::RayGen.into();
		assert_eq!(value, crate::Stages::RAYGEN);

		let value: crate::Stages = crate::ShaderTypes::ClosestHit.into();
		assert_eq!(value, crate::Stages::CLOSEST_HIT);

		let value: crate::Stages = crate::ShaderTypes::AnyHit.into();
		assert_eq!(value, crate::Stages::ANY_HIT);

		let value: crate::Stages = crate::ShaderTypes::Intersection.into();
		assert_eq!(value, crate::Stages::INTERSECTION);

		let value: crate::Stages = crate::ShaderTypes::Miss.into();
		assert_eq!(value, crate::Stages::MISS);

		let value: crate::Stages = crate::ShaderTypes::Callable.into();
		assert_eq!(value, crate::Stages::CALLABLE);
	}

	#[test]
	fn test_image_type_from_extent() {
		let value = image_type_from_extent(vk::Extent3D {
			width: 1,
			height: 1,
			depth: 1,
		})
		.expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D {
			width: 2,
			height: 1,
			depth: 1,
		})
		.expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_1D);

		let value = image_type_from_extent(vk::Extent3D {
			width: 2,
			height: 2,
			depth: 1,
		})
		.expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_2D);

		let value = image_type_from_extent(vk::Extent3D {
			width: 2,
			height: 2,
			depth: 2,
		})
		.expect("Failed to get image type from extent.");
		assert_eq!(value, vk::ImageType::TYPE_3D);
	}
}
