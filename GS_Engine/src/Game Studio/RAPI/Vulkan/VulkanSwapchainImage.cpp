#include "VulkanSwapchainImage.h"

#include "Vulkan.h"

VKImageViewCreator VulkanSwapchainImage::CreateImageView(VKDevice* _Device, VkImage _Image, const Format _Format)
{
	VkImageViewCreateInfo ImageViewCreateInfo = { VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO };
	ImageViewCreateInfo.format = FormatToVkFormat(_Format);
	ImageViewCreateInfo.image = _Image;
	ImageViewCreateInfo.viewType = VK_IMAGE_VIEW_TYPE_2D;
	ImageViewCreateInfo.subresourceRange.aspectMask = VK_IMAGE_ASPECT_COLOR_BIT;
	ImageViewCreateInfo.subresourceRange.baseArrayLayer = 0;
	ImageViewCreateInfo.subresourceRange.baseMipLevel = 0;
	ImageViewCreateInfo.subresourceRange.layerCount = 1;
	ImageViewCreateInfo.subresourceRange.levelCount = 1;

	return VKImageViewCreator(_Device, &ImageViewCreateInfo);
}

VulkanSwapchainImage::VulkanSwapchainImage(VKDevice* _Device, VkImage _Image, Format _Format) : VulkanImageBase(Extent2D(1280, 720), _Format, ImageType::COLOR, ImageDimensions::IMAGE_2D),
ImageView(CreateImageView(_Device, _Image, _Format))
{
}
