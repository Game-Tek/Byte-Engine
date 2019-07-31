#include "Vk_Image.h"

#include "RAPI/Vulkan/Vulkan.h"

#include "Vk_Device.h"

Vk_Image::Vk_Image(const Vk_Device& _Device, VkExtent2D _Extent, VkImageType _Type, VkFormat _Format, VkImageUsageFlags _IUF) : VulkanObject(_Device)
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

