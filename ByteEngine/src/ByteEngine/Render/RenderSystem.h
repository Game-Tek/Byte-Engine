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
#include <ByteEngine/Resources/StaticMeshResourceManager.h>

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
	using DeviceMemory = GAL::VulkanDeviceMemory;
	using CommandBuffer = GAL::VulkanCommandBuffer;
	using RenderPass = GAL::VulkanRenderPass;
	using FrameBuffer = GAL::VulkanFramebuffer;
	using Fence = GAL::VulkanFence;
	using Semaphore = GAL::VulkanSemaphore;
	using Image = GAL::VulkanImage;
	using Shader = GAL::VulkanShader;

	using QueueCapabilities = GAL::VulkanQueueCapabilities;
	using PresentMode = GAL::VulkanPresentMode;
	using ImageFormat = GAL::VulkanFormat;
	using ColorSpace = GAL::VulkanColorSpace;
	using BufferType = GAL::VulkanBufferType;
	using MemoryType = GAL::VulkanMemoryType;
	
private:
	RenderDevice renderDevice;
	RenderContext renderContext;
	GraphicsPipeline graphicsPipeline;
	RenderPass renderPass;
	GTSL::Array<CommandBuffer, 3> commandBuffers;
	GTSL::Array<FrameBuffer, 3> frameBuffers;

	GTSL::Array<GTSL::RGBA, 3> clearValues;

	GTSL::Array<Image, 5> swapchainImages;
	
	GTSL::Array<Semaphore, 3> imagesAvailable;
	GTSL::Array<Semaphore, 3> rendersFinished;
	GTSL::Array<Fence, 3> inFlightFences;
	GTSL::Array<Fence, 3> imagesInFlight;
	
	Queue graphicsQueue;

	Buffer stagingMesh;
	Buffer deviceMesh;
	DeviceMemory mappedDeviceMemory;
	DeviceMemory deviceMemory;

	void* mappedMemoryPointer = nullptr;

	void render(const GameInstance::TaskInfo& taskInfo);
	void staticMeshLoaded(StaticMeshResourceManager::OnStaticMeshLoad);
};
