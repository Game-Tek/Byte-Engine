#pragma once

#include "Core.h"

#include "WorldObject.h"

#include "StaticMeshResource.h"

#include "StaticMeshRenderProxy.h"

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh(const std::string & StaticMeshAsset);
	~StaticMesh();

	//Returns a const pointer to the static mesh resource.
	const StaticMeshResource * GetMeshResource() { return MeshResource; }

private:
	//Pointer to the static mesh resource that this static mesh represents.
	StaticMeshResource * MeshResource;

	//Renderer side representation of this static mesh.
	StaticMeshRenderProxy RenderProxy;
};