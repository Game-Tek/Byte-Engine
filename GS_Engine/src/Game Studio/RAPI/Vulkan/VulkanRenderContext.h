#pragma once

#include "Core.h"

#include "RAPI/RenderContext.h"

#include "Containers/FVector.hpp"
#include "Native/Vk_Surface.h"
#include "Native/Vk_Swapchain.h"
#include "Native/Vk_CommandPool.h"
#include "Native/Vk_CommandBuffer.h"
#include "Native/Vk_Semaphore.h"
#include "Native/Vk_Fence.h"
#include "Native/Vk_Queue.h"

class Vk_Device;

enum VkPresentModeKHR;
enum VkFormat;
enum VkColorSpaceKHR;

struct SurfaceFormat
{
	VkFormat format;
	VkColorSpaceKHR colorSpace;
};

class Window;

GS_CLASS VulkanRenderContext final : public RenderContext
{
	Vk_Surface Surface;

	SurfaceFormat Format;
	VkPresentModeKHR PresentMode;

	Vk_Swapchain Swapchain;
	const uint8 MaxFramesInFlight = 0;
	FVector<Vk_Semaphore> ImagesAvailable;
	FVector<Vk_Semaphore> RendersFinished;
	FVector<Vk_Fence> InFlightFences;

	Vk_Queue PresentationQueue;

	Vk_CommandPool CommandPool;

	uint8 CurrentImage = 0;

	FVector<Vk_CommandBuffer> CommandBuffers;

	static SurfaceFormat FindFormat(const Vk_PhysicalDevice& _PD, VkSurfaceKHR _Surface);
	static VkPresentModeKHR FindPresentMode(const Vk_PhysicalDevice& _PD, const Vk_Surface& _Surface);
public:
	VulkanRenderContext(const Vk_Device& _Device, const Vk_Instance& _Instance, const Vk_PhysicalDevice& _PD, const Window& _Window);
	~VulkanRenderContext() = default;

	void OnResize() final  override;

	void Flush() final override;
	void Present() final override;
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