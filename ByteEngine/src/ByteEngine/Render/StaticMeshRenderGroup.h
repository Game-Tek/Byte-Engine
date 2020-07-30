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
		LoadInfo(RenderSystem* renderDevice, const Buffer& buffer, uint32 size, uint32 offset, uint64 allocId) : RenderSystem(renderDevice), ScratchBuffer(buffer),
		BufferSize(size), BufferOffset(offset), AllocationId(allocId)
		{
		}
		
		RenderSystem* RenderSystem = nullptr;
		Buffer ScratchBuffer;
		uint32 BufferSize, BufferOffset;
		uint64 AllocationId;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	GTSL::Vector<Buffer, BE::PersistentAllocatorReference> meshBuffers;
};
