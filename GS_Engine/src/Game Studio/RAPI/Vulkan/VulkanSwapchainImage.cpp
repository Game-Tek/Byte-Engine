#include "VulkanSwapchainImage.h"

#include "Vulkan.h"

VulkanSwapchainImage::VulkanSwapchainImage(const Vk_Device& _Device, VkImage _Image, VkFormat _Format) : ImageView(_Device, _Image, VK_IMAGE_VIEW_TYPE_2D, _Format, VK_IMAGE_ASPECT_COLOR_BIT)
{
}