#pragma once

#include "SubResourceManager.h"

struct MaterialResourceData final : ResourceData
{
	float Roughness;
};

class MaterialResourceManager : public SubResourceManager
{
	void ReleaseResource(const Id& resourceName) override;
	bool LoadResource(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
	void LoadFallback(const LoadResourceInfo& loadResourceInfo, OnResourceLoadInfo& onResourceLoadInfo) override;
};
