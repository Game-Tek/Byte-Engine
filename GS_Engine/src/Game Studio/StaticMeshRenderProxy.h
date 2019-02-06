#pragma once

#include "Core.h"

#include "RenderProxy.h"

#include "WorldObject.h"

GS_CLASS StaticMeshRenderProxy : public RenderProxy
{
public:
	StaticMeshRenderProxy(WorldObject * Owner);
	~StaticMeshRenderProxy();
};

