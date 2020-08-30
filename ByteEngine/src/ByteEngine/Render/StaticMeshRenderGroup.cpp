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
	meshes.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator()),
	renderAllocations.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* render_system = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	for (auto& e : meshes)
	{
		e.Buffer.Destroy(render_system->GetRenderDevice());
	}
	for (auto& e : renderAllocations) { render_system->DeallocateLocalBufferMemory(e); }
}

ComponentReference StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Device buffer. StaticMeshRenderGroup: "); name += addStaticMeshInfo.MeshName.GetHash();
		bufferCreateInfo.Name = name.begin();
	}
	
	bufferCreateInfo.Size = bufferSize;
	bufferCreateInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(bufferCreateInfo);
	
	void* data; RenderAllocation allocation;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.Buffer = scratch_buffer;
	memoryAllocationInfo.Data = &data;
	memoryAllocationInfo.Allocation = &allocation;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(bufferSize, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	positions.EmplaceBack();
	
	return index++;
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

	RenderAllocation allocation;

	Buffer::CreateInfo createInfo;
	createInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Device buffer. StaticMeshRenderGroup: "); name += onStaticMeshLoad.ResourceName;
		createInfo.Name = name.begin();
	}
	
	createInfo.Size = onStaticMeshLoad.DataBuffer.Bytes();
	createInfo.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION;
	Buffer deviceBuffer(createInfo);

	{
		RenderSystem::BufferLocalMemoryAllocationInfo memoryAllocationInfo;
		memoryAllocationInfo.Allocation = &allocation;
		memoryAllocationInfo.Buffer = deviceBuffer;
		loadInfo->RenderSystem->AllocateLocalBufferMemory(memoryAllocationInfo);
	}
	
	RenderSystem::BufferCopyData buffer_copy_data;
	buffer_copy_data.SourceOffset = 0;
	buffer_copy_data.DestinationOffset = 0;
	buffer_copy_data.SourceBuffer = loadInfo->ScratchBuffer;
	buffer_copy_data.DestinationBuffer = deviceBuffer;
	buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
	buffer_copy_data.Allocation = loadInfo->Allocation;
	loadInfo->RenderSystem->AddBufferCopy(buffer_copy_data);

	{
		Mesh mesh;
		mesh.IndexType = SelectIndexType(onStaticMeshLoad.IndexSize);
		mesh.IndicesCount = onStaticMeshLoad.IndexCount;
		mesh.IndicesOffset = onStaticMeshLoad.IndicesOffset;
		mesh.Buffer = deviceBuffer;
		
		meshes.Insert(loadInfo->InstanceId, mesh);
	}
	
	renderAllocations.Insert(loadInfo->InstanceId, loadInfo->Allocation);

	GTSL::Delete(loadInfo, GetPersistentAllocator());
}