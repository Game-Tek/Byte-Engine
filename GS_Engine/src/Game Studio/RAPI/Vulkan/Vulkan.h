#pragma once

#ifdef GS_PLATFORM_WIN
#define VK_USE_PLATFORM_WIN32_KHR
#include "vulkan/vulkan.h"
// ReSharper disable once CppUnusedIncludeDirective
#include "vulkan/vulkan_win32.h"
#endif // GS_PLATFORM_WIN

#include <stdexcept>

#ifdef GS_DEBUG
#define GS_VK_CHECK(func, text)\
{\
if ((func) != VK_SUCCESS)\
{\
	throw std::runtime_error(text);\
}\
}
#elif
#define GS_VK_CHECK(func, text) func
#endif // GS_DEBUG

#define ALLOCATOR nullptr

#include "RAPI/RenderCore.h"

#include "Extent.h"

#include "Containers/FVector.hpp"

GS_STRUCT PipelineState
{
	VkPipelineVertexInputStateCreateInfo		PipelineVertexInputState = { VK_STRUCTURE_TYPE_PIPELINE_VERTEX_INPUT_STATE_CREATE_INFO };
	FVector<VkVertexInputAttributeDescription> VertexElements;
	FVector<VkVertexInputBindingDescription> BindingDescription;
	VkPipelineInputAssemblyStateCreateInfo		PipelineInputAssemblyState;
	VkPipelineTessellationStateCreateInfo		PipelineTessellationState;
	VkViewport Viewport;
	VkRect2D Scissor;
	VkPipelineViewportStateCreateInfo			PipelineViewportState;
	VkPipelineRasterizationStateCreateInfo		PipelineRasterizationState;
	VkPipelineMultisampleStateCreateInfo		PipelineMultisampleState;
	VkPipelineDepthStencilStateCreateInfo		PipelineDepthStencilState;
	VkPipelineColorBlendAttachmentState ColorBlendAttachment = {};
	VkPipelineColorBlendStateCreateInfo			PipelineColorBlendState;
	VkPipelineDynamicStateCreateInfo			PipelineDynamicState;
};

INLINE Format VkFormatToFormat(VkFormat _Format)
{
	switch (_Format)
	{
	case VK_FORMAT_R8_UNORM:					return Format::R_I8;
	case VK_FORMAT_R16_UNORM:					return Format::R_I16;
	case VK_FORMAT_R32_UINT:					return Format::R_I32;
	case VK_FORMAT_R64_UINT:					return Format::R_I64;
	case VK_FORMAT_R8G8_UNORM:					return Format::RG_I8;
	case VK_FORMAT_R16G16_UNORM:				return Format::RG_I16;
	case VK_FORMAT_R32G32_UINT:					return Format::RG_I32;
	case VK_FORMAT_R64G64_UINT:					return Format::RG_I64;
	case VK_FORMAT_R8G8B8_UNORM:				return Format::RGB_I8;
	case VK_FORMAT_R16G16B16_UNORM:				return Format::RGB_I16;
	case VK_FORMAT_R32G32B32_UINT:				return Format::RGB_I32;
	case VK_FORMAT_R64G64B64_UINT:				return Format::RGB_I64;
	case VK_FORMAT_R8G8B8A8_UNORM:				return Format::RGBA_I8;
	case VK_FORMAT_R16G16B16A16_UNORM:			return Format::RGBA_I16;
	case VK_FORMAT_R32G32B32A32_UINT:			return Format::RGBA_I32;
	case VK_FORMAT_R64G64B64A64_UINT:			return Format::RGBA_I64;
	case VK_FORMAT_B8G8R8_UNORM:				return Format::BGR_I8;
	case VK_FORMAT_B8G8R8A8_UNORM:				return Format::BGRA_I8;
	case VK_FORMAT_R16_SFLOAT:					return Format::R_F16;
	case VK_FORMAT_R32_SFLOAT:					return Format::R_F32;
	case VK_FORMAT_R64_SFLOAT:					return Format::R_F64;
	case VK_FORMAT_R16G16_SFLOAT:				return Format::RG_F16;
	case VK_FORMAT_R32G32_SFLOAT:				return Format::RG_F32;
	case VK_FORMAT_R64G64_SFLOAT:				return Format::RG_F64;
	case VK_FORMAT_R16G16B16_SFLOAT:			return Format::RGB_F16;
	case VK_FORMAT_R32G32B32_SFLOAT:			return Format::RGB_F32;
	case VK_FORMAT_R64G64B64_SFLOAT:			return Format::RGB_F64;
	case VK_FORMAT_R16G16B16A16_SFLOAT:			return Format::RGBA_F16;
	case VK_FORMAT_R32G32B32A32_SFLOAT:			return Format::RGBA_F32;
	case VK_FORMAT_R64G64B64A64_SFLOAT:			return Format::RGBA_F64;
	case VK_FORMAT_D16_UNORM:					return Format::DEPTH16;
	case VK_FORMAT_D32_SFLOAT:					return Format::DEPTH32;
	case VK_FORMAT_D16_UNORM_S8_UINT:			return Format::DEPTH16_STENCIL8;
	case VK_FORMAT_D24_UNORM_S8_UINT:			return Format::DEPTH24_STENCIL8;
	case VK_FORMAT_D32_SFLOAT_S8_UINT:			return Format::DEPTH32_STENCIL8;
	default:									return Format::R_I8;
	}
}

