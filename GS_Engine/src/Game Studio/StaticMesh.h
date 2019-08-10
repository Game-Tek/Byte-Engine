#pragma once

#include "Core.h"

#include "WorldObject.h"
#include "Containers/FString.h"

GS_CLASS StaticMesh : public WorldObject
{
public:
	StaticMesh();
	explicit StaticMesh(const FString & StaticMeshAsset);
	~StaticMesh();

protected:
};