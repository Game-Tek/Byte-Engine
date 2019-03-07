#pragma once

#include "Core.h"

#include "WorldObject.h"
#include "RenderProxy.h"

class MeshRenderProxy;

GS_CLASS MeshObject : public WorldObject
{
public:
	MeshObject() = default;
	explicit MeshObject(MeshRenderProxy * RenderProxy);
	~MeshObject();

	RenderProxy * GetRenderProxy() const { return RenderProxy; }

protected:
	RenderProxy * RenderProxy = nullptr;
};

