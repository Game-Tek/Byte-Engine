#pragma once

#include "Core.h"

#include "WorldObject.h"
#include "Containers/FString.h"

class StaticMeshResource;

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh();
	explicit StaticMesh(const FString & StaticMeshAsset);
	~StaticMesh();

	//Returns a const pointer to the static mesh resource.
	const StaticMeshResource * GetMeshResource() const { return MeshResource; }

protected:
	//Pointer to the static mesh resource that this static mesh represents.
	StaticMeshResource * MeshResource = nullptr;
};