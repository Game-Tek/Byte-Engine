#include "TextureResourceManager.h"

#include <stb image/stb_image.h>

TextureResourceData::~TextureResourceData() { stbi_image_free(ImageData); }

bool TextureResourceManager::LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo)
{
	TextureResourceData data;

	auto X = 0, Y = 0, NofChannels = 0;

	//Load  the image.
	const auto img_data = stbi_load(loadResourceInfo.ResourcePath.c_str(), &X, &Y, &NofChannels, 0);

	if (img_data) //If file is valid
	{
		//Load  the image.
		data.ImageData = img_data;

		data.TextureDimensions.Width = X;
		data.TextureDimensions.Height = Y;
		data.TextureFormat = NofChannels == 4 ? RAPI::ImageFormat::RGBA_I8 : RAPI::ImageFormat::RGB_I8;
		data.ImageDataSize = NofChannels * X * Y;

		return true;
	}

	resources.insert({ loadResourceInfo.ResourceName, data });
	
	return false;
}

void TextureResourceManager::LoadFallback(const LoadResourceInfo& loadResourceInfo,	OnResourceLoadInfo& onResourceLoadInfo)
{
}

ResourceData* TextureResourceManager::GetResource(const Id& name) { return &resources[name]; }

void TextureResourceManager::ReleaseResource(const Id& resourceName) { if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); } }
