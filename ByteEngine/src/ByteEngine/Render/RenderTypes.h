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

struct MaterialInstanceHandle {
	uint32 MaterialIndex, MaterialInstanceIndex;
};

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
 * \brief Handle to a GPU allocation. This handle refers to a GPU local allocation.
 */
struct RenderAllocation
{	
	/**
	 * \brief An opaque ID which MIGHT be used to keep track of some allocator internal data.
	 */
	uint64 AllocationId = 0;

	/**
	* \brief Pointer to a mapped memory section.
	 */
	void* Data = nullptr;
};

#if (_WIN64)
#undef OPAQUE
using Queue = GAL::VulkanQueue;
using Fence = GAL::VulkanFence;
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
using DeviceMemory = GAL::VulkanDeviceMemory;
using RenderDevice = GAL::VulkanRenderDevice;
using BindingsPool = GAL::VulkanBindingsPool;
using RenderContext = GAL::VulkanRenderContext;
using CommandBuffer = GAL::VulkanCommandBuffer;
using PipelineCache = GAL::VulkanPipelineCache;
using PipelineLayout = GAL::VulkanPipelineLayout;
using BindingsSetLayout = GAL::VulkanBindingsSetLayout;
using AccelerationStructure = GAL::VulkanAccelerationStructure;
#endif

constexpr GAL::RenderAPI API = GAL::RenderAPI::VULKAN;

//inline ShaderStage ConvertShaderStage(const GAL::ShaderStages shaderStage)
//{
//	if constexpr (API == GAL::RenderAPI::VULKAN)
//	{
//		return GAL::ShaderStageToVulkanShaderStage(shaderStage);
//	}
//}