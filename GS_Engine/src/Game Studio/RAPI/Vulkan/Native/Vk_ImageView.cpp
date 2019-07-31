#include "Vk_ImageView.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_ImageView::Vk_ImageView(const Vk_Device& _Device, const Vk_Image& _Image, VkImageViewType _IVT, VkFormat _Format, VkImageAspectFlags _IAF) : VulkanObject(_Device)
{
	VkImageViewCreateInfo ImageViewCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	ImageViewCreateInfo.image = _Image;
	ImageViewCreateInfo.viewType = _IVT;
	ImageViewCreateInfo.format = _Format;
	ImageViewCreateInfo.components.r = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.g = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.b = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.a = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.subresourceRange.aspectMask = _IAF;
	ImageViewCreateInfo.subresourceRange.baseMipLevel = 0;
	ImageViewCreateInfo.subresourceRange.levelCount = 1;
	ImageViewCreateInfo.subresourceRange.baseArrayLayer = 0;
	ImageViewCreateInfo.subresourceRange.layerCount = 1;

	GS_VK_CHECK(vkCreateImageView(m_Device, &ImageViewCreateInfo, ALLOCATOR, &ImageView), "Failed to create Image View!")
}

Vk_ImageView::~Vk_ImageView()
{
	vkDestroyImageView(m_Device, ImageView, ALLOCATOR);
}
