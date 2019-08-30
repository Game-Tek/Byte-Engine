#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "Native/VKImageView.h"
#include "Native/VKMemory.h"

GS_CLASS VulkanImageBase : public Image
{
public:
	VulkanImageBase(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType, const ImageDimensions _ID);
	[[nodiscard]] virtual const VKImageView& GetVk_ImageView() const = 0;
};

GS_CLASS VulkanImage final : public VulkanImageBase
{
	VKImage m_Image;
	VKMemory ImageMemory;
	VKImageView ImageView;

public:
	VulkanImage(const VKDevice& _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, ImageUse _ImgUse);

	INLINE VkImageView GetVkImageView() const { return ImageView; }

	const VKImageView& GetVk_ImageView() const override { return ImageView; }
};
