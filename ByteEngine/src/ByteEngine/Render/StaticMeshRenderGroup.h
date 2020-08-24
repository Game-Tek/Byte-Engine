#pragma once

#include <GTSL/Buffer.h>

#include "RenderGroup.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "RenderTypes.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

class RenderSystem;

class StaticMeshRenderGroup final : public RenderGroup
{
public:
	StaticMeshRenderGroup();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	void Render(GameInstance* gameInstance, const RenderSystem* renderSystem);

	struct AddStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
	};
	ComponentReference AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);

	[[nodiscard]] GTSL::Ranger<GTSL::Vector3> GetPositions() const { return positions; }
	[[nodiscard]] GTSL::Ranger<const GTSL::Id64> GetResourceNames() const { return resourceNames; }

	void SetPosition(ComponentReference component, GTSL::Vector3 vector3) { positions[component] = vector3; }
	
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, const Buffer& buffer, RenderAllocation renderAllocation, uint32 instance) : RenderSystem(renderDevice), ScratchBuffer(buffer),
		Allocation(renderAllocation), InstanceId(instance)
		{
		}
		
		RenderSystem* RenderSystem = nullptr;
		Buffer ScratchBuffer;
		RenderAllocation Allocation;
		uint32 InstanceId;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	uint32 index = 0;
	
	GTSL::Vector<Buffer, BE::PersistentAllocatorReference> meshBuffers;
	GTSL::Vector<uint32, BE::PersistentAllocatorReference> indicesOffsets;
	GTSL::Vector<uint32, BE::PersistentAllocatorReference> indicesCount;
	GTSL::Vector<RenderAllocation, BE::PersistentAllocatorReference> renderAllocations;
	GTSL::Vector<IndexType, BE::PersistentAllocatorReference> indexTypes;

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	GTSL::Vector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
};
