#include "VulkanSwapchainImage.h"

#include "Vulkan.h"

VulkanSwapchainImage::VulkanSwapchainImage(const VKDevice& _Device, VkImage _Image, Format _Format) : VulkanImageBase(Extent2D(1280, 720), _Format, ImageType::COLOR, ImageDimensions::IMAGE_2D),
ImageView(_Device, _Image, VK_IMAGE_VIEW_TYPE_2D, FormatToVkFormat(_Format), VK_IMAGE_ASPECT_COLOR_BIT)
{
}