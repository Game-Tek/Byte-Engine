#pragma once

#include "ByteEngine/Core.h"

#include "ByteEngine/Handle.hpp"

#if (BE_VULKAN)
#include <GAL/Vulkan/Vulkan.h>
#include <GAL/Vulkan/VulkanMemory.h>
#include <GAL/Vulkan/VulkanBuffer.h>
#include <GAL/Vulkan/VulkanBindings.h>
#include <GAL/Vulkan/VulkanPipelines.h>
#include <GAL/Vulkan/VulkanQueryPool.h>
#include <GAL/Vulkan/VulkanRenderPass.h>
#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanCommandList.h>
#include <GAL/Vulkan/VulkanRenderContext.h>
#include <GAL/Vulkan/VulkanSynchronization.h>
#include <GAL/Vulkan/VulkanAccelerationStructures.h>
#elif (BE_DX12)
#include <GAL/DX12/DX12.h>
#include <GAL/DX12/DX12Memory.h>
#include <GAL/DX12/DX12Buffer.h>
#include <GAL/DX12/DX12Pipelines.h>
#include <GAL/DX12/DX12QueryPool.h>
#include <GAL/DX12/DX12RenderPass.h>
#include <GAL/DX12/DX12Framebuffer.h>
#include <GAL/DX12/DX12RenderDevice.h>
#include <GAL/DX12/DX12CommandList.h>
#include <GAL/DX12/DX12RenderContext.h>
#include <GAL/DX12/DX12Synchronization.h>
#include <GAL/DX12/DX12AccelerationStructure.hpp>
#endif

#include <GAL/DX12/DX12Bindings.h>

MAKE_HANDLE(uint32, RenderModel);

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

#if (BE_VULKAN)
#undef OPAQUE
using Queue = GAL::VulkanQueue;
using Texture = GAL::VulkanTexture;
using Surface = GAL::VulkanSurface;
using GPUBuffer = GAL::VulkanBuffer;
using QueryPool = GAL::VulkanQueryPool;
using GPUPipeline = GAL::VulkanPipeline;
using RenderPass = GAL::VulkanRenderPass;
using TextureSampler = GAL::VulkanSampler;
using TextureView = GAL::VulkanTextureView;
using CommandList = GAL::VulkanCommandList;
using BindingsSet = GAL::VulkanBindingsSet;
using Synchronizer = GAL::VulkanSynchronizer;
using DeviceMemory = GAL::VulkanDeviceMemory;
using RenderDevice = GAL::VulkanRenderDevice;
using BindingsPool = GAL::VulkanBindingsPool;
using RenderContext = GAL::VulkanRenderContext;
using PipelineCache = GAL::VulkanPipelineCache;
using PipelineLayout = GAL::VulkanPipelineLayout;
using BindingsSetLayout = GAL::VulkanBindingsSetLayout;
using AccelerationStructure = GAL::VulkanAccelerationStructure;
#elif (BE_DX12)
using Queue = GAL::DX12Queue;
using Fence = GAL::DX12Fence;
using GPUBuffer = GAL::DX12Buffer;
using Texture = GAL::DX12Texture;
using Surface = GAL::DX12Surface;
using GPUPipeline = GAL::DX12Pipeline;
using GPUSemaphore = GAL::DX12Semaphore;
using QueryPool = GAL::DX12QueryPool;
using RenderPass = GAL::DX12RenderPass;
using TextureSampler = GAL::DX12Sampler;
using TextureView = GAL::DX12TextureView;
using CommandList = GAL::DX12CommandList;
using BindingsSet = GAL::DX12BindingsSet;
using FrameBuffer = GAL::DX12Framebuffer;
using DeviceMemory = GAL::DX12DeviceMemory;
using RenderDevice = GAL::DX12RenderDevice;
using BindingsPool = GAL::DX12BindingsPool;
using RenderContext = GAL::DX12RenderContext;
using PipelineCache = GAL::DX12PipelineCache;
using PipelineLayout = GAL::DX12PipelineLayout;
using BindingsSetLayout = GAL::DX12BindingsSetLayout;
using AccelerationStructure = GAL::DX12AccelerationStructure;
#endif

constexpr GAL::RenderAPI API = GAL::RenderAPI::VULKAN;

//inline ShaderStage ConvertShaderStage(const GAL::ShaderStages shaderStage)
//{
//	if constexpr (API == GAL::RenderAPI::VULKAN)
//	{
//		return GAL::ShaderStageToVulkanShaderStage(shaderStage);
//	}
//}