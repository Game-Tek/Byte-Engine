#pragma once

#include "RenderComponent.h"

#include "Resources/StaticMesh.h"

class GS_API StaticMeshRenderComponent : public RenderComponent
{
	StaticMesh* m_StaticMesh = nullptr;

public:
	StaticMeshRenderComponent() = default;

	const char* GetName() const override { return "StaticMeshRenderComponent"; }

	[[nodiscard]] RenderableInstructions GetRenderableInstructions() const override;

	void SetStaticMesh(StaticMesh* _NewStaticMesh) { m_StaticMesh = _NewStaticMesh; }
	[[nodiscard]] StaticMesh* GetStaticMesh() const { return m_StaticMesh; }
};