#pragma once

#include "ByteEngine/Game/ComponentCollection.h"

class RenderStaticMeshCollection : public ComponentCollection
{
public:
	[[nodiscard]] const char* GetName() const override { return "RenderStaticMeshCollection"; }
	
	ComponentReference CreateInstance(const CreateInstanceInfo& createInstanceInfo) override { return 0; }
	void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) override {};

	void SetMesh(ComponentReference componentReference, const GTSL::Ranger<const UTF8>& renderMeshName) { ResourceNames.EmplaceBack(renderMeshName); }

	GTSL::Array<GTSL::Id64, 16> ResourceNames;
};