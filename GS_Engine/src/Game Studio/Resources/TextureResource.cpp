#include "TextureResource.h"

#include "stb image/stb_image.h"

TextureResource::~TextureResource()
{
	stbi_image_free(Data);
}

bool TextureResource::LoadResource(const FString& _Path)
{
	int32 X = 0, Y = 0, NofChannels = 0;

	//Load  the image.
	*Data->WriteTo(0, X * Y * NofChannels) = stbi_load(_Path.c_str(), &X, &Y, &NofChannels, 0);

	TextureDimensions.Width = X;
	TextureDimensions.Height = Y;
	TextureFormat = NofChannels == 4 ? Format::RGBA_I8 : Format::RGB_I8;

	//Error checking.
	return Data != nullptr;
}

void TextureResource::LoadFallbackResource(const FString& _Path)
{
	*Data->WriteTo(0, 256 * 256 * 3) = new uint8[256 * 256 * 3];

	for (uint16 X = 0; X < 256; ++X)
	{
		for (uint16 Y = 0; Y < 256; ++Y)
		{
			SCAST(TextureResourceData*, Data)->ImageData[X + Y + 0] = X;
			SCAST(TextureResourceData*, Data)->ImageData[X + Y + 1] = Y;
			SCAST(TextureResourceData*, Data)->ImageData[X + Y + 2] = 125;
		}
	}
}
