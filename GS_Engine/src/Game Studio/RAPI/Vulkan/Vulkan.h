#pragma once

#include <vulkan/vulkan.h>

#include <stdexcept>

#ifdef GS_DEBUG
#define VK_CHECK(func) { if ((func) != VK_SUCCESS) { __debugbreak(); } }
#else
#define GS_VK_CHECK(func) func
#endif // GS_DEBUG

#define ALLOCATOR nullptr

#include "RAPI/RenderCore.h"

#include "Utility/Extent.h"

using namespace RAPI;

INLINE ImageFormat VkFormatToImageFormat(const VkFormat format)
{
	switch (format)
	{
	case VK_FORMAT_R8_UNORM: return ImageFormat::R_I8;
	case VK_FORMAT_R16_UNORM: return ImageFormat::R_I16;
	case VK_FORMAT_R32_UINT: return ImageFormat::R_I32;
	case VK_FORMAT_R64_UINT: return ImageFormat::R_I64;
	case VK_FORMAT_R8G8_UNORM: return ImageFormat::RG_I8;
	case VK_FORMAT_R16G16_UNORM: return ImageFormat::RG_I16;
	case VK_FORMAT_R32G32_UINT: return ImageFormat::RG_I32;
	case VK_FORMAT_R64G64_UINT: return ImageFormat::RG_I64;
	case VK_FORMAT_R8G8B8_UNORM: return ImageFormat::RGB_I8;
	case VK_FORMAT_R16G16B16_UNORM: return ImageFormat::RGB_I16;
	case VK_FORMAT_R32G32B32_UINT: return ImageFormat::RGB_I32;
	case VK_FORMAT_R64G64B64_UINT: return ImageFormat::RGB_I64;
	case VK_FORMAT_R8G8B8A8_UNORM: return ImageFormat::RGBA_I8;
	case VK_FORMAT_R16G16B16A16_UNORM: return ImageFormat::RGBA_I16;
	case VK_FORMAT_R32G32B32A32_UINT: return ImageFormat::RGBA_I32;
	case VK_FORMAT_R64G64B64A64_UINT: return ImageFormat::RGBA_I64;
	case VK_FORMAT_B8G8R8_UNORM: return ImageFormat::BGR_I8;
	case VK_FORMAT_B8G8R8A8_UNORM: return ImageFormat::BGRA_I8;
	case VK_FORMAT_R16_SFLOAT: return ImageFormat::R_F16;
	case VK_FORMAT_R32_SFLOAT: return ImageFormat::R_F32;
	case VK_FORMAT_R64_SFLOAT: return ImageFormat::R_F64;
	case VK_FORMAT_R16G16_SFLOAT: return ImageFormat::RG_F16;
	case VK_FORMAT_R32G32_SFLOAT: return ImageFormat::RG_F32;
	case VK_FORMAT_R64G64_SFLOAT: return ImageFormat::RG_F64;
	case VK_FORMAT_R16G16B16_SFLOAT: return ImageFormat::RGB_F16;
	case VK_FORMAT_R32G32B32_SFLOAT: return ImageFormat::RGB_F32;
	case VK_FORMAT_R64G64B64_SFLOAT: return ImageFormat::RGB_F64;
	case VK_FORMAT_R16G16B16A16_SFLOAT: return ImageFormat::RGBA_F16;
	case VK_FORMAT_R32G32B32A32_SFLOAT: return ImageFormat::RGBA_F32;
	case VK_FORMAT_R64G64B64A64_SFLOAT: return ImageFormat::RGBA_F64;
	case VK_FORMAT_D16_UNORM: return ImageFormat::DEPTH16;
	case VK_FORMAT_D32_SFLOAT: return ImageFormat::DEPTH32;
	case VK_FORMAT_D16_UNORM_S8_UINT: return ImageFormat::DEPTH16_STENCIL8;
	case VK_FORMAT_D24_UNORM_S8_UINT: return ImageFormat::DEPTH24_STENCIL8;
	case VK_FORMAT_D32_SFLOAT_S8_UINT: return ImageFormat::DEPTH32_STENCIL8;
	default: return ImageFormat::R_I8;
	}
}

