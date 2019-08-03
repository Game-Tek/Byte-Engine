#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Extent.h"

//Represents a resource utilized by the rendering API for storing and referencing attachments. Which are images which hold some information which the GPU writes info to.
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