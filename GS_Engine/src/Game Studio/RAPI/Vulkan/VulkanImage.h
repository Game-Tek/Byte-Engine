#pragma once

#include "Core.h"

#include "RAPI/Image.h"

#include "Native/VKImageView.h"
#include "Native/VKMemory.h"
#include "Native/VKImage.h"

GS_CLASS VulkanImageBase : public Image
{
public:
	VulkanImageBase(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType, const ImageDimensions _ID);
	[[nodiscard]] virtual const VKImageView& GetVKImageView() const = 0;
};

GS_CLASS VulkanImage final : public VulkanImageBase
{
	VKImage m_Image;
	VKMemory ImageMemory;
	VKImageView ImageView;

	static VKImageCreator CreateVKImageCreator(VKDevice* _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, ImageUse _ImgUse);
	static VKMemoryCreator CreateVKMemoryCreator(VKDevice* _Device, const VKImage& _Image);
	static VKImageViewCreator CreateVKImageViewCreator(VKDevice* _Device, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const VKImage& _Image);
public:
	VulkanImage(VKDevice* _Device, const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, ImageUse _ImgUse);

	const VKImageView& GetVKImageView() const override { return ImageView; }
};
