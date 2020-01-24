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
#include "VulkanUniformLayout.h"
#include "ScreenQuad.h"
#include "VulkanRenderPass.h"

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
	mutable FVector<VulkanSwapchainImage> swapchainImages;
	FVector<VKSemaphore> ImagesAvailable;
	FVector<VKSemaphore> RendersFinished;
	FVector<VKFence> InFlightFences;

	vkQueue PresentationQueue;

	VKCommandPool CommandPool;

	FVector<VKCommandBuffer> CommandBuffers;
	FVector<VKFramebuffer> FrameBuffers;

	uint8 ImageIndex = 0;
	
	static VKSurfaceCreator CreateSurface(VKDevice* _Device, VKInstance* _Instance, Window* _Window);
	VKSwapchainCreator CreateSwapchain(VKDevice* _Device, VkSwapchainKHR _OldSwapchain) const;
	VKCommandPoolCreator CreateCommandPool(VKDevice* _Device);

	SurfaceFormat FindFormat(const vkPhysicalDevice& _PD, VkSurfaceKHR _Surface);
	static VkPresentModeKHR FindPresentMode(const vkPhysicalDevice& _PD, const VKSurface& _Surface);
public:
	VulkanRenderContext(VulkanRenderDevice* device, VKInstance* _Instance, const vkPhysicalDevice& _PD, Window* _Window);
	~VulkanRenderContext();

	void OnResize(const ResizeInfo& _RI)  override;

	void AcquireNextImage() override;
	void Flush() override;
	void Present() override;
	void BeginRecording() override;
	void EndRecording() override;
	void BeginRenderPass(const RenderPassBeginInfo& _RPBI) override;
	void AdvanceSubPass() override;
	void EndRenderPass(RenderPass* _RP) override;
	void BindMesh(RenderMesh* _Mesh) override;
	void BindUniformLayout(UniformLayout* _UL) override;
	void UpdatePushConstant(const PushConstantsInfo& _PCI) override;
	void BindGraphicsPipeline(GraphicsPipeline* _GP) override;
	void BindComputePipeline(ComputePipeline* _CP) override;
	void DrawIndexed(const DrawInfo& _DrawInfo) override;
	void Dispatch(const Extent3D& _WorkGroups) override;

	void CopyToSwapchain(const CopyToSwapchainInfo& copyToSwapchainInfo) override;
	
	[[nodiscard]] FVector<Image*> GetSwapchainImages() const override;
};