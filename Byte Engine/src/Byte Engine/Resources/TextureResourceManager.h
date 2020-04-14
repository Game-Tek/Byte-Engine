#pragma once
#include "SubResourceManager.h"
#include <unordered_map>
#include <GAL/RenderCore.h>
#include <GTSL/Extent.h>

struct TextureResourceData : ResourceData
{
	byte* ImageData = nullptr;
	size_t ImageDataSize = 0;
	Extent2D TextureDimensions;
	GAL::ImageFormat TextureFormat;
	
	~TextureResourceData();
};

class TextureResourceManager final : public SubResourceManager
{
public:
	const char* GetResourceExtension() override { return "png"; }
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	ResourceData* GetResource(const GTSL::Id64& name) override;
	void ReleaseResource(const GTSL::Id64& resourceName) override;
	[[nodiscard]] GTSL::Id64 GetResourceType() const override { return "Texture"; }
private:
	std::unordered_map<GTSL::Id64, TextureResourceData> resources;
	
};