INLINE VkFormat FormatToVkFormat(const ImageFormat imageFormat)
{
	switch (imageFormat)
	{
	case ImageFormat::R_I8: return VK_FORMAT_R8_UNORM;
	case ImageFormat::R_I16: return VK_FORMAT_R16_UNORM;
	case ImageFormat::R_I32: return VK_FORMAT_R32_UINT;
	case ImageFormat::R_I64: return VK_FORMAT_R64_UINT;
	case ImageFormat::RG_I8: return VK_FORMAT_R8G8_UNORM;
	case ImageFormat::RG_I16: return VK_FORMAT_R16G16_UNORM;
	case ImageFormat::RG_I32: return VK_FORMAT_R32G32_UINT;
	case ImageFormat::RG_I64: return VK_FORMAT_R64G64_UINT;
	case ImageFormat::RGB_I8: return VK_FORMAT_R8G8B8_UNORM;
	case ImageFormat::RGB_I16: return VK_FORMAT_R16G16B16_UNORM;
	case ImageFormat::RGB_I32: return VK_FORMAT_R32G32B32_UINT;
	case ImageFormat::RGB_I64: return VK_FORMAT_R64G64B64_UINT;
	case ImageFormat::RGBA_I8: return VK_FORMAT_R8G8B8A8_UNORM;
	case ImageFormat::RGBA_I16: return VK_FORMAT_R16G16B16A16_UNORM;
	case ImageFormat::RGBA_I32: return VK_FORMAT_R32G32B32A32_UINT;
	case ImageFormat::RGBA_I64: return VK_FORMAT_R64G64B64A64_UINT;
	case ImageFormat::BGRA_I8: return VK_FORMAT_B8G8R8A8_UNORM;
	case ImageFormat::BGR_I8: return VK_FORMAT_B8G8R8_UNORM;
	case ImageFormat::R_F16: return VK_FORMAT_R16_SFLOAT;
	case ImageFormat::R_F32: return VK_FORMAT_R32_SFLOAT;
	case ImageFormat::R_F64: return VK_FORMAT_R64_SFLOAT;
	case ImageFormat::RG_F16: return VK_FORMAT_R16G16_SFLOAT;
	case ImageFormat::RG_F32: return VK_FORMAT_R32G32_SFLOAT;
	case ImageFormat::RG_F64: return VK_FORMAT_R64G64_SFLOAT;
	case ImageFormat::RGB_F16: return VK_FORMAT_R16G16B16_SFLOAT;
	case ImageFormat::RGB_F32: return VK_FORMAT_R32G32B32_SFLOAT;
	case ImageFormat::RGB_F64: return VK_FORMAT_R64G64B64_SFLOAT;
	case ImageFormat::RGBA_F16: return VK_FORMAT_R16G16B16A16_SFLOAT;
	case ImageFormat::RGBA_F32: return VK_FORMAT_R32G32B32A32_SFLOAT;
	case ImageFormat::RGBA_F64: return VK_FORMAT_R64G64B64A64_SFLOAT;
	case ImageFormat::DEPTH16: return VK_FORMAT_D16_UNORM;
	case ImageFormat::DEPTH32: return VK_FORMAT_D32_SFLOAT;
	case ImageFormat::DEPTH16_STENCIL8: return VK_FORMAT_D16_UNORM_S8_UINT;
	case ImageFormat::DEPTH24_STENCIL8: return VK_FORMAT_D24_UNORM_S8_UINT;
	case ImageFormat::DEPTH32_STENCIL8: return VK_FORMAT_D32_SFLOAT_S8_UINT;
	default: return VK_FORMAT_UNDEFINED;
	}
}

INLINE VkAttachmentLoadOp RenderTargetLoadOperationsToVkAttachmentLoadOp(const RenderTargetLoadOperations renderTargetLoadOperations)
{
	switch (renderTargetLoadOperations)
	{
	case RenderTargetLoadOperations::UNDEFINED: return VK_ATTACHMENT_LOAD_OP_DONT_CARE;
	case RenderTargetLoadOperations::LOAD: return VK_ATTACHMENT_LOAD_OP_LOAD;
	case RenderTargetLoadOperations::CLEAR: return VK_ATTACHMENT_LOAD_OP_CLEAR;
	default: return VK_ATTACHMENT_LOAD_OP_MAX_ENUM;
	}
}

INLINE VkAttachmentStoreOp RenderTargetStoreOperationsToVkAttachmentStoreOp(const RenderTargetStoreOperations renderTargetStoreOperations)
{
	switch (renderTargetStoreOperations)
	{
	case RenderTargetStoreOperations::UNDEFINED: return VK_ATTACHMENT_STORE_OP_DONT_CARE;
	case RenderTargetStoreOperations::STORE: return VK_ATTACHMENT_STORE_OP_STORE;
	default: return VK_ATTACHMENT_STORE_OP_MAX_ENUM;
	}
}

