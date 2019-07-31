#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

class Vk_Surface;
class Vk_Semaphore;

MAKE_VK_HANDLE(VkSwapchainKHR)
MAKE_VK_HANDLE(VkImage)

struct VkSwapchainCreateInfoKHR;
struct VkExtent2D;
enum VkColorSpaceKHR;
enum VkFormat;
enum VkPresentModeKHR;

GS_CLASS Vk_Swapchain final : public VulkanObject
{
	VkSwapchainKHR Swapchain = nullptr;

	static VkSwapchainCreateInfoKHR CreateSwapchainCreateInfo(const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain);
public:
	Vk_Swapchain(const Vk_Device& _Device, const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode);
	~Vk_Swapchain();

	void Recreate(const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode);
	uint32 AcquireNextImage(const Vk_Semaphore& _ImageAvailable);
	[[nodiscard]] FVector<VkImage> GetImages() const;

	INLINE VkSwapchainKHR GetVkSwapchain() const { return Swapchain; }
	INLINE operator VkSwapchainKHR() const { return Swapchain; }
};