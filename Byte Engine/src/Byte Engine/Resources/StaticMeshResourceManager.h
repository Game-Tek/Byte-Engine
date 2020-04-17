#pragma once

#include "SubResourceManager.h"
#include <unordered_map>
#include "Byte Engine/Vertex.h"
#include "ResourceData.h"
#include <GTSL/Id.h>

struct StaticMeshResourceData final : ResourceData
{
	//Pointer to Vertex Array.
	Vertex* VertexArray = nullptr;
	//Pointer to index array.
	uint16* IndexArray = nullptr;

	//Vertex Count.
	uint16 VertexCount = 0;
	//Index Count.
	uint16 IndexCount = 0;

	~StaticMeshResourceData()
	{
		delete[] VertexArray;
		delete[] IndexArray;
	}
};

class StaticMeshResourceManager final : public SubResourceManager
{
public:
	inline static constexpr GTSL::Id64 type{ "Static Mesh" };
	
	StaticMeshResourceManager() : SubResourceManager("Static Mesh")
	{
	}

	StaticMeshResourceData* GetResource(const GTSL::Id64& resourceName)
	{
		GTSL::ReadLock<GTSL::ReadWriteMutex> lock(resourceMapMutex);
		return &resources[resourceName];
	}
	
	StaticMeshResourceData* TryGetResource(const GTSL::String& name);
	
	void ReleaseResource(const GTSL::Id64& resourceName)
	{
		resourceMapMutex.WriteLock();
		if(resources[resourceName].DecrementReferences() == 0) { resources.erase(resourceName); }
		resourceMapMutex.WriteUnlock();
	}
	
private:
	std::unordered_map<GTSL::Id64::HashType, StaticMeshResourceData> resources;
};
