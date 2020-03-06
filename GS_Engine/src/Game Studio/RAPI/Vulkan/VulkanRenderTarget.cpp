#include "VulkanRenderTarget.h"

#include "Vulkan.h"

#include "VulkanRenderDevice.h"

VulkanRenderTargetBase::VulkanRenderTargetBase(const RenderTargetCreateInfo& imageCreateInfo) : RenderTarget(imageCreateInfo)
{
}

VulkanRenderTarget::VulkanRenderTarget(VulkanRenderDevice* device, const RenderTargetCreateInfo& imageCreateInfo) : VulkanRenderTargetBase(
	imageCreateInfo)
{
	const auto image_format = FormatToVkFormat(imageCreateInfo.Format);
	const auto image_extent = Extent3DToVkExtent3D(imageCreateInfo.Extent);

	VkImageCreateInfo vk_image_create_info = {VK_STRUCTURE_TYPE_IMAGE_CREATE_INFO};
	vk_image_create_info.format = image_format;
	vk_image_create_info.arrayLayers = 1;
	vk_image_create_info.extent = image_extent;
	vk_image_create_info.imageType = ImageDimensionsToVkImageType(imageCreateInfo.Dimensions);
	vk_image_create_info.initialLayout = VK_IMAGE_LAYOUT_UNDEFINED;
	vk_image_create_info.usage = ImageUseToVkImageUsageFlagBits(imageCreateInfo.Use);
	vk_image_create_info.sharingMode = VK_SHARING_MODE_EXCLUSIVE;
	vk_image_create_info.samples = VK_SAMPLE_COUNT_1_BIT;
	vk_image_create_info.mipLevels = 1;

	GS_VK_CHECK(vkCreateImage(device->GetVkDevice().GetVkDevice(), &vk_image_create_info, ALLOCATOR, &image),
	            "Failed to allocate image!");

	VkMemoryRequirements vk_memory_requirements;
	vkGetImageMemoryRequirements(device->GetVkDevice().GetVkDevice(), image, &vk_memory_requirements);

	VkImageViewCreateInfo vk_image_view_create_info = {VK_STRUCTURE_TYPE_IMAGE_VIEW_CREATE_INFO};
	vk_image_view_create_info.format = image_format;
	vk_image_view_create_info.image = image;
	vk_image_view_create_info.viewType = ImageDimensionsToVkImageViewType(imageCreateInfo.Dimensions);
	vk_image_view_create_info.subresourceRange.aspectMask = ImageTypeToVkImageAspectFlagBits(imageCreateInfo.Type);
	vk_image_view_create_info.subresourceRange.baseArrayLayer = 0;
	vk_image_view_create_info.subresourceRange.baseMipLevel = 0;
	vk_image_view_create_info.subresourceRange.layerCount = 1;
	vk_image_view_create_info.subresourceRange.levelCount = 1;

	device->allocateMemory(&vk_memory_requirements, VK_MEMORY_PROPERTY_DEVICE_LOCAL_BIT, &imageMemory);

	vkBindImageMemory(device->GetVkDevice().GetVkDevice(), image, imageMemory, 0);

	GS_VK_CHECK(
		vkCreateImageView(device->GetVkDevice().GetVkDevice(), &vk_image_view_create_info, ALLOCATOR, &imageView),
		"Failed to create image view!");
}
