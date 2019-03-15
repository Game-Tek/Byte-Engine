#pragma once

#include "Core.h"

#include "MeshRenderProxy.h"

class WorldObject;

GS_CLASS StaticMeshRenderProxy : public MeshRenderProxy
{
public:
	StaticMeshRenderProxy(WorldObject * Owner, const void * MeshData, size_t DataSize, const void * IndexData, uint32 IndexCount);
	~StaticMeshRenderProxy() = default;

	virtual void Draw() override;

protected:
};

