#pragma once

#include "SubResourceManager.h"
#include <unordered_map>
#include "ByteEngine/Vertex.h"
#include "ResourceData.h"
#include <GTSL/Id.h>

struct StaticMeshResourceData final : ResourceHandle
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
	StaticMeshResourceManager() : SubResourceManager("Static Mesh")
	{
	}
	
private:
	std::unordered_map<GTSL::Id64::HashType, StaticMeshResourceData> resources;
};
