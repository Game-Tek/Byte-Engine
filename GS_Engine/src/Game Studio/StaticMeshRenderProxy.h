#pragma once

#include "Core.h"

#include "WorldObject.h"

#include "MeshRenderProxy.h"

class WorldObject;

GS_CLASS StaticMeshRenderProxy : public MeshRenderProxy
{
public:
	StaticMeshRenderProxy(WorldObject * Owner);
	~StaticMeshRenderProxy();

protected:
};

