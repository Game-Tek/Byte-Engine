#pragma once

#include "ByteEngine/Core.h"

#include <GAL/Vulkan/Vulkan.h>
#include <GAL/Vulkan/VulkanMemory.h>
#include <GAL/Vulkan/VulkanBuffer.h>
#include <GAL/Vulkan/VulkanPipelines.h>
#include <GAL/Vulkan/VulkanRenderPass.h>
#include <GAL/Vulkan/VulkanFramebuffer.h>
#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanCommandBuffer.h>
#include <GAL/Vulkan/VulkanRenderContext.h>
#include <GAL/Vulkan/VulkanSynchronization.h>
#include <GAL/Vulkan/VulkanBindings.h>

#include "ByteEngine/Debug/Assert.h"

static constexpr uint8 MAX_CONCURRENT_FRAMES = 3;

using AllocationId = uint64;

struct RenderAllocation
{
	uint32 Size = 0, Offset = 0;
	AllocationId AllocationId = 0;
};

#if (_WIN64)
#undef OPAQUE
constexpr GAL::RenderAPI API = GAL::RenderAPI::VULKAN;
using Queue = GAL::VulkanQueue;
using Fence = GAL::VulkanFence;
using Shader = GAL::VulkanShader;
using Buffer = GAL::VulkanBuffer;
using Texture = GAL::VulkanTexture;
using Surface = GAL::VulkanSurface;
using Pipeline = GAL::VulkanPipeline;
using Semaphore = GAL::VulkanSemaphore;
using RenderPass = GAL::VulkanRenderPass;
using TextureSampler = GAL::VulkanSampler;
using TextureView = GAL::VulkanTextureView;
using BindingsSet = GAL::VulkanBindingsSet;
using FrameBuffer = GAL::VulkanFramebuffer;
using CommandPool = GAL::VulkanCommandPool;
using DeviceMemory = GAL::VulkanDeviceMemory;
using RenderDevice = GAL::VulkanRenderDevice;
using BindingsPool = GAL::VulkanBindingsPool;
using RenderContext = GAL::VulkanRenderContext;
using CommandBuffer = GAL::VulkanCommandBuffer;
using PipelineCache = GAL::VulkanPipelineCache;
using PipelineLayout = GAL::VulkanPipelineLayout;
using BindingsSetLayout = GAL::VulkanBindingsSetLayout;
using RasterizationPipeline = GAL::VulkanRasterizationPipeline;

using CullMode = GAL::CullMode;
using IndexType = GAL::VulkanIndexType;
using Dimensions = GAL::VulkanDimensions;
using ColorSpace = GAL::VulkanColorSpace;
using BufferType = GAL::VulkanBufferType;
using MemoryType = GAL::VulkanMemoryType;
using ShaderType = GAL::VulkanShaderType;
using TextureUses = GAL::VulkanTextureUses;
using PresentMode = GAL::VulkanPresentMode;
using ShaderStage = GAL::VulkanShaderStage;
using BindingType = GAL::VulkanBindingType;
using AccessFlags = GAL::VulkanAccessFlags;
using TextureType = GAL::VulkanTextureType;
using PipelineType = GAL::VulkanPipelineType;
using BindingFlags = GAL::VulkanBindingFlags;
using PipelineStage = GAL::VulkanPipelineStage;
using TextureFormat = GAL::VulkanTextureFormat;
using TextureTiling = GAL::VulkanTextureTiling;
using TextureLayout = GAL::VulkanTextureLayout;
using ShaderDataType = GAL::VulkanShaderDataType;
using QueueCapabilities = GAL::VulkanQueueCapabilities;
#endif

inline TextureType::value_type ConvertTextureType(const GAL::TextureType type)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return TextureTypeToVulkanTextureType(type);
	}
}

inline ShaderStage ConvertShaderStage(const GAL::ShaderStage::value_type shaderStage)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return GAL::ShaderStageToVulkanShaderStage(shaderStage);
	}
}

inline ShaderDataType ConvertShaderDataType(const GAL::ShaderDataType type)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return ShaderDataTypeToVulkanShaderDataType(type);
	}
}

inline ShaderType ConvertShaderType(const GAL::ShaderType shader)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return ShaderTypeToVulkanShaderType(shader);
	}
}

inline BindingType ConvertBindingType(const GAL::BindingType bindingsType)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return BindingTypeToVulkanBindingType(bindingsType);
	}
}


inline Dimensions ConvertDimension(const GAL::Dimension dimension)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return GAL::DimensionsToVulkanDimension(dimension);
	}
}

inline IndexType SelectIndexType(const uint64 indexSize)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		BE_ASSERT(indexSize == 2 || indexSize == 4, "Unexpected size");
		return indexSize == 2 ? IndexType::UINT16 : IndexType::UINT32;
	}
}

inline TextureFormat ConvertFormat(const GAL::TextureFormat format)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return GAL::TextureFormatToVulkanTextureFormat(format);
	}
}

inline uint8 FormatSize(const TextureFormat format)
{
	switch (format)
	{
	case GAL::VulkanTextureFormat::UNDEFINED: return 0;
	case GAL::VulkanTextureFormat::R_I8: break;
	case GAL::VulkanTextureFormat::R_I16: break;
	case GAL::VulkanTextureFormat::R_I32: break;
	case GAL::VulkanTextureFormat::R_I64: break;
	case GAL::VulkanTextureFormat::RG_I8: break;
	case GAL::VulkanTextureFormat::RG_I16: break;
	case GAL::VulkanTextureFormat::RG_I32: break;
	case GAL::VulkanTextureFormat::RG_I64: break;
	case GAL::VulkanTextureFormat::RGB_I8: return 3;
	case GAL::VulkanTextureFormat::RGB_I16: break;
	case GAL::VulkanTextureFormat::RGB_I32: break;
	case GAL::VulkanTextureFormat::RGB_I64: break;
	case GAL::VulkanTextureFormat::RGBA_I8: return 4;
	case GAL::VulkanTextureFormat::RGBA_I16: break;
	case GAL::VulkanTextureFormat::RGBA_I32: break;
	case GAL::VulkanTextureFormat::RGBA_I64: break;
	case GAL::VulkanTextureFormat::BGRA_I8: return 4;
	case GAL::VulkanTextureFormat::BGR_I8: break;
	case GAL::VulkanTextureFormat::DEPTH16: break;
	case GAL::VulkanTextureFormat::DEPTH32: break;
	case GAL::VulkanTextureFormat::DEPTH16_STENCIL8: break;
	case GAL::VulkanTextureFormat::DEPTH24_STENCIL8: break;
	case GAL::VulkanTextureFormat::DEPTH32_STENCIL8: break;
	default: return 0;
	}
}