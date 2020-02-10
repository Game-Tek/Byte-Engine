#pragma once

#include "RenderComponent.h"

#include "Game/StaticMesh.h"

namespace RAPI {
	class RenderMesh;
}

struct StaticMeshRenderComponentCreateInfo : RenderComponentCreateInfo
{
	StaticMesh* StaticMesh = nullptr;
};

class StaticMeshRenderComponent final : public RenderComponent
{
	StaticMesh* staticMesh = nullptr;
	class MaterialRenderResource* renderMaterial = nullptr;
	RAPI::RenderMesh* renderMesh = nullptr;
	
public:
	StaticMeshRenderComponent() = default;

	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderComponent"; }

	[[nodiscard]] Id GetRenderableType() const override { return "StaticMesh"; }

	[[nodiscard]] StaticMesh* GetStaticMesh() const { return staticMesh; }
};
