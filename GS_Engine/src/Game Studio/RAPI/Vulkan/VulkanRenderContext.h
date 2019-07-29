#pragma once

#include "Core.h"

#include "RAPI/RenderContext.h"

#include "Containers/FVector.hpp"
#include "Native/Vk_Surface.h"
#include "Native/Vk_Swapchain.h"
#include "Native/Vk_CommandPool.h"
#include "Native/Vk_CommandBuffer.h"
#include "Native/Vk_Semaphore.h"

class Vk_Device;
enum VkPresentModeKHR;
enum VkColorSpaceKHR;
enum VkFormat;

class Window;

GS_CLASS VulkanRenderContext final : public RenderContext
{
	Vk_Surface Surface;
	Vk_Swapchain Swapchain;
	Vk_Semaphore ImageAvailable;
	Vk_Semaphore RenderFinished;

	Vk_CommandPool CommandPool;

	Vk_Queue PresentationQueue;

	uint8 CurrentImage = 0;
	uint8 MaxFramesInFlight = 0;

	FVector<Vk_CommandBuffer> CommandBuffers;
public:
	VulkanRenderContext(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window);
	~VulkanRenderContext() = default;

	void OnResize() final  override;

	void Present() final override;
	void Flush() final override;
	void BeginRecording() final override;
	void EndRecording() final override;
	void BeginRenderPass(const RenderPassBeginInfo& _RPBI) final override;
	void EndRenderPass(RenderPass* _RP) final override;
	void BindMesh(Mesh* _Mesh) final override;
	void BindGraphicsPipeline(GraphicsPipeline* _GP) final override;
	void BindComputePipeline(ComputePipeline* _CP) final override;
	void DrawIndexed(const DrawInfo& _DI) final override;
	void Dispatch(uint32 _WorkGroupsX, uint32 _WorkGroupsY, uint32 _WorkGroupsZ) final override;
};