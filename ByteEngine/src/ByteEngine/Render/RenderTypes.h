#pragma once

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

static constexpr uint8 MAX_CONCURRENT_FRAMES = 3;

GTSL::Ranger<const GAL::ShaderDataTypes> GetShaderDataTypes(const GTSL::Ranger<const uint8> ranger)
{
	return GTSL::Ranger<const GAL::ShaderDataTypes>(reinterpret_cast<const GAL::ShaderDataTypes*>(ranger.begin()), reinterpret_cast<const GAL::ShaderDataTypes*>(ranger.end()));
}

#if (_WIN64)
using Queue = GAL::VulkanQueue;
using Fence = GAL::VulkanFence;
using Image = GAL::VulkanImage;
using Shader = GAL::VulkanShader;
using Buffer = GAL::VulkanBuffer;
using Surface = GAL::VulkanSurface;
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
using GraphicsPipeline = GAL::VulkanGraphicsPipeline;

using ImageUse = GAL::VulkanImageUse;
using ImageFormat = GAL::VulkanFormat;
using ColorSpace = GAL::VulkanColorSpace;
using BufferType = GAL::VulkanBufferType;
using MemoryType = GAL::VulkanMemoryType;
using PresentMode = GAL::VulkanPresentMode;
using ImageTiling = GAL::VulkanImageTiling;
using QueueCapabilities = GAL::VulkanQueueCapabilities;
#endif