INLINE VkFormat FormatToVkFormat(Format _PF)
{
	switch (_PF)
	{
	case Format::R_I8:				return VK_FORMAT_R8_UNORM;
	case Format::R_I16:				return VK_FORMAT_R16_UNORM;
	case Format::R_I32:				return VK_FORMAT_R32_UINT;
	case Format::R_I64:				return VK_FORMAT_R64_UINT;
	case Format::RG_I8:				return VK_FORMAT_R8G8_UNORM;
	case Format::RG_I16:			return VK_FORMAT_R16G16_UNORM;
	case Format::RG_I32:			return VK_FORMAT_R32G32_UINT;
	case Format::RG_I64:			return VK_FORMAT_R64G64_UINT;
	case Format::RGB_I8:			return VK_FORMAT_R8G8B8_UNORM;
	case Format::RGB_I16:			return VK_FORMAT_R16G16B16_UNORM;
	case Format::RGB_I32:			return VK_FORMAT_R32G32B32_UINT;
	case Format::RGB_I64:			return VK_FORMAT_R64G64B64_UINT;
	case Format::RGBA_I8:			return VK_FORMAT_R8G8B8A8_UNORM;
	case Format::RGBA_I16:			return VK_FORMAT_R16G16B16A16_UNORM;
	case Format::RGBA_I32:			return VK_FORMAT_R32G32B32A32_UINT;
	case Format::RGBA_I64:			return VK_FORMAT_R64G64B64A64_UINT;
	case Format::BGRA_I8:			return VK_FORMAT_B8G8R8A8_UNORM;
	case Format::BGR_I8:			return VK_FORMAT_B8G8R8_UNORM;
	case Format::R_F16:				return VK_FORMAT_R16_SFLOAT;
	case Format::R_F32:				return VK_FORMAT_R32_SFLOAT;
	case Format::R_F64:				return VK_FORMAT_R64_SFLOAT;
	case Format::RG_F16:			return VK_FORMAT_R16G16_SFLOAT;
	case Format::RG_F32:			return VK_FORMAT_R32G32_SFLOAT;
	case Format::RG_F64:			return VK_FORMAT_R64G64_SFLOAT;
	case Format::RGB_F16:			return VK_FORMAT_R16G16B16_SFLOAT;
	case Format::RGB_F32:			return VK_FORMAT_R32G32B32_SFLOAT;
	case Format::RGB_F64:			return VK_FORMAT_R64G64B64_SFLOAT;
	case Format::RGBA_F16:			return VK_FORMAT_R16G16B16A16_SFLOAT;
	case Format::RGBA_F32:			return VK_FORMAT_R32G32B32A32_SFLOAT;
	case Format::RGBA_F64:			return VK_FORMAT_R64G64B64A64_SFLOAT;
	case Format::DEPTH16:			return VK_FORMAT_D16_UNORM;
	case Format::DEPTH32:			return VK_FORMAT_D32_SFLOAT;
	case Format::DEPTH16_STENCIL8:	return VK_FORMAT_D16_UNORM_S8_UINT;
	case Format::DEPTH24_STENCIL8:	return VK_FORMAT_D24_UNORM_S8_UINT;
	case Format::DEPTH32_STENCIL8:	return VK_FORMAT_D32_SFLOAT_S8_UINT;
	default:						return VK_FORMAT_UNDEFINED;
	}
}

