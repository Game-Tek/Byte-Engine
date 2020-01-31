#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "VulkanImage.h"

class VulkanSwapchainImage final : public VulkanImageBase
{
public:
	VulkanSwapchainImage(VulkanRenderDevice* device, const ImageCreateInfo& imageCreateInfo, VkImage image);
	~VulkanSwapchainImage() = default;
};
