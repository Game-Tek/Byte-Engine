#include "MaterialSystem.h"

#include "RenderSystem.h"

BindingType;

const char* BindingTypeString(const BindingType binding)
{
	switch (binding)
	{
	case BindingType::UNIFORM_BUFFER_DYNAMIC: return "UNIFORM_BUFFER_DYNAMIC";
	case BindingType::COMBINED_IMAGE_SAMPLER: return "COMBINED_IMAGE_SAMPLER";
	case BindingType::UNIFORM_BUFFER: return "UNIFORM_BUFFER";
	default: return "null";
	}
}

void MaterialSystem::Initialize(const InitializeInfo& initializeInfo)
{
	renderGroups.Initialize(32, GetPersistentAllocator());
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	GTSL::ForEach(renderGroups,	[&](RenderGroupData& renderGroup)
	{
		renderGroup.BindingsPool.Destroy(renderSystem->GetRenderDevice());
		renderGroup.BindingsSetLayout.Destroy(renderSystem->GetRenderDevice());

		GTSL::ForEach(renderGroup.Instances, [&](MaterialInstance& materialInstance)
		{
			materialInstance.Pipeline.Destroy(renderSystem->GetRenderDevice());
			materialInstance.BindingsPool.Destroy(renderSystem->GetRenderDevice());
			materialInstance.BindingsSetLayout.Destroy(renderSystem->GetRenderDevice());
		});
	});
}

void MaterialSystem::SetGlobalState(GameInstance* gameInstance, const GTSL::Array<GTSL::Array<BindingType, 6>, 6>& globalState)
{
	RenderSystem* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");

	BE_ASSERT(globalState[0].GetLength() == 1 && globalState.GetLength() == 1, "Only one binding set is supported");
	
	for(uint32 i = 0; i < globalState.GetLength(); ++i)
	{
		BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
		bindingsSetLayoutCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> binding_descriptors;
		for(uint32 j = 0; j < globalState[i].GetLength(); ++j)
		{
			binding_descriptors.PushBack(BindingsSetLayout::BindingDescriptor{ globalState[i][j], ShaderStage::ALL, 1 });
		}

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Global state");
			bindingsSetLayoutCreateInfo.Name = name.begin();
		}
		
		bindingsSetLayoutCreateInfo.BindingsDescriptors = binding_descriptors;
		globalBindingsSetLayout.EmplaceBack(bindingsSetLayoutCreateInfo);
	}

	BindingsPool::CreateInfo bindingsPoolCreateInfo;
	bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptor_pool_sizes;
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER_DYNAMIC, 6 });
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::COMBINED_IMAGE_SAMPLER, 16 });
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::STORAGE_BUFFER_DYNAMIC, 16 });
	bindingsPoolCreateInfo.DescriptorPoolSizes = descriptor_pool_sizes;
	bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
	::new(&globalBindingsPool) BindingsPool(bindingsPoolCreateInfo);

	{
		BindingsPool::AllocateBindingsSetsInfo allocateBindingsSetsInfo;
		allocateBindingsSetsInfo.RenderDevice = renderSystem->GetRenderDevice();
		allocateBindingsSetsInfo.BindingsSets = GTSL::Ranger<BindingsSet>(2, globalBindingsSets.begin());
		GTSL::Array<BindingsSetLayout, 6 * MAX_CONCURRENT_FRAMES> bindingsSetLayouts;
		for (uint32 i = 0; i < globalState.GetLength(); ++i)
		{
			for (uint32 j = 0; j < 2; ++j)
			{
				bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[i]);
			}
		}
		allocateBindingsSetsInfo.BindingsSetLayouts = bindingsSetLayouts;
		allocateBindingsSetsInfo.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>{ 1 };
		globalBindingsPool.AllocateBindingsSets(allocateBindingsSetsInfo);

		globalBindingsSets.Resize(renderSystem->GetFrameCount());
	}
	
	{
		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();
		
		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline Layout. Material system global state");
			pipelineLayout.Name = name.begin();
		}

		pipelineLayout.BindingsSetLayouts = globalBindingsSetLayout;
		globalPipelineLayout.Initialize(pipelineLayout);
	}

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<1024> string("Set global state with: \n");

		uint32 i = 0, j = 0;
		for(auto& e : globalState)
		{
			string += "Set: "; string += i; string += '\n';
			
			for(auto& b : e)
			{
				string += '	'; string += "Binding: "; string += j; string += " of type "; string += BindingTypeString(b); string += '\n';
				++j;
			}
			
			++i;
		}
		
		BE_LOG_WARNING(string);
	}
}

