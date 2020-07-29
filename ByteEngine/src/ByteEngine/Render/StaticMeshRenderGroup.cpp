#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "RenderSystem.h"
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
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference];
	load_static_meshInfo.IndicesAlignment = alignment;

	void* load_info;
	GTSL::New<LoadInfo>(&load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer);
	
	load_static_meshInfo.UserData = DYNAMIC_TYPE(LoadInfo, load_info);
	load_static_meshInfo.ActsOn = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, {"StaticMeshRenderGroup", AccessType::READ_WRITE} };
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	BindingsPool::CreateInfo create_info;

	GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> bindings_sets;
	
	GTSL::Array<GAL::BindingDescriptor, 10> binding_descriptors;
	GAL::BindingDescriptor binding_descriptor;
	binding_descriptor.ShaderStage = GAL::ShaderType::VERTEX_SHADER;
	binding_descriptor.BindingType = GAL::BindingType::FLOAT3;
	binding_descriptor.MaxNumberOfBindingsAllocatable = 3;
	binding_descriptors.EmplaceBack(binding_descriptor);

	//create_info.BindingsSets = bindings_sets;
	
	create_info.BindingsDescriptors = binding_descriptors;
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	LoadInfo* load_info = DYNAMIC_CAST(LoadInfo, onStaticMeshLoad.UserData);

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
	
	RenderSystem::BufferCopyData buffer_copy_data;
	buffer_copy_data.SourceOffset = 0;
	buffer_copy_data.DestinationOffset = 0;
	buffer_copy_data.SourceBuffer = load_info->ScratchBuffer;
	buffer_copy_data.DestinationBuffer = device_buffer;
	buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
	load_info->RenderSystem->AddBufferCopy(buffer_copy_data);
	
	meshBuffers.EmplaceBack(device_buffer);

	GTSL::Delete<LoadInfo>(reinterpret_cast<void**>(&load_info), GetPersistentAllocator());

	GraphicsPipeline::CreateInfo pipeline_create_info;
	pipeline_create_info.VertexDescriptor = GetShaderDataTypes(onStaticMeshLoad.VertexDescriptor);
	pipeline_create_info.IsInheritable = true;
}
