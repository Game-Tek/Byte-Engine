#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Extent.h"

GS_CLASS Image
{
	Extent2D ImageExtent;
	ImageDimensions m_ImageDimensions;
	Format ImageFormat;
	ImageType m_ImageType;

	Image(const Extent2D _ImgExtent, const ImageDimensions _ID, const Format _ImgFormat, const ImageType _ImgType) :
		ImageExtent(_ImgExtent),
		m_ImageDimensions(_ID),
		ImageFormat(_ImgFormat),
		m_ImageType(_ImgType)
	{
	}
};