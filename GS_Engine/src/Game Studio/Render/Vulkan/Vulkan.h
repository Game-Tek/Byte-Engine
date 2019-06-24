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

VkFormat PixelFormatToVkFormat(PixelFormat _PF)
{
	switch (_PF)
	{
	case PixelFormat::R_I8:		return VK_FORMAT_R8_UNORM;
	case PixelFormat::R_I16:	return VK_FORMAT_R16_UNORM;
	case PixelFormat::R_I32:	return VK_FORMAT_R32_UINT;
	case PixelFormat::R_I64:	return VK_FORMAT_R64_UINT;
	case PixelFormat::RG_I8:	return VK_FORMAT_R8G8_UNORM;
	case PixelFormat::RG_I16:	return VK_FORMAT_R16G16_UNORM;
	case PixelFormat::RG_I32:	return VK_FORMAT_R32G32_UINT;
	case PixelFormat::RG_I64:	return VK_FORMAT_R64G64_UINT;
	case PixelFormat::RGB_I8:	return VK_FORMAT_R8G8B8_UNORM;
	case PixelFormat::RGB_I16:	return VK_FORMAT_R16G16B16_UNORM;
	case PixelFormat::RGB_I32:	return VK_FORMAT_R32G32B32_UINT;
	case PixelFormat::RGB_I64:	return VK_FORMAT_R64G64B64_UINT;
	case PixelFormat::RGBA_I8:	return VK_FORMAT_R8G8B8A8_UNORM;
	case PixelFormat::RGBA_I16:	return VK_FORMAT_R16G16B16A16_UNORM;
	case PixelFormat::RGBA_I32:	return VK_FORMAT_R32G32B32A32_UINT;
	case PixelFormat::RGBA_I64:	return VK_FORMAT_R64G64B64A64_UINT;
	case PixelFormat::BGRA_I8:	return VK_FORMAT_B8G8R8_UNORM;
	case PixelFormat::R_F16:	return VK_FORMAT_R16_SFLOAT;
	case PixelFormat::R_F32:	return VK_FORMAT_R32_SFLOAT;
	case PixelFormat::R_F64:	return VK_FORMAT_R64_SFLOAT;
	case PixelFormat::RG_F16:	return VK_FORMAT_R16G16_SFLOAT;
	case PixelFormat::RG_F32:	return VK_FORMAT_R32G32_SFLOAT;
	case PixelFormat::RG_F64:	return VK_FORMAT_R64G64_SFLOAT;
	case PixelFormat::RGB_F16:	return VK_FORMAT_R16G16B16_SFLOAT;
	case PixelFormat::RGB_F32:	return VK_FORMAT_R32G32B32_SFLOAT;
	case PixelFormat::RGB_F64:	return VK_FORMAT_R64G64B64_SFLOAT;
	case PixelFormat::RGBA_F16:	return VK_FORMAT_R16G16B16A16_SFLOAT;
	case PixelFormat::RGBA_F32:	return VK_FORMAT_R32G32B32A32_SFLOAT;
	case PixelFormat::RGBA_F64:	return VK_FORMAT_R64G64B64A64_SFLOAT;
	default:					return VK_FORMAT_UNDEFINED;
	}

	GS_ASSERT(false);
}