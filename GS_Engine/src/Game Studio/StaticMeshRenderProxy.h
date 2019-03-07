#pragma once

#include "Core.h"

#include "MeshRenderProxy.h"

class WorldObject;

GS_CLASS StaticMeshRenderProxy : public MeshRenderProxy
{
public:
	StaticMeshRenderProxy(const void * MeshData, size_t DataSize, const void * IndexData, uint32 IndexCount);
	explicit StaticMeshRenderProxy(WorldObject * Owner);
	~StaticMeshRenderProxy() = default;

	virtual void Draw() override;

protected:
};

