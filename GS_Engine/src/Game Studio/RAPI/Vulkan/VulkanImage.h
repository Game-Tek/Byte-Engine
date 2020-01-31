#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include <RAPI/Vulkan/Vulkan.h>

class VulkanRenderDevice;

class VulkanImageBase : public Image
{
protected:
	VkImageView imageView = nullptr;
public:
	VulkanImageBase(const ImageCreateInfo& imageCreateInfo);
	[[nodiscard]] virtual const VkImageView& GetVkImageView() const { return imageView; }
};

class VulkanImage final : public VulkanImageBase
{
	VkImage image = nullptr;
	VkDeviceMemory imageMemory = nullptr;

public:
	VulkanImage(VulkanRenderDevice* device, const ImageCreateInfo& imageCreateInfo);

	[[nodiscard]] VkImage GetVkImage() const { return image; }
};
