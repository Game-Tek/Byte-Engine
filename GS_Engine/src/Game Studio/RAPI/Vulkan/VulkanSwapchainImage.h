#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "VulkanImage.h"

GS_CLASS VulkanSwapchainImage final : public VulkanImageBase
{
	VKImageView ImageView;

public:
	VulkanSwapchainImage(const VKDevice& _Device, VkImage _Image, Format _Format);
	~VulkanSwapchainImage() = default;
	VulkanSwapchainImage& operator=(const VulkanSwapchainImage& _) { ImageView = _.ImageView; return *this; }

	[[nodiscard]] const VKImageView& GetVk_ImageView() const override { return ImageView; };
};

