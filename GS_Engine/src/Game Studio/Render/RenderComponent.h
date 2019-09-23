#pragma once

#include "Core.h"

#include "Game/Component.h"

#include "Resources/StaticMesh.h"

class GS_API RenderComponent : public Component
{
protected:
	//Defines whether this render component updates it's properties during it's lifetime or if the settings found on creation are the ones that will be used for all it's lifetime.
	//All other properties won't be updated during runtime if this flag is set to true, unless stated otherwise.
	bool IsDynamic = false;

	//Determines whether this object will be drawn on this update. DOES NOT DEPEND ON IsDynamic.
	bool Render = true;

public:
	bool GetIsDynamic() const { return IsDynamic; }
	bool GetRender() const { return Render; }
};

class GS_API StaticMeshRenderComponent : public RenderComponent
{
	StaticMesh* m_StaticMesh = nullptr;

public:
	StaticMeshRenderComponent() = default;

	const char* GetName() const override { return "StaticMeshRenderComponent"; }

	void SetStaticMesh(StaticMesh* _NewStaticMesh) { m_StaticMesh = _NewStaticMesh; }
};