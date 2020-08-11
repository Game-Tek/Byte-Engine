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
using GraphicsPipeline = GAL::VulkanGraphicsPipeline;
using BindingsSetLayout = GAL::VulkanBindingsSetLayout;

using ImageUse = GAL::VulkanImageUse;
using ImageFormat = GAL::VulkanFormat;
using ColorSpace = GAL::VulkanColorSpace;
using BufferType = GAL::VulkanBufferType;
using MemoryType = GAL::VulkanMemoryType;
using PresentMode = GAL::VulkanPresentMode;
using ImageTiling = GAL::VulkanImageTiling;
using QueueCapabilities = GAL::VulkanQueueCapabilities;
using ShaderStage = GAL::VulkanShaderStage;
using BindingType = GAL::VulkanBindingType;
using ShaderType = GAL::VulkanShaderType;
using CullMode = GAL::CullMode;
using ShaderDataType = GAL::VulkanShaderDataType;
using PipelineType = GAL::VulkanPipelineType;
using IndexType = GAL::VulkanIndexType;
using ImageLayout = GAL::VulkanImageLayout;
#endif

inline void ConvertShaderDataType(const GTSL::Ranger<const GAL::ShaderDataType> shaderDataTypes, const GTSL::Ranger<ShaderDataType> datas)
{
	for(uint64 i = 0; i < shaderDataTypes.ElementCount(); ++i) { datas[i] = ShaderDataTypeToVulkanShaderDataType(shaderDataTypes[i]); }
}

inline ShaderType ConvertShaderType(const GAL::ShaderType shader)
{
	switch(shader)
	{
	case GAL::ShaderType::VERTEX_SHADER: return GAL::VulkanShaderType::VERTEX;
	case GAL::ShaderType::TESSELLATION_CONTROL_SHADER: return GAL::VulkanShaderType::TESSELLATION_CONTROL;
	case GAL::ShaderType::TESSELLATION_EVALUATION_SHADER: return GAL::VulkanShaderType::TESSELLATION_EVALUATION;
	case GAL::ShaderType::GEOMETRY_SHADER: return GAL::VulkanShaderType::GEOMETRY;
	case GAL::ShaderType::FRAGMENT_SHADER: return GAL::VulkanShaderType::FRAGMENT;
	case GAL::ShaderType::COMPUTE_SHADER: return GAL::VulkanShaderType::COMPUTE;
	default: GAL_DEBUG_BREAK;
	}

	return GAL::VulkanShaderType::VERTEX;
}

inline void ConvertBindingTypes(const GTSL::Ranger<const GAL::BindingType> bindingsTypes, const GTSL::Ranger<BindingType> bindings)
{
	for (uint64 i = 0; i < bindingsTypes.ElementCount(); ++i) { bindings[i] = BindingTypeToVulkanBindingType(bindingsTypes[i]); }
}

inline IndexType SelectIndexType(const uint64 indexSize)
{
	BE_ASSERT(indexSize == 2 || indexSize == 4, "Unexpected size");
	return indexSize == 2 ? IndexType::UINT16 : IndexType::UINT32;
}