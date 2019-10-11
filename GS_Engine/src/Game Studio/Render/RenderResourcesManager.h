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
	std::map<Id::HashType, GraphicsPipeline*> Pipelines;
	// MATERIALS

	// MESHES
	std::map<StaticMesh*, Mesh*> Meshes;
	//FVector<Mesh*> Meshes;
	// MESHES

	void RegisterMaterial(Material* _Mat)
	{
		if (Pipelines.find(Id(_Mat->GetMaterialName()).GetID()) != Pipelines.end())
		{
			//If material exists
			
		}
		else
		{
			//If material doesn't exist
		}
	}
public:
	~RenderResourcesManager();


	//[[nodiscard]] const std::map<Id::HashType, Tuple<Material*, GraphicsPipeline*>>& GetMaterialMap() const { return Materials; }

	Mesh* CreateMesh(StaticMesh* _SM);
};
