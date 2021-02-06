#pragma once

#include <GTSL/FlatHashMap.h>

#include "MaterialSystem.h"

using MeshHandle = uint32;

class RenderState : public Object
{
public:
	RenderState() : Object("RenderState") { availableMaterials.Initialize(32, GetPersistentAllocator()); meshesPerMaterial.Initialize(32, GetPersistentAllocator()); }

	void AddMaterial(MaterialHandle materialHandle)
	{
		availableMaterials.Emplace(materialHandle(), materialHandle);
		meshesPerMaterial.Emplace(materialHandle()).Initialize(32, GetPersistentAllocator());
	}
	
	void RemoveMaterial(MaterialHandle materialHandle)
	{
		availableMaterials.Remove(materialHandle());
	}

	void AddMesh(const MeshHandle meshHandle, const MaterialHandle materialHandle)
	{
		auto result = meshesPerMaterial.TryEmplace(materialHandle());
		
		if(result.State()) { //if material is registered 
			result.Get().EmplaceBack(meshHandle);
		}
		else [[likely]] { //if material doesn't exist
			auto& meshList = meshesPerMaterial.Emplace(materialHandle());
			meshList.Initialize(32, GetPersistentAllocator());
			meshList.EmplaceBack(meshHandle);
		}
	}

private:
	GTSL::FlatHashMap<MaterialHandle, BE::PAR> availableMaterials;
	GTSL::FlatHashMap<GTSL::Vector<MeshHandle, BE::PAR>, BE::PAR> meshesPerMaterial;
};
