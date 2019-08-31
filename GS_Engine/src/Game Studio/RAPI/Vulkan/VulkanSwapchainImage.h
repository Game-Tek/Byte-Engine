#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "VulkanImage.h"

GS_CLASS VulkanSwapchainImage final : public VulkanImageBase
{
	VKImageView ImageView;

	static VKImageViewCreator CreateImageView(VKDevice* _Device, VkImage _Image, const Format _Format);
public:
	VulkanSwapchainImage(VKDevice* _Device, VkImage _Image, Format _Format);
	~VulkanSwapchainImage() = default;

	[[nodiscard]] const VKImageView& GetVKImageView() const override { return ImageView; };
};

