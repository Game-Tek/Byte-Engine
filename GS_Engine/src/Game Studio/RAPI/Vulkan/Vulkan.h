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
	case Format::BGRA_I8:			return VK_FORMAT_B8G8R8_UNORM;
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
	case ImageLayout::GENERAL:					return VK_IMAGE_LAYOUT_GENERAL;
	case ImageLayout::COLOR_ATTACHMENT:			return VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_ATTACHMENT:	return VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_READ_ONLY:	return VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::TRANSFER_SOURCE:			return VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
	case ImageLayout::TRANSFER_DESTINATION:		return VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
	case ImageLayout::PREINITIALIZED:			return VK_IMAGE_LAYOUT_PREINITIALIZED;
	default:									return VK_IMAGE_LAYOUT_UNDEFINED;
	}
}
INLINE VkShaderStageFlagBits ShaderTypeToVkShaderStageFlagBits(ShaderType _ST)
{
	switch (_ST)
	{
	case ShaderType::VERTEX_SHADER:			return VK_SHADER_STAGE_VERTEX_BIT;
	case ShaderType::TESSELLATION_SHADER:	return VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT;
	case ShaderType::GEOMETRY_SHADER:		return VK_SHADER_STAGE_GEOMETRY_BIT;
	case ShaderType::FRAGMENT_SHADER:		return VK_SHADER_STAGE_FRAGMENT_BIT;
	case ShaderType::COMPUTE_SHADER:		return VK_SHADER_STAGE_COMPUTE_BIT;
	default:								return VK_SHADER_STAGE_FLAG_BITS_MAX_ENUM;
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