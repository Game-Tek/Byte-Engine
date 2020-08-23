#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup() : meshBuffers(64, GetPersistentAllocator()),
indicesOffset(64, GetPersistentAllocator()), renderAllocations(64, GetPersistentAllocator()),
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
		bind_index_buffer.Offset = indicesOffset[i];
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
	uint32 buffer_size = 0, indices_offset = 0; uint16 index_size = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &index_size, &index_size, &buffer_size, &indices_offset);

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	buffer_create_info.Size = buffer_size;
	buffer_create_info.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::MemoryRequirements memory_requirements;
	addStaticMeshInfo.RenderSystem->GetRenderDevice()->GetBufferMemoryRequirements(&scratch_buffer, memory_requirements);
	
	void* data; DeviceMemory device_memory; RenderAllocation allocation;

	const uint32 size = memory_requirements.Size;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
	memoryAllocationInfo.Buffer = scratch_buffer;
	memoryAllocationInfo.Data = &data;
	memoryAllocationInfo.Allocation = &allocation;
	memoryAllocationInfo.DeviceMemory = &device_memory;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

	Buffer::BindMemoryInfo bindMemoryInfo;
	bindMemoryInfo.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	bindMemoryInfo.Memory = &device_memory;
	bindMemoryInfo.Offset = allocation.Offset;
	scratch_buffer.BindToMemory(bindMemoryInfo);
	
	void* mesh_load_info;
	GTSL::New<MeshLoadInfo>(&mesh_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, allocation, index);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, {"StaticMeshRenderGroup", AccessType::READ_WRITE} };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = index_size;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	load_static_meshInfo.StartOn = "FrameStart";
	load_static_meshInfo.DoneFor = "FrameEnd";
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName);
	indicesOffset.Insert(index, indices_offset);
	positions.EmplaceBack();
	
	return index++;
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

	RenderAllocation allocation; DeviceMemory device_memory;

	Buffer::CreateInfo create_info;
	create_info.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	create_info.Size = onStaticMeshLoad.DataBuffer.Bytes();
	create_info.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION;
	Buffer device_buffer(create_info);

	RenderDevice::MemoryRequirements memory_requirements;
	loadInfo->RenderSystem->GetRenderDevice()->GetBufferMemoryRequirements(&device_buffer, memory_requirements);

	{
		RenderSystem::BufferLocalMemoryAllocationInfo memoryAllocationInfo;
		memoryAllocationInfo.DeviceMemory = &device_memory;
		memoryAllocationInfo.Allocation = &allocation;
		loadInfo->RenderSystem->AllocateLocalBufferMemory(memoryAllocationInfo);
	}
	
	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	bind_memory_info.Memory = &device_memory;
	bind_memory_info.Offset = allocation.Offset;
	device_buffer.BindToMemory(bind_memory_info);
	
	RenderSystem::BufferCopyData buffer_copy_data;
	buffer_copy_data.SourceOffset = 0;
	buffer_copy_data.DestinationOffset = 0;
	buffer_copy_data.SourceBuffer = loadInfo->ScratchBuffer;
	buffer_copy_data.DestinationBuffer = device_buffer;
	buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
	buffer_copy_data.Allocation = loadInfo->Allocation;
	loadInfo->RenderSystem->AddBufferCopy(buffer_copy_data);
	
	meshBuffers.Insert(loadInfo->InstanceId, device_buffer);
	renderAllocations.Insert(loadInfo->InstanceId, loadInfo->Allocation);
	indicesCount.Insert(loadInfo->InstanceId, onStaticMeshLoad.IndexCount);
	indexTypes.Insert(loadInfo->InstanceId, SelectIndexType(onStaticMeshLoad.IndexSize));

	GTSL::Delete<MeshLoadInfo>(loadInfo, GetPersistentAllocator());
}