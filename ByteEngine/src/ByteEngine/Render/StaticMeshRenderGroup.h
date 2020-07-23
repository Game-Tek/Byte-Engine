#pragma once

#include "RenderGroup.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "RenderTypes.h"

class StaticMeshRenderGroup final : public RenderGroup
{
public:
	StaticMeshRenderGroup();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown() override;

	struct AddStaticMeshInfo
	{
		ComponentReference ComponentReference = 0;
		class RenderSystem* RenderSystem = nullptr;
		const class RenderStaticMeshCollection* RenderStaticMeshCollection = nullptr;
		class StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
	};
	void AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);
	
private:
	struct LoadInfo
	{
		LoadInfo(RenderSystem* renderDevice, const Buffer& buffer) : RenderSystem(renderDevice), ScratchBuffer(buffer)
		{
		}
		
		RenderSystem* RenderSystem = nullptr;
		Buffer ScratchBuffer;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	GTSL::Vector<Buffer, BE::PersistentAllocatorReference> meshBuffers;
};
