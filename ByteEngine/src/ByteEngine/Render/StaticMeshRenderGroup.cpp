#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup() : meshBuffers(64, GetPersistentAllocator()),
indeces(64, GetPersistentAllocator()), renderAllocations(64, GetPersistentAllocator())
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	BindingsSetLayout::CreateInfo bindings_set_layout_create_info;
	bindings_set_layout_create_info.RenderDevice = static_cast<RenderSystem*>(initializeInfo.GameInstance->GetSystem("RenderSystem"))->GetRenderDevice();
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> binding_descriptors;
	binding_descriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::UNIFORM_BUFFER_DYNAMIC, ShaderStage::VERTEX, 1 });
	bindings_set_layout_create_info.BindingsDescriptors = binding_descriptors;
	::new(&bindingsSetLayout) BindingsSetLayout(bindings_set_layout_create_info);
	
	BindingsPool::CreateInfo create_info;
	create_info.RenderDevice = static_cast<RenderSystem*>(initializeInfo.GameInstance->GetSystem("RenderSystem"))->GetRenderDevice();
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptor_pool_sizes;
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{BindingType::UNIFORM_BUFFER_DYNAMIC, 3});
	create_info.DescriptorPoolSizes = descriptor_pool_sizes;
	create_info.MaxSets = 1;
	::new(&bindingsPool) BindingsPool(create_info);

	BindingsPool::AllocateBindingsSetsInfo allocate_bindings_sets_info;
	allocate_bindings_sets_info.RenderDevice = static_cast<RenderSystem*>(initializeInfo.GameInstance->GetSystem("RenderSystem"))->GetRenderDevice();
	allocate_bindings_sets_info.BindingsSets = GTSL::Ranger<BindingsSet>(bindingsSets.GetCapacity(), bindingsSets.begin());
	allocate_bindings_sets_info.BindingsSetLayouts = GTSL::Array<BindingsSetLayout, MAX_CONCURRENT_FRAMES>{ bindingsSetLayout, bindingsSetLayout, bindingsSetLayout };
	bindingsPool.AllocateBindingsSets(allocate_bindings_sets_info);
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* render_system = (RenderSystem*)shutdownInfo.GameInstance->GetSystem("RenderSystem");
	
	for (auto& e : meshBuffers) { e.Destroy(static_cast<RenderSystem*>(shutdownInfo.GameInstance->GetSystem("RenderSystem"))->GetRenderDevice()); }
	for (auto& e : renderAllocations) { render_system->DeallocateLocalBufferMemory(e.Size, e.Offset, e.AllocationId); }
}

void StaticMeshRenderGroup::Render(RenderSystem* renderSystem, GTSL::Matrix4 viewProjectionMatrix)
{	
	for(auto& e : meshBuffers)
	{
		CommandBuffer::DrawIndexedInfo draw_indexed_info;
		draw_indexed_info.RenderDevice = renderSystem->GetRenderDevice();
		draw_indexed_info.IndexCount;
		draw_indexed_info.InstanceCount = 1;
		renderSystem->GetCurrentCommandBuffer()->DrawIndexed();
	}
}

void StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 buffer_size = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference], 256, buffer_size);

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	buffer_create_info.Size = buffer_size;
	buffer_create_info.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::BufferMemoryRequirements buffer_memory_requirements;
	addStaticMeshInfo.RenderSystem->GetRenderDevice()->GetBufferMemoryRequirements(&scratch_buffer, buffer_memory_requirements);
	
	uint32 offset; void* data; DeviceMemory device_memory; AllocationId alloc_id;

	const uint32 size = buffer_memory_requirements.Size;
	const uint32 alignment = buffer_memory_requirements.Alignment;
	
	RenderSystem::BufferScratchMemoryAllocationInfo memory_allocation_info;
	memory_allocation_info.Size = size;
	memory_allocation_info.Offset = &offset;
	memory_allocation_info.Data = &data;
	memory_allocation_info.AllocationId = &alloc_id;
	memory_allocation_info.DeviceMemory = &device_memory;
	addStaticMeshInfo.RenderSystem->AllocateScratchBufferMemory(memory_allocation_info);

	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	bind_memory_info.Memory = &device_memory;
	bind_memory_info.Offset = offset;
	scratch_buffer.BindToMemory(bind_memory_info);
	
	void* mesh_load_info;
	GTSL::New<MeshLoadInfo>(&mesh_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, scratch_buffer, RenderAllocation{ size, offset, alloc_id }, index);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, {"StaticMeshRenderGroup", AccessType::READ_WRITE} };;
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.RenderStaticMeshCollection->ResourceNames[addStaticMeshInfo.ComponentReference];
	load_static_meshInfo.IndicesAlignment = alignment;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	uint32 material_size;
	addStaticMeshInfo.MaterialResourceManager->GetMaterialSize(addStaticMeshInfo.MaterialName, material_size);

	GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
	void* mat_load_info;
	GTSL::New<MaterialLoadInfo>(&mat_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, material_buffer, index);
	
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = addStaticMeshInfo.GameInstance;
	material_load_info.DoneFor = "FrameEnd";
	material_load_info.StartOn = "FrameStart";
	material_load_info.Name = addStaticMeshInfo.MaterialName;
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, mat_load_info);
	material_load_info.DataBuffer = material_buffer;
	addStaticMeshInfo.MaterialResourceManager->LoadMaterial(material_load_info);

	++index;
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	MeshLoadInfo* load_info = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

	uint32 offset = 0; DeviceMemory device_memory; AllocationId alloc_id;
	
	RenderSystem::BufferLocalMemoryAllocationInfo memory_allocation_info;
	memory_allocation_info.Size = onStaticMeshLoad.DataBuffer.Bytes();
	memory_allocation_info.Offset = &offset;
	memory_allocation_info.DeviceMemory = &device_memory;
	memory_allocation_info.AllocationId = &alloc_id;
	load_info->RenderSystem->AllocateLocalBufferMemory(memory_allocation_info);

	Buffer::CreateInfo create_info;
	create_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
	create_info.Size = onStaticMeshLoad.DataBuffer.Bytes();
	create_info.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_DESTINATION;
	Buffer device_buffer(create_info);
	
	RenderSystem::BufferCopyData buffer_copy_data;
	buffer_copy_data.SourceOffset = 0;
	buffer_copy_data.DestinationOffset = 0;
	buffer_copy_data.SourceBuffer = load_info->ScratchBuffer;
	buffer_copy_data.DestinationBuffer = device_buffer;
	buffer_copy_data.Size = onStaticMeshLoad.DataBuffer.Bytes();
	buffer_copy_data.Allocation = load_info->Allocation;
	load_info->RenderSystem->AddBufferCopy(buffer_copy_data);
	
	meshBuffers.Emplace(load_info->InstanceId, device_buffer);
	renderAllocations.Emplace(load_info->InstanceId, load_info->Allocation);
	indeces.Emplace(load_info->InstanceId, onStaticMeshLoad.IndexCount);

	GTSL::Delete<MeshLoadInfo>(load_info, GetPersistentAllocator());
}

void StaticMeshRenderGroup::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onStaticMeshLoad)
{
	auto load_info = DYNAMIC_CAST(MaterialLoadInfo, onStaticMeshLoad.UserData);

	GTSL::Array<ShaderDataType, 10> shader_datas(onStaticMeshLoad.VertexElements.GetLength());
	ConvertShaderDataType(onStaticMeshLoad.VertexElements, shader_datas);
	GraphicsPipeline::CreateInfo pipeline_create_info;
	pipeline_create_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
	pipeline_create_info.VertexDescriptor = shader_datas;
	pipeline_create_info.IsInheritable = true;

	GTSL::Array<BindingsSetLayout, 10> bindings_set_layouts;
	bindings_set_layouts.EmplaceBack(bindingsSetLayout);

	pipeline_create_info.BindingsSetLayouts = bindings_set_layouts;
	pipeline_create_info.PipelineDescriptor.CullMode = CullMode::CULL_BACK;
	
	GTSL::Array<Shader, 10> shaders; uint32 offset = 0;
	for(uint32 i = 0; i < onStaticMeshLoad.ShaderTypes.GetLength(); ++i)
	{
		Shader::CreateInfo create_info;
		create_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
		create_info.ShaderData = GTSL::Ranger<const byte>(onStaticMeshLoad.ShaderSizes[i], onStaticMeshLoad.DataBuffer + offset);
		shaders.EmplaceBack(create_info);
		offset += onStaticMeshLoad.ShaderSizes[i];
	}
	
	GTSL::Array<Pipeline::ShaderInfo, 10> shader_infos;
	for(uint32 i = 0; i < shaders.GetLength(); ++i)
	{
		shader_infos.PushBack({ static_cast<ShaderType>(onStaticMeshLoad.ShaderTypes[i]), &shaders[i] });
	}

	pipeline_create_info.Stages = shader_infos;
	pipeline_create_info.RenderPass = load_info->RenderSystem->GetRenderPass();
	pipelines.Emplace(load_info->Instance, pipeline_create_info);
	
	load_info->Buffer.Free(32, GetPersistentAllocator());
	GTSL::Delete<MaterialLoadInfo>(load_info, GetPersistentAllocator());
}