void MaterialSystem::AddRenderGroup(GameInstance* gameInstance, const GTSL::Id64 renderGroupName, const GTSL::Array<GTSL::Array<BindingType, 6>, 6>& bindings)
{
	RenderGroupData& renderGroupData = renderGroups.Emplace(renderGroupName);

	RenderSystem* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");

	BE_ASSERT(bindings.GetLength() == 1, "Only one binding set is supported");

	for (uint32 i = 0; i < bindings.GetLength(); ++i)
	{
		BindingsSetLayout::CreateInfo setLayout;
		setLayout.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Render group "); name += renderGroupName;
			setLayout.Name = name.begin();
		}
		
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
		for (uint32 j = 0; j < bindings[i].GetLength(); ++j)
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindings[i][j], ShaderStage::ALL, 1 });
		}

		setLayout.BindingsDescriptors = bindingDescriptors;
		setLayout.SpecialBindings = GTSL::Ranger<const uint32>();
		
		renderGroupData.BindingsSetLayout = BindingsSetLayout(setLayout);
	}
	//Bindings set layout

	{
		BindingsPool::CreateInfo bindingsPoolCreateInfo;
		bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptor_pool_sizes;
		descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER_DYNAMIC, 6 });
		descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER, 6 });
		descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::COMBINED_IMAGE_SAMPLER, 16 });
		descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::STORAGE_BUFFER_DYNAMIC, 16 });
		bindingsPoolCreateInfo.DescriptorPoolSizes = descriptor_pool_sizes;
		bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
		::new(&renderGroupData.BindingsPool) BindingsPool(bindingsPoolCreateInfo);
	}
	//Bindings pool

	{
		BindingsPool::AllocateBindingsSetsInfo allocateBindings;
		allocateBindings.RenderDevice = renderSystem->GetRenderDevice();
		allocateBindings.BindingsSets = GTSL::Ranger<BindingsSet>(renderSystem->GetFrameCount(), renderGroupData.BindingsSets.begin());
		{
			GTSL::Array<BindingsSetLayout, 6 * MAX_CONCURRENT_FRAMES> bindingsSetLayouts;
			for (uint32 i = 0; i < bindings.GetLength(); ++i)
			{
				for (uint32 j = 0; j < renderSystem->GetFrameCount(); ++j)
				{
					bindingsSetLayouts.EmplaceBack(renderGroupData.BindingsSetLayout);
				}
			}

			allocateBindings.BindingsSetLayouts = bindingsSetLayouts;
			allocateBindings.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>{ 1 };
			renderGroupData.BindingsPool.AllocateBindingsSets(allocateBindings);

			renderGroupData.BindingsSets.Resize(renderSystem->GetFrameCount());
		}
	}
		
	{
		GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
		bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[0]); //global bindings
		bindingsSetLayouts.EmplaceBack(renderGroupData.BindingsSetLayout); //render group bindings

		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Render group: "); name += renderGroupName;
			pipelineLayout.Name = name.begin();
		}
		
		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		renderGroupData.PipelineLayout.Initialize(pipelineLayout);
	}
	
	renderGroupData.Instances.Initialize(32, GetPersistentAllocator());
	renderGroupData.RenderGroupName = renderGroupName;

	for (uint32 i = 0; i < bindings.GetLength(); ++i)
	{
		BindingsSet::BindingsSetUpdateInfo bindingsSetUpdateInfo;
		bindingsSetUpdateInfo.RenderDevice = renderSystem->GetRenderDevice();
		
		for (uint32 j = 0; j < bindings[i].GetLength(); ++j)
		{
			if (bindings[i][j] == GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC)
			{
				Buffer::CreateInfo bufferInfo;
				bufferInfo.RenderDevice = renderSystem->GetRenderDevice();
				bufferInfo.Size = 1024;
				bufferInfo.BufferType = BufferType::UNIFORM;
				renderGroupData.Buffer = Buffer(bufferInfo);

				DeviceMemory memory;
				
				RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
				memoryAllocationInfo.Buffer = renderGroupData.Buffer;
				memoryAllocationInfo.Allocation = &renderGroupData.Allocation;
				memoryAllocationInfo.Data = &renderGroupData.Data;
				memoryAllocationInfo.DeviceMemory = &memory;
				renderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

				Buffer::BindMemoryInfo bindMemory;
				bindMemory.RenderDevice = renderSystem->GetRenderDevice();
				bindMemory.Memory = &memory;
				bindMemory.Offset = renderGroupData.Allocation.Offset;
				renderGroupData.Buffer.BindToMemory(bindMemory);
				
				BindingsSetLayout::BufferBindingDescriptor binding_descriptor;
				binding_descriptor.UniformCount = 1;
				binding_descriptor.BindingType = bindings[i][j];
				binding_descriptor.Buffers = GTSL::Ranger<Buffer>(1, &renderGroupData.Buffer);
				binding_descriptor.Sizes = GTSL::Array<uint32, 1>{ sizeof(GTSL::Matrix4) };
				binding_descriptor.Offsets = GTSL::Array<uint32, 1>{ 0 };
				bindingsSetUpdateInfo.BufferBindingsSetLayout.EmplaceBack(binding_descriptor);
			}
			else
			{
				__debugbreak();
			}
		}
		
		renderGroupData.BindingsSets[i].Update(bindingsSetUpdateInfo);
	}
	
	if constexpr (_DEBUG)
	{
		GTSL::StaticString<1024> string("Set render group "); string += renderGroupName; string += " state with \n";

		uint32 i = 0, j = 0;
		for (auto& e : bindings)
		{
			string += "Set: "; string += i; string += '\n';

			for (auto& b : e)
			{
				string += '	'; string += "Binding: "; string += j; string += " of type "; string += BindingTypeString(b); string += '\n';
				++j;
			}

			++i;
		}

		BE_LOG_WARNING(string);
	}
}

