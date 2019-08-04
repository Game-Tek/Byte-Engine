#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Extent.h"

//Represents a resource utilized by the rendering API for storing and referencing attachments. Which are images which hold some information which the GPU writes info to.
GS_CLASS Image
{
	Extent2D ImageExtent;
	Format ImageFormat;
	ImageType m_ImageType;

	//Defines the operation that should be run when the attachment is loaded for rendering.
	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	//Defines the operation that should be run when the attachment is done being rendered to.
	StoreOperations StoreOperation = StoreOperations::STORE;
	//Layout of the attachment when first used in the render pass.
	ImageLayout	InitialLayout = ImageLayout::GENERAL;
	//Layout of the attachment after use in the render pass.
	ImageLayout	FinalLayout = ImageLayout::GENERAL;

	Image(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageType _ImgType) :
		ImageExtent(_ImgExtent),
		ImageFormat(_ImgFormat),
		m_ImageType(_ImgType)
	{
	}
};