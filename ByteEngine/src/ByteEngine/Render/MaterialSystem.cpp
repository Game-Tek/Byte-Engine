#include "MaterialSystem.h"

#include "RenderSystem.h"

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

	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	minUniformBufferOffset = renderSystem->GetRenderDevice()->GetMinUniformBufferOffset();
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
			GTSL::StaticString<128> name("Bindings set layout. Material system global state");
			bindingsSetLayoutCreateInfo.Name = name.begin();
		}
		
		bindingsSetLayoutCreateInfo.BindingsDescriptors = binding_descriptors;
		globalBindingsSetLayout.EmplaceBack(bindingsSetLayoutCreateInfo);
	}

	BindingsPool::CreateInfo bindingsPoolCreateInfo;
	bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Bindings pool. Global state");
		bindingsPoolCreateInfo.Name = name.begin();
	}
	
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

		{
			GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(2); //TODO: FILL WITH CORRECT VALUE

			if constexpr (_DEBUG)
			{
				for (uint32 j = 0; j < 2; ++j)
				{
					GTSL::StaticString<64> name("BindingsSet. Global state");
					bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
					bindingsSetsCreateInfo[j].Name = name.begin();
				}
			}
			
			allocateBindingsSetsInfo.BindingsSetCreateInfos = bindingsSetsCreateInfo;
		}
		
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
			GTSL::StaticString<64> name("Bindings set layout. Render group: "); name += renderGroupName;
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

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Bindings pool. Render group: "); name += renderGroupName;
			bindingsPoolCreateInfo.Name = name.begin();
		}
		
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

			{
				GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(2);

				if constexpr (_DEBUG)
				{
					for (uint32 j = 0; j < 2; ++j)
					{
						GTSL::StaticString<64> name("BindingsSet. Render Group: "); name += renderGroupName;
						bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
						bindingsSetsCreateInfo[j].Name = name.begin();
					}
				}

				allocateBindings.BindingsSetCreateInfos = bindingsSetsCreateInfo;
			}
			
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
			GTSL::StaticString<128> name("Pipeline layout. Render group: "); name += renderGroupName;
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

				if constexpr (_DEBUG)
				{
					GTSL::StaticString<64> name("Uniform Buffer. Render group: "); name += renderGroupName;
					bufferInfo.Name = name.begin();
				}
				
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

		for (auto& e : renderGroupData.BindingsSets)
		{
			e.Update(bindingsSetUpdateInfo);
		}
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
	GTSL::New<MaterialLoadInfo>(&mat_load_info, GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), component);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, mat_load_info);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);

	return component++;
}

