#pragma once

#include "Core.h"

#include "WorldObject.h"

class MeshRenderProxy;

GS_CLASS MeshObject : public WorldObject
{
public:
	MeshObject() = default;
	explicit MeshObject(MeshRenderProxy * RenderProxy);
	~MeshObject();

	MeshRenderProxy * GetRenderProxy() const { return RenderProxy; }

protected:
	MeshRenderProxy * RenderProxy = nullptr;
};

