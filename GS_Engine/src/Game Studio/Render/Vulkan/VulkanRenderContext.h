#pragma once

#include "Core.h"

#include "..\RenderContext.h"
#include "VulkanBase.h"

#include "VulkanSync.h"

enum VkPresentModeKHR;
enum VkColorSpaceKHR;
enum VkFormat;
struct VkExtent2D;

struct VkSwapchainCreateInfoKHR;

MAKE_VK_HANDLE(VkSwapchainKHR)
MAKE_VK_HANDLE(VkSurfaceKHR)
MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkImage)
MAKE_VK_HANDLE(VkInstance)
MAKE_VK_HANDLE(VkSemaphore)

GS_CLASS VulkanRenderContext final : public RenderContext
{
	Vulkan_Surface Surface;
	Vulkan_Swapchain Swapchain;
	VulkanSemaphore ImageAvailable;
	VulkanSemaphore RenderFinished;

	VkQueue PresentationQueue = nullptr;
public:
	VulkanRenderContext(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, VkQueue _PresentationQueueIndex);

	void Present() final override;
};

GS_CLASS Vulkan_Swapchain final : public VulkanObject
{
	VkSwapchainKHR Swapchain = nullptr;
	VkPresentModeKHR PresentMode = {};

	VkImage* SwapchainImages = nullptr;

	static uint8 ScorePresentMode(VkPresentModeKHR _PresentMode);
	static void FindPresentMode(VkPresentModeKHR& _PM, VkPhysicalDevice _PD, VkSurfaceKHR _Surface);
	static void CreateSwapchainCreateInfo(VkSwapchainCreateInfoKHR& _SCIK, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain);
public:
	Vulkan_Swapchain(VkDevice _Device, VkPhysicalDevice _PD, VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent);
	~Vulkan_Swapchain();

	void Recreate(VkSurfaceKHR _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent);
	uint32 AcquireNextImage(VkSemaphore _ImageAvailable);

	INLINE VkSwapchainKHR GetVkSwapchain() const { return Swapchain; }
};

GS_CLASS Vulkan_Surface final : public VulkanObject
{
	static VkFormat PickBestFormat(VkPhysicalDevice _PD, VkSurfaceKHR _Surface);

	VkInstance m_Instance = nullptr;
	VkSurfaceKHR Surface = nullptr;
	VkFormat Format = {};
	VkColorSpaceKHR ColorSpace = {};
	VkExtent2D Extent = {};
public:

	Vulkan_Surface(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, HWND _HWND);
	~Vulkan_Surface();

	INLINE VkSurfaceKHR GetVkSurface()					const { return Surface; }
	INLINE VkFormat GetVkSurfaceFormat()				const { return Format; }
	INLINE VkColorSpaceKHR GetVkColorSpaceKHR()			const { return ColorSpace; }
	INLINE VkExtent2D GetVkExtent2D()					const { return Extent; }
};