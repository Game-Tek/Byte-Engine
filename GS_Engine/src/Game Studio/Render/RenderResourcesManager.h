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

	GraphicsPipeline* CreatePipelineFromMaterial(Material* _Mat);

public:
	~RenderResourcesManager();

	//[[nodiscard]] const std::map<Id::HashType, Tuple<Material*, GraphicsPipeline*>>& GetMaterialMap() const { return Materials; }

	Mesh* RegisterMesh(StaticMesh* _SM);
	GraphicsPipeline* RegisterMaterial(Material* _Mat);
};
