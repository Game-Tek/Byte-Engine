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
#include <ByteEngine\Debug\Assert.h>

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
using Image = GAL::VulkanImage;
using Shader = GAL::VulkanShader;
using Buffer = GAL::VulkanBuffer;
using Surface = GAL::VulkanSurface;
using Pipeline = GAL::VulkanPipeline;
using Semaphore = GAL::VulkanSemaphore;
using ImageView = GAL::VulkanImageView;
using RenderPass = GAL::VulkanRenderPass;
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
using ImageUse = GAL::VulkanImageUse;
using ImageFormat = GAL::VulkanFormat;
using IndexType = GAL::VulkanIndexType;
using ColorSpace = GAL::VulkanColorSpace;
using BufferType = GAL::VulkanBufferType;
using MemoryType = GAL::VulkanMemoryType;
using ShaderType = GAL::VulkanShaderType;
using PresentMode = GAL::VulkanPresentMode;
using ImageTiling = GAL::VulkanImageTiling;
using ShaderStage = GAL::VulkanShaderStage;
using BindingType = GAL::VulkanBindingType;
using ImageLayout = GAL::VulkanImageLayout;
using PipelineType = GAL::VulkanPipelineType;
using ShaderDataType = GAL::VulkanShaderDataType;
using QueueCapabilities = GAL::VulkanQueueCapabilities;
#endif

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

inline BindingType ConvertBindingTypes(const GAL::BindingType bindingsType)
{
	if constexpr (API == GAL::RenderAPI::VULKAN)
	{
		return BindingTypeToVulkanBindingType(bindingsType);
	}
}

inline IndexType SelectIndexType(const uint64 indexSize)
{
	BE_ASSERT(indexSize == 2 || indexSize == 4, "Unexpected size");
	return indexSize == 2 ? IndexType::UINT16 : IndexType::UINT32;
}