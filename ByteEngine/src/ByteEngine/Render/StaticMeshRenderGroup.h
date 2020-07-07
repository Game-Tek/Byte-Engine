#pragma once

#include "RenderGroup.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

class StaticMeshRenderGroup final : public RenderGroup
{
public:
	[[nodiscard]] const char* GetName() const override { return "StaticMeshRenderGroup"; }

	void AddStaticMesh(uint32 componentReference, class RenderStaticMeshCollection* renderStaticMeshCollection);
	
private:
	void onStaticMeshLoaded(StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	void* data;
};
