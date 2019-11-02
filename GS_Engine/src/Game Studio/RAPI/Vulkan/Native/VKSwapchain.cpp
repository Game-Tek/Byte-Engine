#include "VKSwapchain.h"

#include "RAPI/Vulkan/Vulkan.h"
#include "VKDevice.h"

#include "VKSemaphore.h"
#include "VKSurface.h"

VKSwapchainCreator::VKSwapchainCreator(VKDevice* _Device, const VkSwapchainCreateInfoKHR* _VkSCIKHR) : VKObjectCreator<VkSwapchainKHR>(_Device)
{
	auto ff = vkCreateSwapchainKHR(m_Device->GetVkDevice(), _VkSCIKHR, ALLOCATOR, &Handle);
	//GS_VK_CHECK(vkCreateSwapchainKHR(m_Device->GetVkDevice(), _VkSCIKHR, ALLOCATOR, &Handle), "Failed to create Swapchain!");
}

VKSwapchain::~VKSwapchain()
{
	vkDestroySwapchainKHR(m_Device->GetVkDevice(), Handle, ALLOCATOR);
}

FVector<VkImage> VKSwapchain::GetImages() const
{
	FVector<VkImage> Images(3);
	uint32_t ImageCount = 0;
	vkGetSwapchainImagesKHR(m_Device->GetVkDevice(), Handle, &ImageCount, nullptr);
	Images.resize(ImageCount);
	vkGetSwapchainImagesKHR(m_Device->GetVkDevice(), Handle, &ImageCount, Images.getData());

	return Images;
}

void VKSwapchain::Recreate(const VKSurface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode)
{
	vkDestroySwapchainKHR(m_Device->GetVkDevice(), Handle, ALLOCATOR);

	VkSwapchainCreateInfoKHR SwapchainCreateInfo = CreateSwapchainCreateInfo(_Surface, _SurfaceFormat, _SurfaceColorSpace, _SurfaceExtent, _PresentMode, Handle);

	GS_VK_CHECK(vkCreateSwapchainKHR(m_Device->GetVkDevice(), &SwapchainCreateInfo, ALLOCATOR, &Handle), "Failed to create Swapchain!")
}

uint32 VKSwapchain::AcquireNextImage(const VKSemaphore& _ImageAvailable) const
{
	uint32 ImageIndex = 0;
	vkAcquireNextImageKHR(m_Device->GetVkDevice(), Handle, 0xffffffffffffffff, _ImageAvailable.GetHandle(), VK_NULL_HANDLE, &ImageIndex);
	return ImageIndex;
}

VkSwapchainCreateInfoKHR VKSwapchain::CreateSwapchainCreateInfo(const VKSurface& _Surface, VkFormat _SurfaceFormat, VkColorSpaceKHR _SurfaceColorSpace, VkExtent2D _SurfaceExtent, VkPresentModeKHR _PresentMode, VkSwapchainKHR _OldSwapchain)
{
	VkSwapchainCreateInfoKHR SwapchainCreateInfo = { VK_STRUCTURE_TYPE_SWAPCHAIN_CREATE_INFO_KHR };

	SwapchainCreateInfo.surface = _Surface.GetHandle();
	SwapchainCreateInfo.minImageCount = 2;
	SwapchainCreateInfo.imageFormat = _SurfaceFormat;
	SwapchainCreateInfo.imageColorSpace = _SurfaceColorSpace;
	SwapchainCreateInfo.imageExtent = _SurfaceExtent;
	//The imageArrayLayers specifies the amount of layers each image consists of. This is always 1 unless you are developing a stereoscopic 3D application.
	SwapchainCreateInfo.imageArrayLayers = 1;
	SwapchainCreateInfo.imageUsage = VK_IMAGE_USAGE_COLOR_ATTACHMENT_BIT;
	SwapchainCreateInfo.imageSharingMode = VK_SHARING_MODE_EXCLUSIVE;
	SwapchainCreateInfo.queueFamilyIndexCount = 0;
	SwapchainCreateInfo.pQueueFamilyIndices = nullptr;
	SwapchainCreateInfo.preTransform = VK_SURFACE_TRANSFORM_IDENTITY_BIT_KHR;
	SwapchainCreateInfo.compositeAlpha = VK_COMPOSITE_ALPHA_OPAQUE_BIT_KHR;
	SwapchainCreateInfo.presentMode = _PresentMode;
	SwapchainCreateInfo.clipped = VK_TRUE;
	SwapchainCreateInfo.oldSwapchain = _OldSwapchain;

	return SwapchainCreateInfo;
}
