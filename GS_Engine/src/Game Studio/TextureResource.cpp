#include "TextureResource.h"

#include "stb image/stb_image.h"

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
	unsigned char * ImageData = stbi_load(FilePath, & reinterpret_cast<int &>(TextureDimensions.Width), & reinterpret_cast<int &>(TextureDimensions.Height), & reinterpret_cast<int &>(NumberOfChannels), 0);

	//Error checking.
	if (!ImageData)
	{
		return LoadFallbackResource();
	}

	return reinterpret_cast<RGB *>(ImageData);
}

RGB * TextureResource::LoadFallbackResource()
{
	return new RGB();
}