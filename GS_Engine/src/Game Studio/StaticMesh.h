#pragma once

#include "Core.h"

#include "WorldObject.h"

#include "StaticMeshResource.h"

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh(const std::string & StaticMeshAsset);
	~StaticMesh();

	//Returns a const pointer to the static mesh resource.
	const StaticMeshResource * GetMeshResource() { return MeshResource; }

protected:
	//Pointer to the static mesh resource that this static mesh represents.
	StaticMeshResource * MeshResource;
};