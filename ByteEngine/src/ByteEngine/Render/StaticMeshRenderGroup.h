#pragma once

#include <GTSL/Buffer.h>

#include "RenderGroup.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "RenderTypes.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

class StaticMeshRenderGroup final : public RenderGroup
{
public:
	StaticMeshRenderGroup();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown() override;

	struct AddStaticMeshInfo
	{
		ComponentReference ComponentReference = 0;
		GTSL::Id64 MaterialName;
		class RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		const class RenderStaticMeshCollection* RenderStaticMeshCollection = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialResourceManager* MaterialResourceManager = nullptr;
	};
	void AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);
	
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

	struct MaterialLoadInfo
	{
		MaterialLoadInfo(RenderSystem* renderSystem, const GTSL::Buffer& buffer, uint32 instance) : RenderSystem(renderSystem), Buffer(buffer), Instance(instance)
		{
			
		}

		RenderSystem* RenderSystem = nullptr;
		GTSL::Buffer Buffer;
		uint32 Instance = 0;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);
	void onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onStaticMeshLoad);

	BindingsSetLayout bindingsSetLayout;

	uint32 index = 0;
	
	GTSL::Vector<Buffer, BE::PersistentAllocatorReference> meshBuffers;
	GTSL::Vector<RenderAllocation, BE::PersistentAllocatorReference> renderAllocations;
	
	GTSL::Vector<GraphicsPipeline, BE::PersistentAllocatorReference> pipelines;
	GTSL::Vector<GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES>, BE::PersistentAllocatorReference> bindingsSets;
	GTSL::Vector<BindingsPool, BE::PersistentAllocatorReference> bindingsPools;
};