void MaterialSystem::SetMaterialParameter(const ComponentReference material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	auto& mat = renderGroups.At(materialNames[material].First).Instances.At(materialNames[material].Second);

	uint32 parameter = 0;
	for (auto e : mat.ShaderParameters.ParameterNames)
	{
		if (e == parameterName) break;
		++parameter;
	}
	BE_ASSERT(parameter != mat.ShaderParameters.ParameterNames.GetLength(), "Ooops");

	//TODO: DEFER WRITING TO NOT OVERWRITE RUNNING FRAME
	byte* FILL = static_cast<byte*>(mat.Data);
	GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL + mat.ShaderParameters.ParameterOffset[parameter]);
	FILL = static_cast<byte*>(mat.Data) + GTSL::Math::PowerOf2RoundUp(mat.DataSize, static_cast<uint32>(minUniformBufferOffset));
	GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL + mat.ShaderParameters.ParameterOffset[parameter]);
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{
	auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

	auto* renderSystem = loadInfo->RenderSystem;
	
	auto& renderGroup = renderGroups.At(onMaterialLoadInfo.RenderGroup);
	auto& instance = renderGroup.Instances.Emplace(onMaterialLoadInfo.ResourceName);

	BindingsPool::CreateInfo bindingsPoolCreateInfo;
	bindingsPoolCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Bindings pool. Material: "); name += onMaterialLoadInfo.ResourceName;
		bindingsPoolCreateInfo.Name = name.begin();
	}
	
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptorPoolSizes;
	
	BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
	bindingsSetLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
	for(auto& e : onMaterialLoadInfo.BindingSets[0])
	{
		auto bindingType = BindingTypeToVulkanBindingType(e.Type);
		bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindingType, ShaderStage::ALL, 1 });
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ bindingType, 3 }); //TODO: ASK FOR CORRECT NUMBER OF DESCRIPTORS
	}
	bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDescriptors;
	bindingsSetLayoutCreateInfo.SpecialBindings = GTSL::Ranger<const uint32>();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<128> name("Bindings set layout. Material: "); name += onMaterialLoadInfo.ResourceName;
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

	{
		GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(2); //TODO: FILL WITH CORRECT VALUE

		if constexpr (_DEBUG)
		{
			for (uint32 j = 0; j < 2; ++j)
			{
				GTSL::StaticString<64> name("BindingsSet. Material: "); name += onMaterialLoadInfo.ResourceName;
				
				bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
				bindingsSetsCreateInfo[j].Name = name.begin();
			}
		}

		allocateBindingsSetsInfo.BindingsSetCreateInfos = bindingsSetsCreateInfo;
	}
	
	instance.BindingsPool.AllocateBindingsSets(allocateBindingsSetsInfo);
	instance.BindingsSets.Resize(loadInfo->RenderSystem->GetFrameCount());
	
	GTSL::Array<ShaderDataType, 10> vertexDescriptor(onMaterialLoadInfo.VertexElements.GetLength());
	for(uint32 i = 0; i < onMaterialLoadInfo.VertexElements.GetLength(); ++i)
	{
		vertexDescriptor[i] = ConvertShaderDataType(onMaterialLoadInfo.VertexElements[i]);
	}
	RasterizationPipeline::CreateInfo pipelineCreateInfo;
	pipelineCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Raster pipeline. Material: "); name += onMaterialLoadInfo.ResourceName;
		pipelineCreateInfo.Name = name.begin();
	}
	pipelineCreateInfo.VertexDescriptor = vertexDescriptor;
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

	{
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
		pipelineCreateInfo.PipelineCache = renderSystem->GetPipelineCache();
		instance.Pipeline = RasterizationPipeline(pipelineCreateInfo);
	}
	
	loadInfo->Buffer.Free(32, GetPersistentAllocator());
	GTSL::Delete<MaterialLoadInfo>(loadInfo, GetPersistentAllocator());

	//SETUP MATERIAL UNIFORMS FROM LOADED DATA
	{
		uint32 offset = 0;
		
		for (uint32 i = 0; i < onMaterialLoadInfo.Uniforms.GetLength(); ++i)
		{
			for (uint32 j = 0; j < onMaterialLoadInfo.Uniforms[i].GetLength(); ++j)
			{
				instance.ShaderParameters.ParameterNames.EmplaceBack(onMaterialLoadInfo.Uniforms[i][j].Name);
				instance.ShaderParameters.ParameterOffset.EmplaceBack(offset);
				offset += ShaderDataTypesSize(onMaterialLoadInfo.Uniforms[i][j].Type);
			}
		}

		instance.DataSize = offset;
	}

	for (uint32 i = 0; i < onMaterialLoadInfo.BindingSets.GetLength(); ++i)
	{
		BindingsSet::BindingsSetUpdateInfo bindingsSetUpdateInfo;
		bindingsSetUpdateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		for (uint32 j = 0; j < onMaterialLoadInfo.Uniforms[i].GetLength(); ++j)
		{
			if (onMaterialLoadInfo.BindingSets[i][j].Type == GAL::BindingType::UNIFORM_BUFFER_DYNAMIC)
			{
				Buffer::CreateInfo bufferInfo;

				if constexpr (_DEBUG)
				{
					GTSL::StaticString<64> name("Uniform Buffer. Material: "); name += onMaterialLoadInfo.ResourceName;
					bufferInfo.Name = name.begin();
				}
				
				bufferInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
				bufferInfo.Size = 1024;
				bufferInfo.BufferType = BufferType::UNIFORM;
				instance.Buffer = Buffer(bufferInfo);

				DeviceMemory memory;

				RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
				memoryAllocationInfo.Buffer = instance.Buffer;
				memoryAllocationInfo.Allocation = &instance.Allocation;
				memoryAllocationInfo.Data = &instance.Data;
				memoryAllocationInfo.DeviceMemory = &memory;
				renderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

				Buffer::BindMemoryInfo bindMemory;
				bindMemory.RenderDevice = renderSystem->GetRenderDevice();
				bindMemory.Memory = &memory;
				bindMemory.Offset = instance.Allocation.Offset;
				instance.Buffer.BindToMemory(bindMemory);

				BindingsSetLayout::BufferBindingDescriptor bufferBindingDescriptor;
				bufferBindingDescriptor.UniformCount = 1;
				bufferBindingDescriptor.BindingType = ConvertBindingType(onMaterialLoadInfo.BindingSets[i][j].Type);
				bufferBindingDescriptor.Buffers = GTSL::Ranger<Buffer>(1, &instance.Buffer);
				bufferBindingDescriptor.Sizes = GTSL::Array<uint32, 1>{ instance.DataSize };
				bufferBindingDescriptor.Offsets = GTSL::Array<uint32, 1>{ 0 };
				bindingsSetUpdateInfo.BufferBindingsSetLayout.EmplaceBack(bufferBindingDescriptor);
			}
			else
			{
				__debugbreak();
			}
		}

		for (auto& e : instance.BindingsSets)
		{
			e.Update(bindingsSetUpdateInfo);
		}
	}

	materialNames.Insert(loadInfo->Component, onMaterialLoadInfo.RenderGroup, onMaterialLoadInfo.ResourceName);
}
