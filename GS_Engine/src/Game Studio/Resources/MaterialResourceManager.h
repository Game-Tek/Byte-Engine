#pragma once

#include "SubResourceManager.h"
#include <unordered_map>

struct MaterialResourceData final : ResourceData
{
	float Roughness;
};

class MaterialResourceManager final : public SubResourceManager
{
	std::unordered_map<Id, MaterialResourceData> resources;
	
public:
	[[nodiscard]] Id GetResourceType() const override { return "Material"; }
	const char* GetResourceExtension() override { return "gsmat"; }
	void ReleaseResource(const Id& resourceName) override;
	ResourceData* GetResource(const Id& name) override;
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
};
