#pragma once

#include "SubResourceManager.h"
#include <unordered_map>
#include "ResourceData.h"

struct MaterialResourceData final : ResourceData
{
	float Roughness;
};

class MaterialResourceManager final : public SubResourceManager
{
	std::unordered_map<GTSL::Id64::HashType, MaterialResourceData> resources;
	
public:
	inline static constexpr GTSL::Id64 type{ "Material" };
	
	MaterialResourceManager() : SubResourceManager("Material")
	{
	}

	MaterialResourceData* GetResource(const GTSL::Id64& resourceName)
	{
		ReadLock<ReadWriteMutex> lock(resourceMapMutex);
		return &resources[resourceName];
	}
	
	MaterialResourceData* TryGetResource(const GTSL::String& name);
	
	void ReleaseResource(const GTSL::Id64& resourceName)
	{
		resourceMapMutex.WriteLock();
		if (resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); }
		resourceMapMutex.WriteUnlock();
	}
};
