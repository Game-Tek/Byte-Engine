#pragma once

#include "SubResourceManager.h"
#include <unordered_map>
#include <GTSL/Extent.h>
#include "ResourceData.h"
#include <GTSL/Id.h>

struct TextureResourceData final : ResourceHandle
{
	byte* ImageData = nullptr;
	size_t ImageDataSize = 0;
	GTSL::Extent2D TextureDimensions;
	//GAL::ImageFormat TextureFormat;
	
	~TextureResourceData();
};

class TextureResourceManager final : public SubResourceManager
{
public:
	TextureResourceManager() : SubResourceManager("Texture")
	{
	}	
};
