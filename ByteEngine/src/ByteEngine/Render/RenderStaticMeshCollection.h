#pragma once

#include "ByteEngine/Game/ComponentCollection.h"

#include <GTSL/Math/Vector3.h>

class RenderStaticMeshCollection : public ComponentCollection
{
public:
	RenderStaticMeshCollection() : positions(16, GetPersistentAllocator()) {}
	
	void SetMesh(ComponentReference componentReference, const GTSL::Id64 renderMeshName) { resourceNames[componentReference] = renderMeshName; }
	
	[[nodiscard]] GTSL::Ranger<GTSL::Vector3> GetPositions() const { return positions; }
	[[nodiscard]] GTSL::Ranger<const GTSL::Id64> GetResourceNames() const { return resourceNames; }
	
	ComponentReference AddMesh()
	{
		resourceNames.EmplaceBack(); return positions.EmplaceBack();
	}

	void SetPosition(ComponentReference component, GTSL::Vector3 vector3) { positions[component] = vector3; }
private:
	GTSL::Array<GTSL::Id64, 16> resourceNames;
	GTSL::Vector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
};