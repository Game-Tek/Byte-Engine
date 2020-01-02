#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Utility/Extent.h"

struct GS_API ImageCreateInfo
{
	Extent2D Extent = { 1280, 720 };
	Format ImageFormat = Format::RGBA_I8;
	ImageType Type = ImageType::COLOR;
	ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;
	ImageUse Use = ImageUse::INPUT_ATTACHMENT;

};

class GS_API Image
{
protected:
	Extent2D ImageExtent = { 1280, 720 };
	Format ImageFormat = Format::RGBA_I8;
	ImageType Type = ImageType::COLOR;
	ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;

public:
	Image(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType, const ImageDimensions _ID) :
		ImageExtent(_ImgExtent),
		ImageFormat(_ImgFormat),
		Type(_ImgType),
		Dimensions(_ID)
	{
	}

	Image() = default;

	INLINE Extent2D GetExtent() const { return ImageExtent; }
	INLINE Format GetImageFormat() const { return ImageFormat; }
	INLINE ImageType GetImageType() const { return Type; }
	INLINE ImageDimensions GetImageDimensions() const { return Dimensions; }

};