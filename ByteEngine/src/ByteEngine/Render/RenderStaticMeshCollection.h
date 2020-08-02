#pragma once

#include "ByteEngine/Game/ComponentCollection.h"

class RenderStaticMeshCollection : public ComponentCollection
{
public:
	void SetMesh(ComponentReference componentReference, const GTSL::Ranger<const UTF8>& renderMeshName) { ResourceNames.EmplaceBack(renderMeshName); }

	GTSL::Array<GTSL::Id64, 16> ResourceNames;
};