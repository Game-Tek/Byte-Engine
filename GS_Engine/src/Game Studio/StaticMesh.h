#pragma once

#include "Core.h"

#include "MeshObject.h"

#include <string>

class StaticMeshResource;

GS_CLASS StaticMesh : public MeshObject
{
public:
	StaticMesh();
	explicit StaticMesh(const std::string & StaticMeshAsset);
	~StaticMesh();

	//Returns a const pointer to the static mesh resource.
	const StaticMeshResource * GetMeshResource() const { return MeshResource; }

protected:
	//Pointer to the static mesh resource that this static mesh represents.
	StaticMeshResource * MeshResource = nullptr;
};