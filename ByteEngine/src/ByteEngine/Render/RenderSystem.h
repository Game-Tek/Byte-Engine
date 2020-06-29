#pragma once

#include "ByteEngine/Game/System.h"

#include <GAL/Vulkan/VulkanRenderDevice.h>
#include <GAL/Vulkan/VulkanRenderContext.h>
#include <GAL/Vulkan/VulkanPipelines.h>
#include <GAL/Vulkan/VulkanRenderPass.h>
#include <GAL/Vulkan/VulkanFramebuffer.h>
#include <GAL/Vulkan/VulkanBuffer.h>
#include <GAL/Vulkan/VulkanCommandBuffer.h>
#include <GAL/Vulkan/VulkanMemory.h>
#include <GAL/Vulkan/VulkanSynchronization.h>

#include "ByteEngine/Game/GameInstance.h"

namespace GTSL {
	class Window;
}

class RenderSystem : public System
{
public:
	RenderSystem() = default;

	struct InitializeRendererInfo
	{
		GTSL::Window* Window{ 0 };
	};
	void InitializeRenderer(const InitializeRendererInfo& initializeRenderer);
	
	void UpdateWindow(GTSL::Window& window);
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown() override;

	using RenderDevice = GAL::VulkanRenderDevice;
	using RenderContext = GAL::VulkanRenderContext;
	using Queue = GAL::VulkanQueue;
	using Buffer = GAL::VulkanBuffer;
	using GraphicsPipeline = GAL::VulkanGraphicsPipeline;
	using Memory = GAL::VulkanDeviceMemory;
	using CommandBuffer = GAL::VulkanCommandBuffer;
	using RenderPass = GAL::VulkanRenderPass;
	using FrameBuffer = GAL::VulkanFramebuffer;
	using Fence = GAL::VulkanFence;
	using Semaphore = GAL::VulkanSemaphore;
	using Image = GAL::VulkanImage;
	
private:
	RenderDevice renderDevice;
	RenderContext renderContext;
	GraphicsPipeline graphicsPipeline;
	RenderPass renderPass;

	GTSL::Array<Image, 5> swapchainImages;
	GTSL::Array<Semaphore, 5> imagesAvailable;
	GTSL::Array<Fence, 5> renderFinished;
	
	Queue graphicsQueue;

	void test(const GameInstance::TaskInfo& taskInfo);
};
