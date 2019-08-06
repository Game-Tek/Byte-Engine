#include "VulkanImage.h"

#include "Vulkan.h"

#include "Native/Vk_Device.h"

VulkanImage::VulkanImage(const Vk_Device& _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const ImageUse _ImgUse, LoadOperations _LO, StoreOperations _SO, ImageLayout _IL, ImageLayout _FL) :
	Image(_ImgExtent, _ImgFormat, _ID,_ImgType, _ImgUse, _LO, _SO, _IL, _FL), 
	m_Image(_Device, Extent2DToVkExtent2D(_ImgExtent), ImageDimensionsToVkImageType(_ID), FormatToVkFormat(_ImgFormat), ImageUseToVkImageUsageFlagBits(_ImgUse)),
	ImageMemory(_Device),
	ImageView(_Device, m_Image, ImageDimensionsToVkImageViewType(_ID), FormatToVkFormat(_ImgFormat), ImageTypeToVkImageAspectFlagBits(_ImgType))
{
	VkMemoryRequirements MemoryRequirements;
	vkGetImageMemoryRequirements(_Device, m_Image, &MemoryRequirements);
	ImageMemory.AllocateDeviceMemory(&MemoryRequirements);

	ImageMemory.BindImageMemory(m_Image);
}
