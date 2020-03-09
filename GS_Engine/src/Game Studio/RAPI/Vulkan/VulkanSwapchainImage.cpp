#include "VulkanSwapchainImage.h"

#include "Vulkan.h"

#include "RAPI/Vulkan/VulkanRenderDevice.h"

VulkanSwapchainImage::VulkanSwapchainImage(VulkanRenderDevice* device, const RenderTargetCreateInfo& imageCreateInfo,
                                           VkImage image) : VulkanRenderTargetBase(imageCreateInfo)
{
	VkImageViewCreateInfo vk_image_view_create_info{ VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	vk_image_view_create_info.format = FormatToVkFormat(imageCreateInfo.Format);
	vk_image_view_create_info.image = image;
	vk_image_view_create_info.viewType = VK_IMAGE_VIEW_TYPE_2D;
	vk_image_view_create_info.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	vk_image_view_create_info.subresourceRange.baseArrayLayer = 0;
	vk_image_view_create_info.subresourceRange.baseMipLevel = 0;
	vk_image_view_create_info.subresourceRange.layerCount = 1;
	vk_image_view_create_info.subresourceRange.levelCount = 1;

	VK_CHECK(vkCreateImageView(device->GetVkDevice(), &vk_image_view_create_info, device->GetVkAllocationCallbacks(), &imageView));
}
