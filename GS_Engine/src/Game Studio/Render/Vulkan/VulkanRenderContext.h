#pragma once

#include "Core.h"

#include "..\RenderContext.h"
#include "VulkanBase.h"

#include "Vk_CommandBuffer.h"
#include "VulkanSync.h"

enum VkPresentModeKHR;
enum VkColorSpaceKHR;
enum VkFormat;
struct VkExtent2D;

class Window;

struct VkSwapchainCreateInfoKHR;

MAKE_VK_HANDLE(VkSwapchainKHR)
MAKE_VK_HANDLE(VkSurfaceKHR)
MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkImage)
MAKE_VK_HANDLE(VkInstance)

GS_CLASS VulkanRenderContext final : public RenderContext
{
	Vk_Surface Surface;
	Vk_Swapchain Swapchain;
	VulkanSemaphore ImageAvailable;
	VulkanSemaphore RenderFinished;

	Vk_CommandPool CommandPool;
	Vk_CommandBuffer CommandBuffer[3];

	VkQueue PresentationQueue = nullptr;

	uint8 CurrentCommandBufferIndex = 0;
public:
	VulkanRenderContext(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window,VkQueue _PresentationQueue, uint32 _PresentationQueueIndex);

	void Present() final override;
	void Flush() final override;
	void BeginRecording() final override;
	void EndRecording() final override;
	void BeginRenderPass(const RenderPassBeginInfo& _RPBI) final override;
	void EndRenderPass(RenderPass* _RP) final override;
	void BindVertexBuffer(VertexBuffer* _VB) final override;
	void BindIndexBuffer(IndexBuffer* _IB) final override;
	void BindGraphicsPipeline(GraphicsPipeline* _GP) final override;
	void BindComputePipeline(ComputePipeline* _CP) final override;
	void DrawIndexed(const DrawInfo& _DI) final override;
	void DrawIndexedInstanced(uint16 _IndexCount) final override;
	void Dispatch(uint32 _WorkGroupsX, uint32 _WorkGroupsY, uint32 _WorkGroupsZ) final override;

};

GS_CLASS Vk_Swapchain final : public VulkanObject
{
	VkSwapchainKHR Swapchain = nullptr;
	VkPresentModeKHR PresentMode = {};

	FVector<VkImage> Images;

	static uint8 ScorePresentMode(VkPresentModeKHR _PresentMode);
	static void FindPresentMode(VkPresentModeKHR& _PM, VkPhysicalDevice _PD, VkSurfaceKHR _Surface);
	static void CreateSwapchainCreateInfo(VkSwapchainCreateInfoKHR& _SCIK, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain);
public:
	Vk_Swapchain(VkDevice _Device, VkPhysicalDevice _PD, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent);
	~Vk_Swapchain();

	void Recreate(VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent);
	uint32 AcquireNextImage(VkSemaphore _ImageAvailable);

	INLINE VkSwapchainKHR GetVkSwapchain() const { return Swapchain; }
	INLINE const FVector<VkImage>& GetImages() const { return Images; }
};

GS_CLASS Vk_Surface final : public VulkanObject
{
	static VkFormat PickBestFormat(VkPhysicalDevice _PD, VkSurfaceKHR _Surface);

	VkInstance m_Instance = nullptr;
	VkSurfaceKHR Surface = nullptr;
	VkFormat Format = {};
	VkColorSpaceKHR ColorSpace = {};
	VkExtent2D Extent = {};
public:

	Vk_Surface(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, Window* _Window);
	~Vk_Surface();

	INLINE VkSurfaceKHR GetVkSurface()					const { return Surface; }
	INLINE VkFormat GetVkSurfaceFormat()				const { return Format; }
	INLINE VkColorSpaceKHR GetVkColorSpaceKHR()			const { return ColorSpace; }
	INLINE VkExtent2D GetVkExtent2D()					const { return Extent; }
};