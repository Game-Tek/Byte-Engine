#pragma once
#include "SubResourceManager.h"
#include <unordered_map>
#include "Utility/Extent.h"
#include "RAPI/RenderCore.h"

struct TextureResourceData : ResourceData
{
	byte* ImageData = nullptr;
	size_t ImageDataSize = 0;
	Extent2D TextureDimensions;
	RAPI::ImageFormat TextureFormat;
	
	~TextureResourceData();
};

class TextureResourceManager final : public SubResourceManager
{
public:
	const char* GetResourceExtension() override { return "png"; }
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	ResourceData* GetResource(const Id64& name) override;
	void ReleaseResource(const Id64& resourceName) override;
	[[nodiscard]] Id64 GetResourceType() const override { return "Texture"; }
private:
	std::unordered_map<Id64, TextureResourceData> resources;
	
};
