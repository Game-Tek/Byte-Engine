#pragma once

#include "Core.h"

#include "VulkanBase.h"

enum VkPresentModeKHR;

MAKE_VK_HANDLE(VkSwapchainKHR)
MAKE_VK_HANDLE(VkSurfaceKHR)
MAKE_VK_HANDLE(VkFormat)
MAKE_VK_HANDLE(VkColorSpaceKHR)
MAKE_VK_HANDLE(VkExtent2D)
MAKE_VK_HANDLE(VkPhysicalDevice)
MAKE_VK_HANDLE(VkImage)
MAKE_VK_HANDLE(VkInstance)

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

	INLINE VkSwapchainKHR GetVkSwapchain() const { return Swapchain; }
};

GS_CLASS Vulkan_Surface final : public VulkanObject
{
	static VkSurfaceFormatKHR PickBestFormat(VkPhysicalDevice _PD, VkSurfaceKHR _Surface);

public:
	VkInstance m_Instance = nullptr;
	VkSurfaceKHR Surface = nullptr;
	VkSurfaceFormatKHR Format = {};

	Vulkan_Surface(VkDevice _Device, VkInstance _Instance, VkPhysicalDevice _PD, HWND _HWND);
	~Vulkan_Surface();
};