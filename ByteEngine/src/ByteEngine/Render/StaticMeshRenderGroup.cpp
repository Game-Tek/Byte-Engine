#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup() : meshBuffers(64, GetPersistentAllocator()),
indicesOffsets(64, GetPersistentAllocator()), renderAllocations(64, GetPersistentAllocator()),
indicesCount(64, GetPersistentAllocator()), indexTypes(64, GetPersistentAllocator())
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	positions.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* render_system = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	for (auto& e : meshBuffers) { e.Destroy(render_system->GetRenderDevice()); }
	for (auto& e : renderAllocations) { render_system->DeallocateLocalBufferMemory(e); }
}

void StaticMeshRenderGroup::Render(GameInstance* gameInstance, const RenderSystem* renderSystem)
{	
	for(uint32 i = 0; i < meshBuffers.GetLength(); ++i)
	{		
		CommandBuffer::BindVertexBufferInfo bind_vertex_info;
		bind_vertex_info.RenderDevice = renderSystem->GetRenderDevice();
		bind_vertex_info.Buffer = &meshBuffers[i];
		bind_vertex_info.Offset = 0;
		renderSystem->GetCurrentCommandBuffer()->BindVertexBuffer(bind_vertex_info);
		
		CommandBuffer::BindIndexBufferInfo bind_index_buffer;
		bind_index_buffer.RenderDevice = renderSystem->GetRenderDevice();
		bind_index_buffer.Buffer = &meshBuffers[i];
		bind_index_buffer.Offset = indicesOffsets[i];
		bind_index_buffer.IndexType = indexTypes[i];
		renderSystem->GetCurrentCommandBuffer()->BindIndexBuffer(bind_index_buffer);
		
		CommandBuffer::DrawIndexedInfo draw_indexed_info;
		draw_indexed_info.RenderDevice = renderSystem->GetRenderDevice();
		draw_indexed_info.InstanceCount = 1;
		draw_indexed_info.IndexCount = indicesCount[i];
		renderSystem->GetCurrentCommandBuffer()->DrawIndexed(draw_indexed_info);
	}
}

ComponentReference StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 bufferSize = 0, indicesOffset = 0; uint16 indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &indexSize, &indexSize, &bufferSize, &indicesOffset);

	Buffer::CreateInfo bufferCreateInfo;
	bufferCreateInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Device buffer. StaticMeshRenderGroup: "); name += addStaticMeshInfo.MeshName;
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
	
	void* mesh_load_info;
	GTSL::New<MeshLoadInfo>(&mesh_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, {"StaticMeshRenderGroup", AccessType::READ_WRITE} };
	
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
	indicesOffsets.Insert(index, indicesOffset);
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
	
	meshBuffers.Insert(loadInfo->InstanceId, deviceBuffer);
	renderAllocations.Insert(loadInfo->InstanceId, loadInfo->Allocation);
	indicesCount.Insert(loadInfo->InstanceId, onStaticMeshLoad.IndexCount);
	indexTypes.Insert(loadInfo->InstanceId, SelectIndexType(onStaticMeshLoad.IndexSize));

	GTSL::Delete<MeshLoadInfo>(loadInfo, GetPersistentAllocator());
}