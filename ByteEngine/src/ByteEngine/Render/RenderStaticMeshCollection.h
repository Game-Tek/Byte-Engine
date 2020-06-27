#pragma once

#include "ByteEngine/Game/ComponentCollection.h"

class RenderStaticMeshCollection : public ComponentCollection
{
public:
	[[nodiscard]] const char* GetName() const override { return "RenderStaticMeshCollection"; }
	
	ComponentReference CreateInstance(const CreateInstanceInfo& createInstanceInfo) override { return 0; }
	void DestroyInstance(const DestroyInstanceInfo& destroyInstancesInfo) override {};

	void SetMesh(ComponentReference componentReference, const GTSL::StaticString<128>& renderMeshName) { Strings.EmplaceBack(renderMeshName); }

	GTSL::Array<GTSL::StaticString<128>, 16> Strings;
};