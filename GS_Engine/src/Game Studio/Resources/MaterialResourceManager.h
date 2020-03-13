#pragma once

#include "SubResourceManager.h"
#include <unordered_map>

struct MaterialResourceData final : ResourceData
{
	float Roughness;
};

class MaterialResourceManager : public SubResourceManager
{
	std::unordered_map<Id, MaterialResourceData> resources;
public:
	void ReleaseResource(const Id& resourceName) override;
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
};
