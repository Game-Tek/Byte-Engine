#pragma once

#include "Core.h"

#include "WorldObject.h"

#include "StaticMeshResource.h"

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh(const std::string & Path);
	~StaticMesh();

private:
	StaticMeshResource * MeshData;
};