#pragma once

#include "Core.h"

#include "RAPI/RenderContext.h"

#include "Containers/FVector.hpp"
#include "Native/VKSurface.h"
#include "Native/VKSwapchain.h"
#include "Native/VKCommandPool.h"
#include "Native/VKCommandBuffer.h"
#include "Native/VKSemaphore.h"
#include "Native/VKFence.h"
#include "Native/vkQueue.h"
#include "VulkanPipelines.h"
#include "VulkanSwapchainImage.h"

class VKDevice;

enum VkPresentModeKHR;
enum VkFormat;
enum VkColorSpaceKHR;

struct SurfaceFormat
{
	VkFormat format;
	VkColorSpaceKHR colorSpace;
};

class Window;

class GS_API VulkanRenderContext final : public RenderContext
{
	Extent2D RenderExtent;

	VKSurface Surface;

	SurfaceFormat Format;
	VkPresentModeKHR PresentMode;

	VKSwapchain Swapchain;
	FVector<VkImage> SwapchainImages;
	mutable FVector<VulkanSwapchainImage*> Images;
	FVector<VKSemaphore> ImagesAvailable;
	FVector<VKSemaphore> RendersFinished;
	FVector<VKFence> InFlightFences;

	vkQueue PresentationQueue;

	VKCommandPool CommandPool;

	FVector<VKCommandBuffer> CommandBuffers;

	uint8 ImageIndex = 0;

	static VKSurfaceCreator CreateSurface(VKDevice* _Device, const VKInstance& _Instance, const Window& _Window);
	VKSwapchainCreator CreateSwapchain(VKDevice* _Device, VkSwapchainKHR _OldSwapchain) const;
	VKCommandPoolCreator CreateCommandPool(VKDevice* _Device);

	SurfaceFormat FindFormat(const vkPhysicalDevice& _PD, VkSurfaceKHR _Surface);
	static VkPresentModeKHR FindPresentMode(const vkPhysicalDevice& _PD, const VKSurface& _Surface);
public:
	VulkanRenderContext(VKDevice* _Device, const VKInstance& _Instance, const vkPhysicalDevice& _PD, const Window& _Window);
	~VulkanRenderContext();

	void OnResize() final  override;

	void AcquireNextImage() override;
	void Flush() final override;
	void Present() final override;
	void BeginRecording() final override;
	void EndRecording() final override;
	void BeginRenderPass(const RenderPassBeginInfo& _RPBI) final override;
	void AdvanceSubPass() override;
	void EndRenderPass(RenderPass* _RP) final override;
	void BindMesh(Mesh* _Mesh) final override;
	void BindUniformLayout(UniformLayout* _UL) override;
	void UpdatePushConstant(const PushConstantsInfo& _PCI) override;
	void BindGraphicsPipeline(GraphicsPipeline* _GP) final override;
	void BindComputePipeline(ComputePipeline* _CP) final override;
	void DrawIndexed(const DrawInfo& _DI) final override;
	void Dispatch(const Extent3D& _WorkGroups) final override;

	[[nodiscard]] FVector<Image*> GetSwapchainImages() const final override;
};