#pragma once

#ifdef GS_PLATFORM_WIN
#include <vulkan/vulkan.h>
#include <vulkan/vulkan_win32.h>
#define GLFW_INCLUDE_VULKAN
#include <GLFW/glfw3.h>
#define GLFW_EXPOSE_NATIVE_WIN32
#include <GLFW/glfw3native.h>
#endif // GS_PLATFORM_WIN

#ifdef GS_DEBUG
#define GS_VK_CHECK(func, text)\
{\
if (func != VK_SUCCESS)\
{\
	throw std::runtime_error(text);\
}\
}
#elif
#define GS_VK_CHECK(func, text) func
#endif // GS_DEBUG

#define ALLOCATOR nullptr

#include <stdexcept>

#include "Render/RenderCore.h"

VkFormat ColorFormatToVkFormat(ColorFormat _PF)
{
	switch (_PF)
	{
	case ColorFormat::R_I8:		return VK_FORMAT_R8_UNORM;
	case ColorFormat::R_I16:	return VK_FORMAT_R16_UNORM;
	case ColorFormat::R_I32:	return VK_FORMAT_R32_UINT;
	case ColorFormat::R_I64:	return VK_FORMAT_R64_UINT;
	case ColorFormat::RG_I8:	return VK_FORMAT_R8G8_UNORM;
	case ColorFormat::RG_I16:	return VK_FORMAT_R16G16_UNORM;
	case ColorFormat::RG_I32:	return VK_FORMAT_R32G32_UINT;
	case ColorFormat::RG_I64:	return VK_FORMAT_R64G64_UINT;
	case ColorFormat::RGB_I8:	return VK_FORMAT_R8G8B8_UNORM;
	case ColorFormat::RGB_I16:	return VK_FORMAT_R16G16B16_UNORM;
	case ColorFormat::RGB_I32:	return VK_FORMAT_R32G32B32_UINT;
	case ColorFormat::RGB_I64:	return VK_FORMAT_R64G64B64_UINT;
	case ColorFormat::RGBA_I8:	return VK_FORMAT_R8G8B8A8_UNORM;
	case ColorFormat::RGBA_I16:	return VK_FORMAT_R16G16B16A16_UNORM;
	case ColorFormat::RGBA_I32:	return VK_FORMAT_R32G32B32A32_UINT;
	case ColorFormat::RGBA_I64:	return VK_FORMAT_R64G64B64A64_UINT;
	case ColorFormat::BGRA_I8:	return VK_FORMAT_B8G8R8_UNORM;
	case ColorFormat::R_F16:	return VK_FORMAT_R16_SFLOAT;
	case ColorFormat::R_F32:	return VK_FORMAT_R32_SFLOAT;
	case ColorFormat::R_F64:	return VK_FORMAT_R64_SFLOAT;
	case ColorFormat::RG_F16:	return VK_FORMAT_R16G16_SFLOAT;
	case ColorFormat::RG_F32:	return VK_FORMAT_R32G32_SFLOAT;
	case ColorFormat::RG_F64:	return VK_FORMAT_R64G64_SFLOAT;
	case ColorFormat::RGB_F16:	return VK_FORMAT_R16G16B16_SFLOAT;
	case ColorFormat::RGB_F32:	return VK_FORMAT_R32G32B32_SFLOAT;
	case ColorFormat::RGB_F64:	return VK_FORMAT_R64G64B64_SFLOAT;
	case ColorFormat::RGBA_F16:	return VK_FORMAT_R16G16B16A16_SFLOAT;
	case ColorFormat::RGBA_F32:	return VK_FORMAT_R32G32B32A32_SFLOAT;
	case ColorFormat::RGBA_F64:	return VK_FORMAT_R64G64B64A64_SFLOAT;
	default:					return VK_FORMAT_UNDEFINED;
	}

	GS_ASSERT(false);
}