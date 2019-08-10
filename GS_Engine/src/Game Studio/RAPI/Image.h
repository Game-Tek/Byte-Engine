#pragma once

#include "Core.h"

#include "RenderCore.h"
#include "Extent.h"

GS_STRUCT ImageCreateInfo
{
	Extent2D Extent = {1280, 720 };
	Format ImageFormat = Format::RGBA_I8;
	ImageType Type = ImageType::COLOR;
	ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;
	ImageUse Use = ImageUse::COLOR_ATTACHMENT;

	//Defines the operation that should be run when the attachment is loaded for rendering.
	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	//Defines the operation that should be run when the attachment is done being rendered to.
	StoreOperations StoreOperation = StoreOperations::STORE;
	//Layout of the attachment when first used in the render pass.
	ImageLayout	InitialLayout = ImageLayout::GENERAL;
	//Layout of the attachment after use in the render pass.
	ImageLayout	FinalLayout = ImageLayout::GENERAL;
};

//Represents a resource utilized by the rendering API for storing and referencing attachments. Which are images which hold some information which the GPU writes info to.
GS_CLASS Image
{
protected:
	Extent2D ImageExtent = { 1280, 720 };
	Format ImageFormat = Format::RGB_F32;
	ImageType m_ImageType = ImageType::COLOR;
	ImageDimensions Dimensions = ImageDimensions::IMAGE_2D;
	ImageUse Use = ImageUse::COLOR_ATTACHMENT;

	//Defines the operation that should be run when the attachment is loaded for rendering.
	LoadOperations LoadOperation = LoadOperations::UNDEFINED;
	//Defines the operation that should be run when the attachment is done being rendered to.
	StoreOperations StoreOperation = StoreOperations::STORE;
	//Layout of the attachment when first used in the render pass.
	ImageLayout	InitialLayout = ImageLayout::GENERAL;
	//Layout of the attachment after use in the render pass.
	ImageLayout	FinalLayout = ImageLayout::GENERAL;

public:
	Image(const Extent2D _ImgExtent, const Format _ImgFormat, const ImageDimensions _ID, const ImageType _ImgType, const ImageUse _ImgUse, LoadOperations _LO, StoreOperations _SO, ImageLayout _IL, ImageLayout _FL) :
		ImageExtent(_ImgExtent),
		ImageFormat(_ImgFormat),
		m_ImageType(_ImgType),
		Dimensions(_ID),
		Use(_ImgUse),
		LoadOperation(_LO),
		StoreOperation(_SO),
		InitialLayout(_IL),
		FinalLayout(_IL)
	{
	}

	Image() = default;

	INLINE Extent2D GetExtent() const { return ImageExtent; }
	INLINE Format GetImageFormat() const { return ImageFormat; }
	INLINE ImageType GetImageType() const { return m_ImageType; }
	INLINE ImageDimensions GetImageDimensions() const { return Dimensions; }
	INLINE ImageUse GetImageUse() const { return Use; }
	INLINE LoadOperations GetImageLoadOperation() const { return LoadOperation; }
	INLINE StoreOperations GetImageStoreOperation() const { return  StoreOperation; }
	INLINE ImageLayout GetImageInitialLayout() const { return InitialLayout; }
	INLINE ImageLayout GetImageFinalLayout() const { return FinalLayout; }
};