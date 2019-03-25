#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "ImageSize.h"

GS_CLASS Texture : public RendererObject
{
public:
	Texture(const ImageSize & TextureSize, uint32 TextureColorComponents, uint32 PixelDataFormat, uint32 PixelDataType);
	Texture(const char * ImageFilePath);
	~Texture();

	void Bind() const override;
	void UnBind() const override;

	static void SetActiveTextureUnit(uint8 Index);
protected:
	ImageSize TextureDimensions;
};

