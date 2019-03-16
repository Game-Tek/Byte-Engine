#pragma once

#include "Core.h"

#include "Resource.h"

#include "RGB.h"

#include "ImageSize.h"

GS_CLASS TextureResource : public Resource
{
public:
	TextureResource(const char * FilePath);
	~TextureResource();

	size_t GetDataSize() const override{ return sizeof(*Data); }

protected:
	RGB * Data;

	//Used to hold the texture's dimensions once it's been loaded.
	ImageSize TextureDimensions;

	//Used to hold the number of channels this texture has.
	unsigned char NumberOfChannels = 0;

	RGB * Load(const char * FilePath);

	RGB * LoadFallbackResource();
};

