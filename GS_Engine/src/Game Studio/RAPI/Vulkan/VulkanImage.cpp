#include "VulkanImage.h"

#include "Vulkan.h"

#include "Native/VKDevice.h"

VulkanImageBase::VulkanImageBase(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType,
	const ImageDimensions _ID) : Image(_ImgExtent, _ImgFormat, _ImgType, _ID)
{
}

VulkanImage::VulkanImage(const VKDevice& _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const ImageUse _ImgUse) :
	VulkanImageBase(_ImgExtent, _ImgFormat, _ImgType, _ID), 
	m_Image(_Device, Extent2DToVkExtent2D(_ImgExtent), ImageDimensionsToVkImageType(_ID), FormatToVkFormat(_ImgFormat), ImageUseToVkImageUsageFlagBits(_ImgUse)),
	ImageMemory(_Device),
	ImageView(_Device, m_Image, ImageDimensionsToVkImageViewType(_ID), FormatToVkFormat(_ImgFormat), ImageTypeToVkImageAspectFlagBits(_ImgType))
{
	VkMemoryRequirements MemoryRequirements;
	vkGetImageMemoryRequirements(_Device, m_Image, &MemoryRequirements);
	ImageMemory.AllocateDeviceMemory(MemoryRequirements, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT);

	ImageMemory.BindImageMemory(m_Image);
}
