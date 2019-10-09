#pragma once

#include <map>

#include "Containers/Id.h"
#include "Containers/FVector.hpp"

#include "Material.h"
#include "Containers/Tuple.h"

class GraphicsPipeline;
class StaticMesh;
class Mesh;

class RenderResourcesManager
{
	// MATERIALS
	std::map<Id::HashType, Tuple<Material*, GraphicsPipeline*>> Materials;
	FVector<GraphicsPipeline*> Pipelines;
	// MATERIALS

	// MESHES
	std::map<StaticMesh*, Mesh*> Meshes;
	//FVector<Mesh*> Meshes;
	// MESHES

public:
	~RenderResourcesManager();

	template<class T>
	Material* CreateMaterial()
	{
		Material* NewMaterial = new T();
		if (!Materials.try_emplace(Id(NewMaterial->GetMaterialName()).GetID(), NewMaterial).second)
		{
			delete NewMaterial;
			NewMaterial = nullptr;
		}
		return NewMaterial;
	}

	Material* GetMaterial(const char* _MaterialName)
	{
		return Materials[Id(_MaterialName).GetID()].First;
	}

	[[nodiscard]] const std::map<Id::HashType, Tuple<Material*, GraphicsPipeline*>>& GetMaterialMap() const { return Materials; }

	Mesh* CreateMesh(StaticMesh* _SM);
};