INLINE VkImageLayout ImageLayoutToVkImageLayout(const ImageLayout imageLayout)
{
	switch (imageLayout)
	{
	case ImageLayout::UNDEFINED: return VK_IMAGE_LAYOUT_UNDEFINED;
	case ImageLayout::SHADER_READ: return VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL;
	case ImageLayout::GENERAL: return VK_IMAGE_LAYOUT_GENERAL;
	case ImageLayout::COLOR_ATTACHMENT: return VK_IMAGE_LAYOUT_COLOR_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_ATTACHMENT: return VK_IMAGE_LAYOUT_DEPTH_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::DEPTH_STENCIL_READ_ONLY: return VK_IMAGE_LAYOUT_DEPTH_READ_ONLY_STENCIL_ATTACHMENT_OPTIMAL;
	case ImageLayout::TRANSFER_SOURCE: return VK_IMAGE_LAYOUT_TRANSFER_SRC_OPTIMAL;
	case ImageLayout::TRANSFER_DESTINATION: return VK_IMAGE_LAYOUT_TRANSFER_DST_OPTIMAL;
	case ImageLayout::PREINITIALIZED: return VK_IMAGE_LAYOUT_PREINITIALIZED;
	case ImageLayout::PRESENTATION: return VK_IMAGE_LAYOUT_PRESENT_SRC_KHR;
	default: return VK_IMAGE_LAYOUT_UNDEFINED;
	}
}