INLINE VkAttachmentLoadOp LoadOperationsToVkAttachmentLoadOp(LoadOperations _LOp)
{
	switch (_LOp)
	{
	case LoadOperations::UNDEFINED:	return VK_ATTACHMENT_LOAD_OP_DONT_CARE;
	case LoadOperations::LOAD:		return VK_ATTACHMENT_LOAD_OP_LOAD;
	case LoadOperations::CLEAR:		return VK_ATTACHMENT_LOAD_OP_CLEAR;
	default:						return VK_ATTACHMENT_LOAD_OP_MAX_ENUM;
	}
}
INLINE VkAttachmentStoreOp StoreOperationsToVkAttachmentStoreOp(StoreOperations _SOp)
{
	switch (_SOp)
	{
	case StoreOperations::UNDEFINED:	return VK_ATTACHMENT_STORE_OP_DONT_CARE;
	case StoreOperations::STORE:		return VK_ATTACHMENT_STORE_OP_STORE;
	default:							return VK_ATTACHMENT_STORE_OP_MAX_ENUM;
	}
}
INLINE VkImageLayout ImageLayoutToVkImageLayout(ImageLayout _IL)
{
	switch (_IL)
	{
	case ImageLayout::UNDEFINED:				return VK_IMAGE_LAYOUT_UNDEFINED;
	case ImageLayout::SHADER_READ:				return VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
	case ImageLayout::GENERAL:					return VK_IMAGE_LAYOUT_GENERAL;
	case ImageLayout::COLOR_ATTACHMENT:			return VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_ATTACHMENT:	return VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_READ_ONLY:	return VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::TRANSFER_SOURCE:			return VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
	case ImageLayout::TRANSFER_DESTINATION:		return VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
	case ImageLayout::PREINITIALIZED:			return VK_IMAGE_LAYOUT_PREINITIALIZED;
	case ImageLayout::PRESENTATION:				return VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
	default:									return VK_IMAGE_LAYOUT_UNDEFINED;
	}
}
INLINE VkShaderStageFlagBits ShaderTypeToVkShaderStageFlagBits(ShaderType _ST)
{
	switch (_ST)
	{
	case ShaderType::ALL_STAGES:					return VK_SHADER_STAGE_ALL_GRAPHICS;
	case ShaderType::VERTEX_SHADER:					return VK_SHADER_STAGE_VERTEX_BIT;
	case ShaderType::TESSELLATION_CONTROL_SHADER:	return VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT;
	case ShaderType::TESSELLATION_EVALUATION_SHADER:return VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT;
	case ShaderType::GEOMETRY_SHADER:				return VK_SHADER_STAGE_GEOMETRY_BIT;
	case ShaderType::FRAGMENT_SHADER:				return VK_SHADER_STAGE_FRAGMENT_BIT;
	case ShaderType::COMPUTE_SHADER:				return VK_SHADER_STAGE_COMPUTE_BIT;
	default:										return VK_SHADER_STAGE_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkExtent2D Extent2DToVkExtent2D(Extent2D _Extent)
{
	return { _Extent.Width, _Extent.Height };
}

INLINE VkImageViewType ImageDimensionsToVkImageViewType(ImageDimensions _ID)
{
	switch (_ID)
	{
	case ImageDimensions::IMAGE_1D: return VK_IMAGE_VIEW_TYPE_1D;
	case ImageDimensions::IMAGE_2D: return VK_IMAGE_VIEW_TYPE_2D;
	case ImageDimensions::IMAGE_3D: return VK_IMAGE_VIEW_TYPE_3D;
	default:						return VK_IMAGE_VIEW_TYPE_MAX_ENUM;
	}
}

INLINE VkImageType ImageDimensionsToVkImageType(ImageDimensions _ID)
{
	switch (_ID)
	{
	case ImageDimensions::IMAGE_1D:			return VK_IMAGE_TYPE_1D;
	case ImageDimensions::IMAGE_2D:			return VK_IMAGE_TYPE_2D;
	case ImageDimensions::IMAGE_3D:			return VK_IMAGE_TYPE_3D;
	default:								return VK_IMAGE_TYPE_MAX_ENUM;
	}
}

INLINE uint32 ImageTypeToVkImageAspectFlagBits(ImageType _IT)
{
	switch (_IT)
	{
	case ImageType::COLOR:			return VK_IMAGE_ASPECT_COLOR_BIT;
	case ImageType::DEPTH:			return VK_IMAGE_ASPECT_DEPTH_BIT;
	case ImageType::STENCIL:		return VK_IMAGE_ASPECT_STENCIL_BIT;
	case ImageType::DEPTH_STENCIL:	return VK_IMAGE_ASPECT_DEPTH_BIT | VK_IMAGE_ASPECT_STENCIL_BIT;
	default:						return VK_IMAGE_ASPECT_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkFormat ShaderDataTypesToVkFormat(ShaderDataTypes _SDT)
{
	switch (_SDT)
	{
	case ShaderDataTypes::FLOAT:	return VK_FORMAT_R32_SFLOAT;
	case ShaderDataTypes::FLOAT2:	return VK_FORMAT_R32G32_SFLOAT;
	case ShaderDataTypes::FLOAT3:	return VK_FORMAT_R32G32B32_SFLOAT;
	case ShaderDataTypes::FLOAT4:	return VK_FORMAT_R32G32B32A32_SFLOAT;
	case ShaderDataTypes::INT:		return VK_FORMAT_R32_SINT;
	case ShaderDataTypes::INT2:		return VK_FORMAT_R32G32_SINT;
	case ShaderDataTypes::INT3:		return VK_FORMAT_R32G32B32_SINT;
	case ShaderDataTypes::INT4:		return VK_FORMAT_R32G32B32A32_SINT;
	case ShaderDataTypes::BOOL:		return VK_FORMAT_R32_SINT;
	default:						return VK_FORMAT_UNDEFINED;
	}
}

INLINE VkImageUsageFlagBits ImageUseToVkImageUsageFlagBits(ImageUse _IU)
{
	switch(_IU)
	{
	case ImageUse::TRANSFER_SOURCE:				return VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
	case ImageUse::TRANSFER_DESTINATION:		return VK_IMAGE_USAGE_TRANSFER_DST_BIT;
	case ImageUse::SAMPLE:						return VK_IMAGE_USAGE_SAMPLED_BIT;
	case ImageUse::STORAGE:						return VK_IMAGE_USAGE_STORAGE_BIT;
	case ImageUse::COLOR_ATTACHMENT:			return VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	case ImageUse::DEPTH_STENCIL_ATTACHMENT:	return VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT;
	case ImageUse::TRANSIENT_ATTACHMENT:		return VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT;
	case ImageUse::INPUT_ATTACHMENT:			return VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT;
	default:									return VK_IMAGE_USAGE_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkDescriptorType UniformTypeToVkDescriptorType(UniformType _UT)
{
	switch (_UT)
	{
	case UniformType::SAMPLER:					return VK_DESCRIPTOR_TYPE_SAMPLER;
	case UniformType::COMBINED_IMAGE_SAMPLER:	return VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
	case UniformType::SAMPLED_IMAGE:			return VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE;
	case UniformType::STORAGE_IMAGE:			return VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
	case UniformType::UNIFORM_TEXEL_BUFFER:		return VK_DESCRIPTOR_TYPE_UNIFORM_TEXEL_BUFFER;
	case UniformType::STORAGE_TEXEL_BUFFER:		return VK_DESCRIPTOR_TYPE_STORAGE_TEXEL_BUFFER;
	case UniformType::UNIFORM_BUFFER:			return VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
	case UniformType::STORAGE_BUFFER:			return VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
	case UniformType::UNIFORM_BUFFER_DYNAMIC:	return VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC;
	case UniformType::STORAGE_BUFFER_DYNAMIC:	return VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC;
	case UniformType::INPUT_ATTACHMENT:			return VK_DESCRIPTOR_TYPE_INPUT_ATTACHMENT;
	default:									return VK_DESCRIPTOR_TYPE_MAX_ENUM;
	}
};