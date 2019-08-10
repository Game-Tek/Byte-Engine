#pragma once

#include "Core.h"

#include "RAPI/Vulkan/VulkanBase.h"

#include "Vk_Image.h"

MAKE_VK_HANDLE(VkImageView)

enum VkImageViewType;
enum VkFormat;

GS_CLASS Vk_ImageView final : public VulkanObject
{
	VkImageView ImageView = nullptr;

public:
	Vk_ImageView(const Vk_Device& _Device, VkImage _Image, VkImageViewType _IVT, VkFormat _Format, unsigned _IAF);
	~Vk_ImageView();

	Vk_ImageView& operator=(const Vk_ImageView& _) { ImageView = _.ImageView; return *this; }

	INLINE operator VkImageView() const { return ImageView; }
};