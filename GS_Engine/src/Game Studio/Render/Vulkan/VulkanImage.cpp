#include "Vulkan.h"

#include "VulkanImage.h"

Vk_Image::Vk_Image(VkDevice _Device, VkExtent2D _Extent, VkImageType _Type, VkFormat _Format, VkImageUsageFlags _IUF) : VulkanObject(m_Device)
{
	VkImageCreateInfo ImageCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO };
	ImageCreateInfo.imageType = _Type;
	ImageCreateInfo.extent.width = _Extent.width;
	ImageCreateInfo.extent.height = _Extent.height;
	ImageCreateInfo.extent.depth = 1;
	ImageCreateInfo.mipLevels = 1;
	ImageCreateInfo.arrayLayers = 1;
	ImageCreateInfo.format = _Format;
	ImageCreateInfo.samples = VK_SAMPLE_COUNT_1_BIT;
	ImageCreateInfo.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
	ImageCreateInfo.usage = _IUF;
	ImageCreateInfo.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
	ImageCreateInfo.tiling = VK_IMAGE_TILING_OPTIMAL;

	GS_VK_CHECK(vkCreateImage(m_Device, &ImageCreateInfo, ALLOCATOR, &Image), "Failed to create Image!")
}

Vk_Image::~Vk_Image()
{
	vkDestroyImage(m_Device, Image, ALLOCATOR);
}

Vk_ImageView::Vk_ImageView(VkDevice _Device, VkImage _Image, VkImageViewType _IVT, VkFormat _Format, VkImageAspectFlagBits _IAFB) : VulkanObject(_Device)
{
	VkImageViewCreateInfo ImageViewCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	ImageViewCreateInfo.image = _Image;
	ImageViewCreateInfo.viewType = _IVT;
	ImageViewCreateInfo.format = _Format;
	ImageViewCreateInfo.components.r = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.g = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.b = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.components.a = VK_COMPONENT_SWIZZLE_IDENTITY;
	ImageViewCreateInfo.subresourceRange.aspectMask = _IAFB;
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
