#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	meshBuffers.Initialize(64, GetPersistentAllocator());
}

void StaticMeshRenderGroup::Shutdown()
{
	meshBuffers.Free(GetPersistentAllocator());
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
	
	RenderSystem::ScratchMemoryAllocationInfo memory_allocation_info;
	memory_allocation_info.Size = size;
	memory_allocation_info.MemoryType = buffer_memory_requirements.MemoryTypes;
	memory_allocation_info.Offset = &offset;
	memory_allocation_info.Data = &data;
	memory_allocation_info.DeviceMemory = &device_memory;
	addStaticMeshInfo.RenderSystem->AllocateScratchMemory(memory_allocation_info);

	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	bind_memory_info.Memory = &device_memory;
	bind_memory_info.Offset = offset;
	scratch_buffer.BindToMemory(bind_memory_info);

	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference];
	load_static_meshInfo.IndicesAlignment = alignment;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(Buffer, data);
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	//CommandBuffer::CopyBuffersInfo copy_buffers_info;
	//copy_buffers_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	//copy_buffers_info.Size = size; //SIZE FROM ALLOC
	//copy_buffers_info.Destination;
	//copy_buffers_info.DestinationOffset;
	//copy_buffers_info.Source = &scratch_buffer;
	//copy_buffers_info.SourceOffset = 0;
	//addStaticMeshInfo.RenderSystem->GetTransferCommandBuffer()->CopyBuffers(copy_buffers_info);
	//
	//meshBuffers.EmplaceBack(GetPersistentAllocator(), buffer_create_info);
}
