#include "TextureResourceManager.h"

#include <stb image/stb_image.h>
#include <GTSL/System.h>
#include <GTSL/Id.h>

TextureResourceData::~TextureResourceData() { stbi_image_free(ImageData); }

TextureResourceData* TextureResourceManager::TryGetResource(const GTSL::String& name)
{
	const GTSL::Id64 hashed_name(name);

	{
		resourceMapMutex.ReadLock();
		if (resources.contains(hashed_name))
		{
			resourceMapMutex.ReadUnlock();
			resourceMapMutex.WriteLock();
			auto& res = resources.at(hashed_name);
			res.IncrementReferences();
			resourceMapMutex.WriteUnlock();
			return &res;
		}
		resourceMapMutex.ReadUnlock();
	}

	TextureResourceData data;

	auto X = 0, Y = 0, NofChannels = 0;

	GTSL::String path(255, &transientAllocator);
	GTSL::System::GetRunningPath(path);
	path += "resources/";
	path += name;
	path += '.';
	path += "png";
	
	const auto img_data = stbi_load(path.c_str(), &X, &Y, &NofChannels, 0);

	if (img_data) //If file is valid
	{
		//Load  the image.
		data.ImageData = img_data;

		data.TextureDimensions.Width = X;
		data.TextureDimensions.Height = Y;
		//data.TextureFormat = NofChannels == 4 ? GAL::ImageFormat::RGBA_I8 : GAL::ImageFormat::RGB_I8;
		data.ImageDataSize = NofChannels * X * Y;

		return nullptr;
	}

	resourceMapMutex.WriteLock();
	resources.emplace(hashed_name, GTSL::MakeTransferReference(data));
	resourceMapMutex.WriteUnlock();
	
	return nullptr;
}
