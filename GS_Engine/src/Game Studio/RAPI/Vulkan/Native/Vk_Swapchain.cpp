#include "Vk_Swapchain.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "Vk_Device.h"

#include "Vk_Semaphore.h"
#include "Vk_Surface.h"

Vk_Swapchain::Vk_Swapchain(const Vk_Device& _Device, const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode) : VulkanObject(_Device)
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo = CreateSwapchainCreateInfo(_Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, _PresentMode, VK_NULL_HANDLE);

	GS_VK_CHECK(vkCreateSwapchainKHR(m_Device, &SwapchainCreateInfo, ALLOCATOR, &Swapchain), "Failed to create Swapchain!");
}

Vk_Swapchain::~Vk_Swapchain()
{
	vkDestroySwapchainKHR(m_Device, Swapchain, ALLOCATOR);
}

FVector<VkImage> Vk_Swapchain::GetImages() const
{
	FVector<VkImage> Images(3);
	uint32_t ImageCount = 0;
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, nullptr);
	Images.resize(ImageCount);
	vkGetSwapchainImagesKHR(m_Device, Swapchain, &ImageCount, Images.data());

	return Images;
}

void Vk_Swapchain::Recreate(const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode)
{
	vkDestroySwapchainKHR(m_Device, Swapchain, ALLOCATOR);

	VkSwapchainCreateInfoKHR SwapchainCreateInfo = CreateSwapchainCreateInfo(_Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, _PresentMode, Swapchain);

	GS_VK_CHECK(vkCreateSwapchainKHR(m_Device, &SwapchainCreateInfo, ALLOCATOR, &Swapchain), "Failed to create Swapchain!")
}

uint32 Vk_Swapchain::AcquireNextImage(const Vk_Semaphore& _ImageAvailable)
{
	uint32 ImageIndex = 0;
	vkAcquireNextImageKHR(m_Device, Swapchain, 0xffffffffffffffff, _ImageAvailable, VK_NULL_HANDLE, &ImageIndex);
	return ImageIndex;
}

VkSwapchainCreateInfoKHR Vk_Swapchain::CreateSwapchainCreateInfo(const Vk_Surface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain)
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };

	SwapchainCreateInfo.surface = _Surface;
	SwapchainCreateInfo.minImageCount = 4;
	SwapchainCreateInfo.imageFormat = _SurfaceFormat;
	SwapchainCreateInfo.imageColorSpace = _SurfaceColorSpace;
	SwapchainCreateInfo.imageExtent = _SurfaceExtent;
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	SwapchainCreateInfo.imageArrayLayers = 1;
	//Should be VK_IMAGE_USAGE_TRANSFER_DST_BIT
	SwapchainCreateInfo.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	SwapchainCreateInfo.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	SwapchainCreateInfo.queueFamilyIndexCount = 0;
	SwapchainCreateInfo.pQueueFamilyIndices = nullptr;
	SwapchainCreateInfo.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	//The compositeAlpha field specifies if the alpha channel should be used for blending with other windows in the window system.
	//You'll almost always want to simply ignore the alpha channel, hence VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR.
	SwapchainCreateInfo.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	SwapchainCreateInfo.presentMode = _PresentMode;
	SwapchainCreateInfo.clipped = VK_TRUE;
	SwapchainCreateInfo.oldSwapchain = _OldSwapchain;

	return SwapchainCreateInfo;
}