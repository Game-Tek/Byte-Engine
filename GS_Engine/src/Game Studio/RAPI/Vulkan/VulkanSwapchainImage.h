#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "Native/Vk_ImageView.h"

GS_CLASS VulkanSwapchainImage final : public Image
{
	Vk_ImageView ImageView;

public:
	VulkanSwapchainImage(const Vk_Device& _Device, VkImage _Image, VkFormat _Format);
	~VulkanSwapchainImage() = default;
	VulkanSwapchainImage& operator=(const VulkanSwapchainImage& _) { ImageView = _.ImageView; return *this; }

};

