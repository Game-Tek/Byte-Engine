#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "VulkanImage.h"

GS_CLASS VulkanSwapchainImage final : public VulkanImageBase
{
	Vk_ImageView ImageView;

public:
	VulkanSwapchainImage(const Vk_Device& _Device, VkImage _Image, Format _Format);
	~VulkanSwapchainImage() = default;
	VulkanSwapchainImage& operator=(const VulkanSwapchainImage& _) { ImageView = _.ImageView; return *this; }

	[[nodiscard]] const Vk_ImageView& GetVk_ImageView() const override { return ImageView; };
};

