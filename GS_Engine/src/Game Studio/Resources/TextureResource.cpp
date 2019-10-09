#include "TextureResource.h"

#include "stb image/stb_image.h"

TextureResource::~TextureResource()
{
	stbi_image_free(Data);
}

bool TextureResource::LoadResource(const FString& _Path)
{
	int32 X, Y, NofChannels;

	//Load  the image.
	Data = stbi_load(_Path.c_str(), &X, &Y, &NofChannels, 0);

	TextureDimensions.Width = X;
	TextureDimensions.Height = Y;
	TextureFormat = NofChannels == 4 ? Format::RGBA_I8 : Format::RGB_I8;

	//Error checking.
	return Data != nullptr;
}

void TextureResource::LoadFallbackResource(const FString& _Path)
{
	Data = new uint8[256 * 256 * 3];

	for (uint16 X = 0; X < 256; ++X)
	{
		for (uint16 Y = 0; Y < 256; ++Y)
		{
			SCAST(uint8*, Data)[X + Y + 0] = X;
			SCAST(uint8*, Data)[X + Y + 1] = Y;
			SCAST(uint8*, Data)[X + Y + 2] = 125;
		}
	}
}
