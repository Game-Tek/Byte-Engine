#pragma once

#include "Core.h"

#include "RendererObject.h"

#include "ImageSize.h"

class Texture : public RendererObject
{
public:
	Texture(const char * ImageFilePath);
	~Texture();

	void Bind() const;
	void ActivateTexture(unsigned short Index) const;
protected:
	ImageSize TextureDimensions;
};

