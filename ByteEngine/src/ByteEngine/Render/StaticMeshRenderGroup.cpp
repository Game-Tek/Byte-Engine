#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "RenderSystem.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup() : meshBuffers(64, GetPersistentAllocator()),
indicesOffset(64, GetPersistentAllocator()), renderAllocations(64, GetPersistentAllocator()), pipelines(64, GetPersistentAllocator()),
indicesCount(64, GetPersistentAllocator())
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	auto render_device = static_cast<RenderSystem*>(initializeInfo.GameInstance->GetSystem("RenderSystem"));
	
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

	DeviceMemory device_memory;
	
	RenderSystem::BufferScratchMemoryAllocationInfo allocation_info;
	allocation_info.Size = sizeof(GTSL::Matrix4) * MAX_CONCURRENT_FRAMES;
	allocation_info.Offset = &offset;
	allocation_info.AllocationId = &uniformAllocation;
	allocation_info.Data = &uniformPointer;
	allocation_info.DeviceMemory = &device_memory;
	render_device->AllocateScratchBufferMemory(allocation_info);

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = render_device->GetRenderDevice();
	buffer_create_info.Size = allocation_info.Size;
	buffer_create_info.BufferType = BufferType::UNIFORM;
	uniformBuffer = Buffer(buffer_create_info);

	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = render_device->GetRenderDevice();
	bind_memory_info.Offset = offset;
	bind_memory_info.Memory = &device_memory;
	uniformBuffer.BindToMemory(bind_memory_info);

	for(auto& e : bindingsSets)
	{
		BindingsSet::BindingsSetUpdateInfo bindings_set_update_info;
		bindings_set_update_info.RenderDevice = render_device->GetRenderDevice();

		BindingsSetLayout::BufferBindingDescriptor binding_descriptor;
		binding_descriptor.UniformCount = 1;
		binding_descriptor.BindingType = GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC;
		binding_descriptor.Buffers = GTSL::Ranger<Buffer>(1, &uniformBuffer);
		binding_descriptor.Sizes = GTSL::Array<uint32, 1>{ sizeof(GTSL::Matrix4) * MAX_CONCURRENT_FRAMES };
		binding_descriptor.Offsets = GTSL::Array<uint32, 1>{ 0 };
		
		bindings_set_update_info.BufferBindingsSetLayout.EmplaceBack(binding_descriptor);
		e.Update(bindings_set_update_info);
	}
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* render_system = static_cast<RenderSystem*>(shutdownInfo.GameInstance->GetSystem("RenderSystem"));
	
	for (auto& e : meshBuffers) { e.Destroy(render_system->GetRenderDevice()); }
	for (auto& e : renderAllocations) { render_system->DeallocateLocalBufferMemory(e.Size, e.Offset, e.AllocationId); }
	for (auto& e : pipelines) { e.Destroy(render_system->GetRenderDevice()); }

	uniformBuffer.Destroy(render_system->GetRenderDevice());
	render_system->DeallocateScratchBufferMemory(sizeof(GTSL::Matrix4) * MAX_CONCURRENT_FRAMES, offset, uniformAllocation);
	BindingsPool::FreeBindingsSetInfo free_bindings_set_info;
	free_bindings_set_info.RenderDevice = render_system->GetRenderDevice();
	free_bindings_set_info.BindingsSet = bindingsSets;
	bindingsPool.FreeBindingsSet(free_bindings_set_info);
	bindingsSetLayout.Destroy(render_system->GetRenderDevice());
	bindingsPool.Destroy(render_system->GetRenderDevice());
}

