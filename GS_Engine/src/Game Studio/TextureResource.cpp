#include "TextureResource.h"

#include "stb image/stb_image.h"

#include "Logger.h"

TextureResource::TextureResource(const char * FilePath)
{
	Data = Load(FilePath);
}

TextureResource::~TextureResource()
{
	stbi_image_free(Data);
}

RGB * TextureResource::Load(const char * FilePath)
{
	//Load  the image.
	unsigned char * ImageData = stbi_load(FilePath, &(int &)TextureDimensions.Width, &(int &)TextureDimensions.Height, &(int &)NumberOfChannels, 0);

	//Error checking.
	if (!ImageData)
	{
		GS_LOG_WARNING("Failed to load texture: %s", FilePath)

		return LoadFallbackResource();
	}

	return (RGB *)ImageData;
}

RGB * TextureResource::LoadFallbackResource()
{
	return new RGB();
}