#pragma once

#include "RenderComponent.h"

#include "Resources/StaticMeshResource.h"

#include "Game/StaticMesh.h"

class GS_API StaticMeshRenderComponent : public RenderComponent
{
	StaticMesh* staticMesh = nullptr;
	Mesh* renderMesh = nullptr;

	static void CreateInstanceResources(CreateInstanceResourcesInfo& _CIRI);
	static void BuildTypeInstanceSortData(BuildTypeInstanceSortDataInfo& _BTISDI);
	static void BindTypeResources(BindTypeResourcesInfo& _BTRI);
	static void DrawInstance(DrawInstanceInfo& _DII);
public:
	StaticMeshRenderComponent() = default;

	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderComponent"; }
	[[nodiscard]] RenderableInstructions GetRenderableInstructions() const override;
	[[nodiscard]] const char* GetRenderableTypeName() const override { return "StaticMesh"; }

	void SetStaticMesh(StaticMesh* _NewStaticMesh) { staticMesh = _NewStaticMesh; FlagResourceDirty(); }
	[[nodiscard]] StaticMesh* GetStaticMesh() const { return staticMesh; }
};
