#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup()
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	positions.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());

	addedMeshes.First = 0; addedMeshes.Second = 0;
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

ComponentReference StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Buffer. StaticMesh: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name;
	}
	
	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer;
	
	HostRenderAllocation allocation;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.CreateInfo = &bufferCreateInfo;
	memoryAllocationInfo.Allocation = &allocation;
	memoryAllocationInfo.Buffer = &scratch_buffer;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

	uint32 index = positions.GetFirstFreeIndex().Get();
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = Task<StaticMeshResourceManager::OnStaticMeshLoad>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	++addedMeshes.Second;
	
	return ComponentReference(GetSystemId(), index);
}

ComponentReference StaticMeshRenderGroup::AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Buffer. StaticMesh: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name;
	}

	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer;

	HostRenderAllocation allocation;

	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.CreateInfo = &bufferCreateInfo;
	memoryAllocationInfo.Allocation = &allocation;
	memoryAllocationInfo.Buffer = &scratch_buffer;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

	uint32 index = positions.GetFirstFreeIndex().Get();

	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onRayTracedStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, static_cast<byte*>(allocation.Data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceAt(index);

	return ComponentReference(GetSystemId(), index);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		auto meshRef = loadInfo->RenderSystem->CreateMesh(onStaticMeshLoad.ResourceName, loadInfo->ScratchBuffer, onStaticMeshLoad.VertexSize * onStaticMeshLoad.VertexCount, onStaticMeshLoad.IndexCount, onStaticMeshLoad.IndexSize);
		loadInfo->RenderSystem->AddMeshToId(meshRef, loadInfo->Material.MaterialType);

		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};

	taskInfo.GameInstance->AddFreeDynamicTask(Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}

void StaticMeshRenderGroup::onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		RenderSystem::CreateRayTracingMeshInfo meshInfo;
		meshInfo.SourceBuffer = loadInfo->ScratchBuffer;
		meshInfo.SourceAllocation = loadInfo->Allocation;
		meshInfo.Vertices = onStaticMeshLoad.VertexCount;
		meshInfo.IndexCount = onStaticMeshLoad.IndexCount;
		meshInfo.IndicesOffset = onStaticMeshLoad.IndicesOffset;
		meshInfo.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);

		GTSL::Matrix3x4 matrix;
		meshInfo.Matrix = &matrix;
		loadInfo->RenderSystem->CreateRayTracedMesh(meshInfo);
		
		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};

	taskInfo.GameInstance->AddFreeDynamicTask(Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}
