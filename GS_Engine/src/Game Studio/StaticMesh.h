#pragma once

#include "Core.h"

#include "WorldObject.h"

class String;
class StaticMeshRenderProxy;
class StaticMeshResource;

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh();
	explicit StaticMesh(const String & StaticMeshAsset);
	~StaticMesh();

	//Returns a const pointer to the static mesh resource.
	const StaticMeshResource * GetMeshResource() const { return MeshResource; }

	RenderProxy * GetRenderProxy() override { return (RenderProxy *)(MeshRenderProxy); }

protected:
	//Pointer to the static mesh resource that this static mesh represents.
	StaticMeshResource * MeshResource = nullptr;

	StaticMeshRenderProxy * MeshRenderProxy = nullptr;
};