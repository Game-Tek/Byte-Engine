#include "MaterialSystem.h"

#include "RenderSystem.h"

void MaterialSystem::Initialize(const InitializeInfo& initializeInfo)
{
	pipelines.Initialize(32, GetPersistentAllocator());
	
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	BindingsSetLayout::CreateInfo bindings_set_layout_create_info;
	bindings_set_layout_create_info.RenderDevice = render_device->GetRenderDevice();
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> binding_descriptors;
	binding_descriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::UNIFORM_BUFFER_DYNAMIC, ShaderStage::VERTEX, 1 });
	bindings_set_layout_create_info.BindingsDescriptors = binding_descriptors;
	::new(&bindingsSetLayout) BindingsSetLayout(bindings_set_layout_create_info);

	BindingsPool::CreateInfo create_info;
	create_info.RenderDevice = render_device->GetRenderDevice();
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptor_pool_sizes;
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER_DYNAMIC, 3 });
	create_info.DescriptorPoolSizes = descriptor_pool_sizes;
	create_info.MaxSets = MAX_CONCURRENT_FRAMES;
	::new(&bindingsPool) BindingsPool(create_info);

	BindingsPool::AllocateBindingsSetsInfo allocate_bindings_sets_info;
	allocate_bindings_sets_info.RenderDevice = render_device->GetRenderDevice();
	allocate_bindings_sets_info.BindingsSets = GTSL::Ranger<BindingsSet>(bindingsSets.GetCapacity(), bindingsSets.begin());
	allocate_bindings_sets_info.BindingsSetLayouts = GTSL::Array<BindingsSetLayout, MAX_CONCURRENT_FRAMES>{ bindingsSetLayout, bindingsSetLayout, bindingsSetLayout };
	bindingsPool.AllocateBindingsSets(allocate_bindings_sets_info);

	bindingsSets.Resize(3);
	
	for (auto& e : bindingsSets)
	{
		BindingsSet::BindingsSetUpdateInfo bindings_set_update_info;
		bindings_set_update_info.RenderDevice = render_device->GetRenderDevice();

		BindingsSetLayout::BufferBindingDescriptor binding_descriptor;
		binding_descriptor.UniformCount = 1;
		binding_descriptor.BindingType = GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC;
		binding_descriptor.Buffers = GTSL::Ranger<Buffer>(1, &uniformBuffer);
		binding_descriptor.Sizes = GTSL::Array<uint32, 1>{ sizeof(GTSL::Matrix4) };
		binding_descriptor.Offsets = GTSL::Array<uint32, 1>{ 0 };

		bindings_set_update_info.BufferBindingsSetLayout.EmplaceBack(binding_descriptor);
		e.Update(bindings_set_update_info);
	}

	//CommandBuffer::BindBindingsSetInfo bind_bindings_set_info;
	//bind_bindings_set_info.RenderDevice = renderSystem->GetRenderDevice();
	//bind_bindings_set_info.BindingsSets = GTSL::Ranger<BindingsSet>(1, &bindingsSets[renderSystem->GetCurrentFrame()]);
	//bind_bindings_set_info.Pipeline = &pipelines[i];
	//bind_bindings_set_info.Offsets = GTSL::Array<uint32, 1>{ offset };
	//bind_bindings_set_info.PipelineType = PipelineType::GRAPHICS;
	//renderSystem->GetCurrentCommandBuffer()->BindBindingsSet(bind_bindings_set_info);
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	for (auto& e : pipelines) { e.Destroy(render_system->GetRenderDevice()); }
	bindingsSetLayout.Destroy(render_system->GetRenderDevice());
	bindingsPool.Destroy(render_system->GetRenderDevice());
}

ComponentReference MaterialSystem::CreateMaterial(const Id name)
{
	//uint32 material_size = 0;
	//addStaticMeshInfo.MaterialResourceManager->GetMaterialSize(addStaticMeshInfo.MaterialName, material_size);
	//
	//GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	//
	//MaterialResourceManager::MaterialLoadInfo material_load_info;
	//material_load_info.ActsOn = acts_on;
	//material_load_info.GameInstance = addStaticMeshInfo.GameInstance;
	//material_load_info.StartOn = "FrameStart";
	//material_load_info.DoneFor = "FrameEnd";
	//material_load_info.Name = addStaticMeshInfo.MaterialName;
	//material_load_info.DataBuffer = GTSL::Ranger<byte>(material_buffer.GetCapacity(), material_buffer.GetData());
	//void* mat_load_info;
	//GTSL::New<MaterialLoadInfo>(&mat_load_info, GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, GTSL::MoveRef(material_buffer), index);
	//material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, mat_load_info);
	//material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onMaterialLoaded>(this);
	//addStaticMeshInfo.MaterialResourceManager->LoadMaterial(material_load_info);
	
	materialNames.EmplaceBack(materialNames);
	return component++;
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onStaticMeshLoad)
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

	pipeline_create_info.PipelineDescriptor.BlendEnable = false;
	pipeline_create_info.PipelineDescriptor.CullMode = CullMode::CULL_BACK;
	pipeline_create_info.PipelineDescriptor.ColorBlendOperation = GAL::BlendOperation::ADD;

	pipeline_create_info.SurfaceExtent = { 1280, 720 };

	GTSL::Array<Shader, 10> shaders; uint32 offset = 0;
	for (uint32 i = 0; i < onStaticMeshLoad.ShaderTypes.GetLength(); ++i)
	{
		Shader::CreateInfo create_info;
		create_info.RenderDevice = load_info->RenderSystem->GetRenderDevice();
		create_info.ShaderData = GTSL::Ranger<const byte>(onStaticMeshLoad.ShaderSizes[i], onStaticMeshLoad.DataBuffer + offset);
		shaders.EmplaceBack(create_info);
		offset += onStaticMeshLoad.ShaderSizes[i];
	}

	GTSL::Array<Pipeline::ShaderInfo, 10> shader_infos;
	for (uint32 i = 0; i < shaders.GetLength(); ++i)
	{
		shader_infos.PushBack({ ConvertShaderType(onStaticMeshLoad.ShaderTypes[i]), &shaders[i] });
	}

	pipeline_create_info.Stages = shader_infos;
	pipeline_create_info.RenderPass = load_info->RenderSystem->GetRenderPass();
	pipelines.Insert(load_info->Instance, pipeline_create_info);

	load_info->Buffer.Free(32, GetPersistentAllocator());
	GTSL::Delete<MaterialLoadInfo>(load_info, GetPersistentAllocator());
}
