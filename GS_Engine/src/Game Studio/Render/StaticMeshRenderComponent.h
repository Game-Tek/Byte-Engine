#pragma once

#include "RenderComponent.h"

#include "Game/StaticMesh.h"

namespace RAPI {
	class RenderMesh;
}

struct StaticMeshRenderComponentCreateInfo : RenderComponentCreateInfo
{
	StaticMesh StaticMesh;
};

class StaticMeshRenderComponent final : public RenderComponent
{
	StaticMesh staticMesh;
	MaterialRenderResource* renderMaterial = nullptr;
	RAPI::RenderMesh* renderMesh = nullptr;
	
public:
	StaticMeshRenderComponent() = default;

	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderComponent"; }

	[[nodiscard]] Id64 GetRenderableType() const override { return "StaticMesh"; }

	[[nodiscard]] StaticMesh* GetStaticMesh() const { return staticMesh; }
};
