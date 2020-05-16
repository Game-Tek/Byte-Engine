#pragma once

#include "SubResourceManager.h"
#include <unordered_map>
#include <GTSL/Extent.h>
#include "ResourceData.h"
#include <GTSL/Id.h>

struct TextureResourceData final : ResourceData
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
	
	TextureResourceData* GetResource(const GTSL::Id64& name)
	{
		GTSL::ReadLock<GTSL::ReadWriteMutex> lock(resourceMapMutex);
		return &resources[name];
	}

	TextureResourceData* TryGetResource(const GTSL::String& name);
	
	void ReleaseResource(const GTSL::Id64& resourceName)
	{
		resourceMapMutex.WriteLock();
		if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); }
		resourceMapMutex.WriteUnlock();
	}
	
private:
	std::unordered_map<GTSL::Id64::HashType, TextureResourceData> resources;
	
};
