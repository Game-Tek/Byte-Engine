#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "Native/Vk_ImageView.h"
#include "Native/Vk_Memory.h"

GS_CLASS VulkanImageBase : public Image
{
public:
	VulkanImageBase(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType, const ImageDimensions _ID);
	[[nodiscard]] virtual const Vk_ImageView& GetVk_ImageView() const = 0;
};

GS_CLASS VulkanImage final : public VulkanImageBase
{
	Vk_Image m_Image;
	Vk_Memory ImageMemory;
	Vk_ImageView ImageView;

public:
	VulkanImage(const Vk_Device& _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, ImageUse _ImgUse);

	INLINE VkImageView GetVkImageView() const { return ImageView; }

	const Vk_ImageView& GetVk_ImageView() const override { return ImageView; }
};
