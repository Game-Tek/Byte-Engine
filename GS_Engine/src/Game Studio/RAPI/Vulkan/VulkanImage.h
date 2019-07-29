#pragma once

#include "Core.h"

#include "VulkanBase.h"
#include "RAPI/Image.h"
#include "Native/Vk_ImageView.h"

GS_CLASS VulkanImage final : public Image
{
	Vk_ImageView ImageView;

public:
	VulkanImage();

	INLINE VkImageView GetVkImageView() const { return ImageView.GetVkImageView(); }
};
