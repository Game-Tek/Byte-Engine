#pragma once

#include "ByteEngine/Core.h"

#include "ByteEngine/Debug/Assert.h"

#include <GAL/Vulkan/Vulkan.h>
#include <GAL/Vulkan/VulkanMemory.h>
#include <GAL/Vulkan/VulkanBuffer.h>
#include <GAL/Vulkan/VulkanBindings.h>
#include <GAL/Vulkan/VulkanPipelines.h>
#include <GAL/Vulkan/VulkanQueryPool.h>
#include <GAL/Vulkan/VulkanRenderPass.h>
#include <GAL/Vulkan/VulkanFramebuffer.h>
#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanCommandBuffer.h>
#include <GAL/Vulkan/VulkanRenderContext.h>
#include <GAL/Vulkan/VulkanSynchronization.h>
#include <GAL/Vulkan/VulkanAccelerationStructures.h>

/**
 * \brief Defines the maximum number of frames that can be processed concurrently in the CPU and GPU.
 * This might be used to define the number of resources to allocate for those resources that don't allow concurrent use.
 * Most of the time the CPU will be working on frame N+1, while the GPU will be working on frame N and each respective unit
 * will be modifying and/or reading the resources for the frame they are currently working on.
 *
 * This number was chosen since we consider that under normal conditions no more than two frames will ever be worked on concurrently.
 */
static constexpr uint8 MAX_CONCURRENT_FRAMES = 3;

/**
 * \brief Typedef for opaque GPU allocator ID. Refer to RenderAllocation::AllocationId for more details.
 */
using AllocationId = uint64;

/**
 * \brief Handle to a GPU allocation. This handle refers to a GPU local allocation.
 */
struct RenderAllocation
{
	/**
	 * \brief Size of the allocation. In bytes.
	 */
	uint32 Size = 0;
	/**
	 * \brief Offset of the allocation in bytes to the start of a device memory allocation. One will not normally access this value,
	 * it's just here for the allocator.
	 */
	uint32 Offset = 0;
	
	/**
	 * \brief An opaque ID which MIGHT be used to keep track of some allocator internal data.
	 */
	AllocationId AllocationId = 0;

	/**
	* \brief Pointer to a mapped memory section.
	 */
	void* Data = nullptr;
};

#if (_WIN64)
#undef OPAQUE
using Queue = GAL::VulkanQueue;
using Fence = GAL::VulkanFence;
using Shader = GAL::VulkanShader;
using Buffer = GAL::VulkanBuffer;
using Texture = GAL::VulkanTexture;
using Surface = GAL::VulkanSurface;
using Pipeline = GAL::VulkanPipeline;
using Semaphore = GAL::VulkanSemaphore;
using QueryPool = GAL::VulkanQueryPool;
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
using ComputePipeline = GAL::VulkanComputePipeline;
using BindingsSetLayout = GAL::VulkanBindingsSetLayout;
using RayTracingPipeline = GAL::VulkanRayTracingPipeline;
using AccelerationStructure = GAL::VulkanAccelerationStructure;
using RasterizationPipeline = GAL::VulkanRasterizationPipeline;

using CullMode = GAL::CullMode;
using QueryType = GAL::VulkanQueryType;
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
using GeometryType = GAL::VulkanGeometryType;
using BindingFlags = GAL::VulkanBindingFlags;
using GeometryFlags = GAL::VulkanGeometryFlags;
using PipelineStage = GAL::VulkanPipelineStage;
using TextureFormat = GAL::VulkanTextureFormat;
using TextureTiling = GAL::VulkanTextureTiling;
using TextureLayout = GAL::VulkanTextureLayout;
using ShaderDataType = GAL::VulkanShaderDataType;
using AllocationFlags = GAL::VulkanAllocateFlags;
using QueueCapabilities = GAL::VulkanQueueCapabilities;
using BuildType = GAL::VulkanAccelerationStructureBuildType;
using GeometryInstanceFlags = GAL::VulkanGeometryInstanceFlags;
using AccelerationStructureType = GAL::VulkanAccelerationStructureType;
using AccelerationStructureFlags = GAL::VulkanAccelerationStructureFlags;
#endif

constexpr GAL::RenderAPI API = GAL::RenderAPI::VULKAN;

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
	case GAL::VulkanTextureFormat::R_I8: return 1;
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