INLINE VkShaderStageFlagBits ShaderTypeToVkShaderStageFlagBits(const ShaderType shaderType)
{
	switch (shaderType)
	{
	case ShaderType::ALL_STAGES: return VK_SHADER_STAGE_ALL_GRAPHICS;
	case ShaderType::VERTEX_SHADER: return VK_SHADER_STAGE_VERTEX_BIT;
	case ShaderType::TESSELLATION_CONTROL_SHADER: return VK_SHADER_STAGE_TESSELLATION_CONTROL_BIT;
	case ShaderType::TESSELLATION_EVALUATION_SHADER: return VK_SHADER_STAGE_TESSELLATION_EVALUATION_BIT;
	case ShaderType::GEOMETRY_SHADER: return VK_SHADER_STAGE_GEOMETRY_BIT;
	case ShaderType::FRAGMENT_SHADER: return VK_SHADER_STAGE_FRAGMENT_BIT;
	case ShaderType::COMPUTE_SHADER: return VK_SHADER_STAGE_COMPUTE_BIT;
	default: return VK_SHADER_STAGE_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkExtent2D Extent2DToVkExtent2D(const Extent2D extent)
{
	return {extent.Width, extent.Height};
}

INLINE VkExtent3D Extent3DToVkExtent3D(const Extent3D extent)
{
	return { extent.Width, extent.Height, extent.Depth };
}

INLINE VkImageViewType ImageDimensionsToVkImageViewType(const ImageDimensions imageDimensions)
{
	switch (imageDimensions)
	{
	case ImageDimensions::IMAGE_1D: return VK_IMAGE_VIEW_TYPE_1D;
	case ImageDimensions::IMAGE_2D: return VK_IMAGE_VIEW_TYPE_2D;
	case ImageDimensions::IMAGE_3D: return VK_IMAGE_VIEW_TYPE_3D;
	default: return VK_IMAGE_VIEW_TYPE_MAX_ENUM;
	}
}

INLINE VkImageType ImageDimensionsToVkImageType(const ImageDimensions imageDimensions)
{
	switch (imageDimensions)
	{
	case ImageDimensions::IMAGE_1D: return VK_IMAGE_TYPE_1D;
	case ImageDimensions::IMAGE_2D: return VK_IMAGE_TYPE_2D;
	case ImageDimensions::IMAGE_3D: return VK_IMAGE_TYPE_3D;
	default: return VK_IMAGE_TYPE_MAX_ENUM;
	}
}

INLINE uint32 ImageTypeToVkImageAspectFlagBits(const ImageType imageType)
{
	switch (imageType)
	{
	case ImageType::COLOR: return VK_IMAGE_ASPECT_COLOR_BIT;
	case ImageType::DEPTH: return VK_IMAGE_ASPECT_DEPTH_BIT;
	case ImageType::STENCIL: return VK_IMAGE_ASPECT_STENCIL_BIT;
	case ImageType::DEPTH_STENCIL: return VK_IMAGE_ASPECT_DEPTH_BIT | VK_IMAGE_ASPECT_STENCIL_BIT;
	default: return VK_IMAGE_ASPECT_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkFormat ShaderDataTypesToVkFormat(const ShaderDataTypes shaderDataTypes)
{
	switch (shaderDataTypes)
	{
	case ShaderDataTypes::FLOAT: return VK_FORMAT_R32_SFLOAT;
	case ShaderDataTypes::FLOAT2: return VK_FORMAT_R32G32_SFLOAT;
	case ShaderDataTypes::FLOAT3: return VK_FORMAT_R32G32B32_SFLOAT;
	case ShaderDataTypes::FLOAT4: return VK_FORMAT_R32G32B32A32_SFLOAT;
	case ShaderDataTypes::INT: return VK_FORMAT_R32_SINT;
	case ShaderDataTypes::INT2: return VK_FORMAT_R32G32_SINT;
	case ShaderDataTypes::INT3: return VK_FORMAT_R32G32B32_SINT;
	case ShaderDataTypes::INT4: return VK_FORMAT_R32G32B32A32_SINT;
	case ShaderDataTypes::BOOL: return VK_FORMAT_R32_SINT;
	default: return VK_FORMAT_UNDEFINED;
	}
}

INLINE VkImageUsageFlagBits ImageUseToVkImageUsageFlagBits(const ImageUse imageUse)
{
	switch (imageUse)
	{
	case ImageUse::TRANSFER_SOURCE: return VK_IMAGE_USAGE_TRANSFER_SRC_BIT;
	case ImageUse::TRANSFER_DESTINATION: return VK_IMAGE_USAGE_TRANSFER_DST_BIT;
	case ImageUse::SAMPLE: return VK_IMAGE_USAGE_SAMPLED_BIT;
	case ImageUse::STORAGE: return VK_IMAGE_USAGE_STORAGE_BIT;
	case ImageUse::COLOR_ATTACHMENT: return VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	case ImageUse::DEPTH_STENCIL_ATTACHMENT: return VK_IMAGE_USAGE_DEPTH_STENCIL_ATTACHMENT_BIT;
	case ImageUse::TRANSIENT_ATTACHMENT: return VK_IMAGE_USAGE_TRANSIENT_ATTACHMENT_BIT;
	case ImageUse::INPUT_ATTACHMENT: return VK_IMAGE_USAGE_INPUT_ATTACHMENT_BIT;
	default: return VK_IMAGE_USAGE_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkDescriptorType UniformTypeToVkDescriptorType(const BindingType uniformType)
{
	switch (uniformType)
	{
	case BindingType::SAMPLER: return VK_DESCRIPTOR_TYPE_SAMPLER;
	case BindingType::COMBINED_IMAGE_SAMPLER: return VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER;
	case BindingType::SAMPLED_IMAGE: return VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE;
	case BindingType::STORAGE_IMAGE: return VK_DESCRIPTOR_TYPE_STORAGE_IMAGE;
	case BindingType::UNIFORM_TEXEL_BUFFER: return VK_DESCRIPTOR_TYPE_UNIFORM_TEXEL_BUFFER;
	case BindingType::STORAGE_TEXEL_BUFFER: return VK_DESCRIPTOR_TYPE_STORAGE_TEXEL_BUFFER;
	case BindingType::UNIFORM_BUFFER: return VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER;
	case BindingType::STORAGE_BUFFER: return VK_DESCRIPTOR_TYPE_STORAGE_BUFFER;
	case BindingType::UNIFORM_BUFFER_DYNAMIC: return VK_DESCRIPTOR_TYPE_UNIFORM_BUFFER_DYNAMIC;
	case BindingType::STORAGE_BUFFER_DYNAMIC: return VK_DESCRIPTOR_TYPE_STORAGE_BUFFER_DYNAMIC;
	case BindingType::INPUT_ATTACHMENT: return VK_DESCRIPTOR_TYPE_INPUT_ATTACHMENT;
	default: return VK_DESCRIPTOR_TYPE_MAX_ENUM;
	}
};

INLINE VkCullModeFlagBits CullModeToVkCullModeFlagBits(const CullMode cullMode)
{
	switch (cullMode)
	{
	case CullMode::CULL_BACK: return VK_CULL_MODE_BACK_BIT;
	case CullMode::CULL_FRONT: return VK_CULL_MODE_FRONT_BIT;
	default: return VK_CULL_MODE_FLAG_BITS_MAX_ENUM;
	}
}

INLINE VkCompareOp CompareOperationToVkCompareOp(const CompareOperation compareOperation)
{
	switch (compareOperation)
	{
	case CompareOperation::NEVER: return VK_COMPARE_OP_NEVER;
	case CompareOperation::LESS: return VK_COMPARE_OP_LESS;
	case CompareOperation::EQUAL: return VK_COMPARE_OP_EQUAL;
	case CompareOperation::LESS_OR_EQUAL: return VK_COMPARE_OP_LESS_OR_EQUAL;
	case CompareOperation::GREATER: return VK_COMPARE_OP_GREATER;
	case CompareOperation::NOT_EQUAL: return VK_COMPARE_OP_NOT_EQUAL;
	case CompareOperation::GREATER_OR_EQUAL: return VK_COMPARE_OP_GREATER_OR_EQUAL;
	case CompareOperation::ALWAYS: return VK_COMPARE_OP_ALWAYS;
	default: ;
	}
	return {};
}

INLINE VkPresentModeKHR PresentModeToVkPresentModeKHR(const PresentMode presentMode)
{
	switch (presentMode)
	{
	case PresentMode::FIFO: return VK_PRESENT_MODE_FIFO_KHR;
	case PresentMode::SWAP: return VK_PRESENT_MODE_MAILBOX_KHR;
	default: return VK_PRESENT_MODE_MAX_ENUM_KHR;
	}
}