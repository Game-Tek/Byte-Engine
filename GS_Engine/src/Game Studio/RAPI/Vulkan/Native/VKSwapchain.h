#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"
#include "Containers/FVector.hpp"

class VKSurface;
class VKSemaphore;

MAKE_VK_HANDLE(VkSwapchainKHR)
MAKE_VK_HANDLE(VkImage)

struct VkSwapchainCreateInfoKHR;
struct VkExtent2D;
enum VkColorSpaceKHR;
enum VkFormat;
enum VkPresentModeKHR;

struct GS_API VKSwapchainCreator final : VKObjectCreator<VkSwapchainKHR>
{
	VKSwapchainCreator(VKDevice* _Device, const VkSwapchainCreateInfoKHR * _VkSCIKHR);
};

class GS_API VKSwapchain final : public VKObject<VkSwapchainKHR>
{
	static VkSwapchainCreateInfoKHR CreateSwapchainCreateInfo(const VKSurface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain);
public:
	VKSwapchain(const VKSwapchainCreator& _VKSC) : VKObject<VkSwapchainKHR>(_VKSC)
	{
	}

	~VKSwapchain();

	void Recreate(const VKSurface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode);
	uint32 AcquireNextImage(const VKSemaphore& _ImageAvailable) const;

	[[nodiscard]] FVector<VkImage> GetImages() const;
};