ComponentReference MaterialSystem::CreateMaterial(const CreateMaterialInfo& info)
{
	uint32 material_size = 0;
	info.MaterialResourceManager->GetMaterialSize(info.MaterialName, material_size);

	GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());

	const auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = info.GameInstance;
	material_load_info.StartOn = "FrameStart";
	material_load_info.DoneFor = "FrameEnd";
	material_load_info.Name = info.MaterialName;
	material_load_info.DataBuffer = GTSL::Ranger<byte>(material_buffer.GetCapacity(), material_buffer.GetData());
	void* mat_load_info;
	GTSL::New<MaterialLoadInfo>(&mat_load_info, GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer));
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, mat_load_info);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);

	materialNames.EmplaceBack(info.MaterialName);
	return component++;
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{
	auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

	auto& renderGroup = renderGroups.At(onMaterialLoadInfo.RenderGroup);
	auto& instance = renderGroup.Instances.Emplace(onMaterialLoadInfo.ResourceName);

	BindingsPool::CreateInfo bindingsPoolCreateInfo;
	bindingsPoolCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptorPoolSizes;
	
	BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
	bindingsSetLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
	for(auto& e : onMaterialLoadInfo.BindingSets[0])
	{
		auto bindingType = GAL::BindingTypeToVulkanBindingType(e);
		bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindingType, ShaderStage::ALL, 1 });
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ bindingType, 3 }); //TODO: ASK FOR CORRECT NUMBER OF DESCRIPTORS
	}
	bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDescriptors;
	bindingsSetLayoutCreateInfo.SpecialBindings = GTSL::Ranger<const uint32>();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<128> name("Material "); name += onMaterialLoadInfo.ResourceName;
		bindingsSetLayoutCreateInfo.Name = name.begin();
	}
	
	instance.BindingsSetLayout = BindingsSetLayout(bindingsSetLayoutCreateInfo);

	bindingsPoolCreateInfo.DescriptorPoolSizes = descriptorPoolSizes;
	bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
	instance.BindingsPool = BindingsPool(bindingsPoolCreateInfo);

	BindingsPool::AllocateBindingsSetsInfo allocateBindingsSetsInfo;
	allocateBindingsSetsInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	allocateBindingsSetsInfo.BindingsSets = GTSL::Ranger<BindingsSet>(loadInfo->RenderSystem->GetFrameCount(), instance.BindingsSets.begin());
	allocateBindingsSetsInfo.BindingsSetLayouts = GTSL::Array<BindingsSetLayout, MAX_CONCURRENT_FRAMES>{ instance.BindingsSetLayout, instance.BindingsSetLayout, instance.BindingsSetLayout };
	allocateBindingsSetsInfo.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>();
	instance.BindingsPool.AllocateBindingsSets(allocateBindingsSetsInfo);
	instance.BindingsSets.Resize(loadInfo->RenderSystem->GetFrameCount());

	//for (uint32 i = 0; i < onMaterialLoadInfo.BindingSets.GetLength(); ++i)
	//{
	//	BindingsSet::BindingsSetUpdateInfo bindings_set_update_info;
	//	bindings_set_update_info.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	//	
	//	for (uint32 j = 0; j < onMaterialLoadInfo.BindingSets[i].GetLength(); ++j)
	//	{
	//		if (bindingDescriptors[j].BindingType == GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC)
	//		{
	//			//TODO: ALLOCATE BUFFER
	//			
	//			BindingsSetLayout::BufferBindingDescriptor binding_descriptor;
	//			binding_descriptor.UniformCount = 1;
	//			binding_descriptor.BindingType = bindingDescriptors[j].BindingType;
	//			binding_descriptor.Buffers = GTSL::Ranger<Buffer>(1, &uniformBuffer);
	//			binding_descriptor.Sizes = GTSL::Array<uint32, 1>{ sizeof(GTSL::Matrix4) };
	//			binding_descriptor.Offsets = GTSL::Array<uint32, 1>{ 0 };
	//			bindings_set_update_info.BufferBindingsSetLayout.EmplaceBack(binding_descriptor);
	//		}
	//		else
	//		{
	//			__debugbreak();
	//		}
	//	}
	//	
	//	instance.BindingsSets[i].Update(bindings_set_update_info);
	//}
	
	GTSL::Array<ShaderDataType, 10> shader_datas(onMaterialLoadInfo.VertexElements.GetLength());
	ConvertShaderDataType(onMaterialLoadInfo.VertexElements, shader_datas);
	RasterizationPipeline::CreateInfo pipelineCreateInfo;
	pipelineCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	pipelineCreateInfo.VertexDescriptor = shader_datas;
	pipelineCreateInfo.IsInheritable = true;

	GTSL::Array<BindingsSetLayout, 10> bindings_set_layouts;
	bindings_set_layouts.EmplaceBack(instance.BindingsSetLayout);
	
	{
		GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
		bindingsSetLayouts.PushBack(GTSL::Ranger<BindingsSetLayout>(globalBindingsSetLayout)); //global bindings
		bindingsSetLayouts.EmplaceBack(renderGroup.BindingsSetLayout); //render group bindings
		bindingsSetLayouts.EmplaceBack(instance.BindingsSetLayout); //instance group bindings

		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
		
		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline Layout. Material: "); name += onMaterialLoadInfo.ResourceName;
			pipelineLayout.Name = name.begin();
		}
		
		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		instance.PipelineLayout.Initialize(pipelineLayout);
	}

	pipelineCreateInfo.PipelineDescriptor.BlendEnable = false;
	pipelineCreateInfo.PipelineDescriptor.CullMode = CullMode::CULL_BACK;
	pipelineCreateInfo.PipelineDescriptor.ColorBlendOperation = GAL::BlendOperation::ADD;

	pipelineCreateInfo.SurfaceExtent = { 1280, 720 };

	GTSL::Array<Shader, 10> shaders; uint32 offset = 0;
	for (uint32 i = 0; i < onMaterialLoadInfo.ShaderTypes.GetLength(); ++i)
	{
		Shader::CreateInfo create_info;
		create_info.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
		create_info.ShaderData = GTSL::Ranger<const byte>(onMaterialLoadInfo.ShaderSizes[i], onMaterialLoadInfo.DataBuffer + offset);
		shaders.EmplaceBack(create_info);
		offset += onMaterialLoadInfo.ShaderSizes[i];
	}

	GTSL::Array<Pipeline::ShaderInfo, 10> shader_infos;
	for (uint32 i = 0; i < shaders.GetLength(); ++i)
	{
		shader_infos.PushBack({ ConvertShaderType(onMaterialLoadInfo.ShaderTypes[i]), &shaders[i] });
	}

	pipelineCreateInfo.Stages = shader_infos;
	pipelineCreateInfo.RenderPass = loadInfo->RenderSystem->GetRenderPass();
	pipelineCreateInfo.PipelineLayout = &instance.PipelineLayout;
	instance.Pipeline = RasterizationPipeline(pipelineCreateInfo);

	loadInfo->Buffer.Free(32, GetPersistentAllocator());
	GTSL::Delete<MaterialLoadInfo>(loadInfo, GetPersistentAllocator());

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<1024> string("Added material "); string += onMaterialLoadInfo.ResourceName; string += " state with \n";
		
		uint32 i = 0, j = 0;
		for (auto& e : onMaterialLoadInfo.BindingSets)
		{
			string += "Set: "; string += i; string += '\n';

			for (auto& b : e)
			{
				string += '	'; string += "Binding: "; string += j; string += " of type "; string += BindingTypeString(GAL::BindingTypeToVulkanBindingType(b)); string += '\n';
				++j;
			}

			++i;
		}

		BE_LOG_WARNING(string);
	}
}