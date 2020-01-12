#include "TextureResource.h"

#include "stb image/stb_image.h"
#include <fstream>

TextureResource::TextureResourceData::~TextureResourceData()
{
	stbi_image_free(ImageData);
}

bool TextureResource::LoadResource(const FString& _Path)
{
	auto X = 0, Y = 0, NofChannels = 0;

	//Load  the image.
	const auto imgdata = reinterpret_cast<char*>(stbi_load(_Path.c_str(), &X, &Y, &NofChannels, 0));
	
	if (imgdata) //If file is valid
	{
		//Load  the image.
		data.ImageData = imgdata;
		
		TextureDimensions.Width = X;
		TextureDimensions.Height = Y;
		TextureFormat = NofChannels == 4 ? Format::RGBA_I8 : Format::RGB_I8;

		//SCAST(TextureResourceData*, Data)->imageDataSize = X * Y * NofChannels;

		return true;
	}

	return false;
}

void TextureResource::LoadFallbackResource(const FString& _Path)
{
	*data.WriteTo(0, 256 * 256 * 3) = new uint8[256 * 256 * 3];

	TextureDimensions.Width = 256;
	TextureDimensions.Height = 256;

	TextureFormat = Format::RGB_I8;
	
	for (uint16 X = 0; X < 256; ++X)
	{
		for (uint16 Y = 0; Y < 256; ++Y)
		{
			data.ImageData[X + Y + 0] = X;
			data.ImageData[X + Y + 1] = Y;
			data.ImageData[X + Y + 2] = 125;
		}
	}
}
