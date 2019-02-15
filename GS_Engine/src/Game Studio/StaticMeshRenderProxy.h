#pragma once

#include "Core.h"

#include "RenderProxy.h"

#include "WorldObject.h"
#include "StaticMeshResource.h"

GS_CLASS StaticMeshRenderProxy : public RenderProxy
{
public:
	StaticMeshRenderProxy(StaticMeshResource * MeshResource);
	~StaticMeshRenderProxy();
};

