#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup() : meshBuffers()
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	meshBuffers.Initialize(64);
}

void StaticMeshRenderGroup::Shutdown()
{
	//for (auto& e : meshBuffers) { e.Destroy(); }
}

void StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 buffer_size = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference], 256, buffer_size);

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	buffer_create_info.Size = buffer_size;
	buffer_create_info.BufferType = (uint32)BufferType::VERTEX | (uint32)BufferType::INDEX | (uint32)BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	addStaticMeshInfo.RenderSystem->GetRenderDevice()->GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);
	
	uint32 offset; void* data; DeviceMemory device_memory;

	const uint32 size = buffer_memory_requirements.Size;
	const uint32 alignment = buffer_memory_requirements.Alignment;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memory_allocation_info;
	memory_allocation_info.Size = size;
	memory_allocation_info.Offset = &offset;
	memory_allocation_info.Data = &data;
	memory_allocation_info.DeviceMemory = &device_memory;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memory_allocation_info);

	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	bind_memory_info.Memory = &device_memory;
	bind_memory_info.Offset = offset;
	scratch_buffer.BindToMemory(bind_memory_info);

	//BE::Application::Get()->GetGameInstance()->GetThreadPool()->EnqueueTask();
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference];
	load_static_meshInfo.IndicesAlignment = alignment;

	const auto load_info = new LoadInfo((RenderSystem*)addStaticMeshInfo.RenderSystem, scratch_buffer);
	
	load_static_meshInfo.UserData = DYNAMIC_TYPE(LoadInfo, load_info);
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	CommandBuffer::CopyBuffersInfo copy_buffers_info;
	
	LoadInfo* load_info = DYNAMIC_CAST(LoadInfo, onStaticMeshLoad.UserData);
	
	copy_buffers_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
	copy_buffers_info.Size = onStaticMeshLoad.DataBuffer.Bytes();

	uint32 offset = 0; DeviceMemory device_memory;
	
	RenderSystem::BufferLocalMemoryAllocationInfo memory_allocation_info;
	memory_allocation_info.Size = onStaticMeshLoad.DataBuffer.Bytes();
	memory_allocation_info.Offset = &offset;
	memory_allocation_info.DeviceMemory = &device_memory;
	load_info->RenderSystem->AllocateLocalBufferMemory(memory_allocation_info);

	Buffer::CreateInfo create_info;
	create_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
	create_info.Size = onStaticMeshLoad.DataBuffer.Bytes();
	create_info.BufferType = (uint8)BufferType::VERTEX | (uint8)BufferType::INDEX | (uint8)BufferType::TRANSFER_DESTINATION;
	Buffer device_buffer(create_info);
	
	copy_buffers_info.Destination = &device_buffer;
	copy_buffers_info.DestinationOffset = offset;
	copy_buffers_info.Source = &load_info->ScratchBuffer;
	copy_buffers_info.SourceOffset = 0;
	load_info->RenderSystem->GetTransferCommandBuffer()->CopyBuffers(copy_buffers_info);
	
	meshBuffers.EmplaceBack(device_buffer);

	delete load_info;
}
