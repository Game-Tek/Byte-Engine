#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Utility/Extent.h"

namespace RAPI
{

	struct ImageCreateInfo
	{
		Extent3D Extent = { 1280, 720 };
		Format ImageFormat = Format::RGBA_I8;
		ImageType Type = ImageType::COLOR;
		ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;
		ImageUse Use = ImageUse::INPUT_ATTACHMENT;
	};

	class Image
	{
	protected:
		Extent3D ImageExtent = { 1280, 720 };
		Format ImageFormat = Format::RGBA_I8;
		ImageType Type = ImageType::COLOR;
		ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;

	public:

		explicit Image(const ImageCreateInfo& imageCreateInfo) :
			ImageExtent(imageCreateInfo.Extent),
			ImageFormat(imageCreateInfo.ImageFormat),
			Type(imageCreateInfo.Type),
			Dimensions(imageCreateInfo.Dimensions)
		{
		}

		Image() = default;

		INLINE Extent3D GetExtent() const { return ImageExtent; }
		INLINE Format GetImageFormat() const { return ImageFormat; }
		INLINE ImageType GetImageType() const { return Type; }
		INLINE ImageDimensions GetImageDimensions() const { return Dimensions; }
	};

}