void StaticMeshRenderGroup::Render(GameInstance* gameInstance, RenderSystem* renderSystem, GTSL::Matrix4 viewProjectionMatrix)
{
	auto positions = static_cast<RenderStaticMeshCollection*>(gameInstance->GetComponentCollection("RenderStaticMeshCollection"))->GetPositions();

	for(uint32 i = 0; i < meshBuffers.GetLength(); ++i)
	{
		*(static_cast<GTSL::Matrix4*>(uniformPointer) + renderSystem->GetCurrentFrame()) = viewProjectionMatrix *= GTSL::Math::Translation(positions[i]);
		
		CommandBuffer::BindPipelineInfo bind_pipeline_info;
		bind_pipeline_info.RenderDevice = renderSystem->GetRenderDevice();
		bind_pipeline_info.Pipeline = &pipelines[i];
		bind_pipeline_info.PipelineType = PipelineType::GRAPHICS;
		renderSystem->GetCurrentCommandBuffer()->BindPipeline(bind_pipeline_info);
		
		CommandBuffer::BindBindingsSetInfo bind_bindings_set_info;
		bind_bindings_set_info.RenderDevice = renderSystem->GetRenderDevice();
		bind_bindings_set_info.BindingsSets = GTSL::Ranger<BindingsSet>(1, &bindingsSets[renderSystem->GetCurrentFrame()]);
		bind_bindings_set_info.Pipeline = &pipelines[i];
		bind_bindings_set_info.Offsets = GTSL::Array<uint32, 1>{ static_cast<uint32>(sizeof(GTSL::Matrix4)) * renderSystem->GetCurrentFrame() };
		renderSystem->GetCurrentCommandBuffer()->BindBindingsSet(bind_bindings_set_info);
		
		CommandBuffer::BindVertexBufferInfo bind_vertex_info;
		bind_vertex_info.RenderDevice = renderSystem->GetRenderDevice();
		bind_vertex_info.Buffer = &meshBuffers[i];
		bind_vertex_info.Offset = 0;
		renderSystem->GetCurrentCommandBuffer()->BindVertexBuffer(bind_vertex_info);
		
		CommandBuffer::BindIndexBufferInfo bind_index_buffer;
		bind_index_buffer.RenderDevice = renderSystem->GetRenderDevice();
		bind_index_buffer.Buffer = &meshBuffers[i];
		bind_index_buffer.Offset = indicesOffset[i];
		renderSystem->GetCurrentCommandBuffer()->BindIndexBuffer(bind_index_buffer);
		
		CommandBuffer::DrawIndexedInfo draw_indexed_info;
		draw_indexed_info.RenderDevice = renderSystem->GetRenderDevice();
		draw_indexed_info.InstanceCount = 1;
		draw_indexed_info.IndexCount = indicesCount[i];
		renderSystem->GetCurrentCommandBuffer()->DrawIndexed(draw_indexed_info);
	}
}

void StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 buffer_size = 0, indices_offset = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.RenderStaticMeshCollection->GetResourceNames()[addStaticMeshInfo.ComponentReference], sizeof(uint32), buffer_size, &indices_offset);

	Buffer::CreateInfo buffer_create_info;
	buffer_create_info.RenderDevice = addStaticMeshInfo.RenderSystem->GetRenderDevice();
	buffer_create_info.Size = buffer_size;
	buffer_create_info.BufferType = BufferType::VERTEX | BufferType::INDEX | BufferType::TRANSFER_SOURCE;
	Buffer scratch_buffer(buffer_create_info);

	RenderDevice::MemoryRequirements memory_requirements;
	addStaticMeshInfo.RenderSystem->GetRenderDevice()->GetBufferMemoryRequirements(&scratch_buffer, memory_requirements);
	
	uint32 offset; void* data; DeviceMemory device_memory; AllocationId alloc_id;

	const uint32 size = memory_requirements.Size;
	const uint32 alignment = memory_requirements.Alignment;
	
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

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, {"StaticMeshRenderGroup", AccessType::READ_WRITE} };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Ranger<byte>(size, static_cast<byte*>(data));
	load_static_meshInfo.Name = addStaticMeshInfo.RenderStaticMeshCollection->GetResourceNames()[addStaticMeshInfo.ComponentReference];
	load_static_meshInfo.IndicesAlignment = sizeof(uint32);
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	load_static_meshInfo.StartOn = "FrameStart";
	load_static_meshInfo.DoneFor = "FrameEnd";
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	uint32 material_size = 0;
	addStaticMeshInfo.MaterialResourceManager->GetMaterialSize(addStaticMeshInfo.MaterialName, material_size);

	GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = addStaticMeshInfo.GameInstance;
	material_load_info.StartOn = "FrameStart";
	material_load_info.DoneFor = "FrameEnd";
	material_load_info.Name = addStaticMeshInfo.MaterialName;
	material_load_info.DataBuffer = GTSL::Ranger<byte>(material_buffer.GetCapacity(), material_buffer.GetData());
	void* mat_load_info;
	GTSL::New<MaterialLoadInfo>(&mat_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, GTSL::MoveRef(material_buffer), index);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, mat_load_info);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onMaterialLoaded>(this);
	addStaticMeshInfo.MaterialResourceManager->LoadMaterial(material_load_info);

	indicesOffset.Emplace(index, indices_offset);
	
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

	Buffer::BindMemoryInfo bind_memory_info;
	bind_memory_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
	bind_memory_info.Memory = &device_memory;
	bind_memory_info.Offset = offset;
	device_buffer.BindToMemory(bind_memory_info);
	
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
	indicesCount.Emplace(load_info->InstanceId, onStaticMeshLoad.IndexCount);

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
		shader_infos.PushBack({ ConvertShaderType(onStaticMeshLoad.ShaderTypes[i]), &shaders[i] });
	}

	pipeline_create_info.Stages = shader_infos;
	pipeline_create_info.RenderPass = load_info->RenderSystem->GetRenderPass();
	pipelines.Emplace(load_info->Instance, pipeline_create_info);
	
	load_info->Buffer.Free(32, GetPersistentAllocator());
	GTSL::Delete<MaterialLoadInfo>(load_info, GetPersistentAllocator());
}
