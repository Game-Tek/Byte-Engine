#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "ImageSize.h"

GS_CLASS Texture : public RendererObject
{
public:
	Texture(const char * ImageFilePath);
	~Texture();

	void Bind() const override;
	void ActivateTexture(unsigned short Index) const;
protected:
	ImageSize TextureDimensions;
};

