#pragma once

#include <GAL/Vulkan/Vulkan.h>
#include <GAL/Vulkan/VulkanBuffer.h>
#include <GAL/Vulkan/VulkanCommandBuffer.h>
#include <GAL/Vulkan/VulkanFramebuffer.h>
#include <GAL/Vulkan/VulkanMemory.h>
#include <GAL/Vulkan/VulkanPipelines.h>
#include <GAL/Vulkan/VulkanRenderContext.h>
#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanRenderPass.h>
#include <GAL/Vulkan/VulkanSynchronization.h>

#if (_WIN64)
using RenderDevice = GAL::VulkanRenderDevice;
using Queue = GAL::VulkanQueue;
using RenderContext = GAL::VulkanRenderContext;
using Buffer = GAL::VulkanBuffer;
using GraphicsPipeline = GAL::VulkanGraphicsPipeline;
using DeviceMemory = GAL::VulkanDeviceMemory;
using CommandBuffer = GAL::VulkanCommandBuffer;
using CommandPool = GAL::VulkanCommandPool;
using RenderPass = GAL::VulkanRenderPass;
using FrameBuffer = GAL::VulkanFramebuffer;
using Fence = GAL::VulkanFence;
using Semaphore = GAL::VulkanSemaphore;
using Image = GAL::VulkanImage;
using ImageView = GAL::VulkanImageView;
using Shader = GAL::VulkanShader;
using Surface = GAL::VulkanSurface;

using QueueCapabilities = GAL::VulkanQueueCapabilities;
using PresentMode = GAL::VulkanPresentMode;
using ImageFormat = GAL::VulkanFormat;
using ImageUse = GAL::VulkanImageUse;
using ColorSpace = GAL::VulkanColorSpace;
using BufferType = GAL::VulkanBufferType;
using MemoryType = GAL::VulkanMemoryType;
#endif
