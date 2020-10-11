#include "MaterialSystem.h"

#include "FrameManager.h"
#include "RenderSystem.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include <GTSL/SIMD/SIMD.hpp>
#include <GAL/Texture.h>

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
	materials.Initialize(32, GetPersistentAllocator());

	//auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	minUniformBufferOffset = 64;//renderSystem->GetRenderDevice()->GetMinUniformBufferOffset(); //TODO: FIX!!!
	
	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessType::READ_WRITE }, { "RenderSystem", AccessType::READ } };
		//initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "FrameStart", "RenderStart");
		initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "RenderStartSetup", "RenderEndSetup");
	}
	
	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessType::READ_WRITE }, };
		initializeInfo.GameInstance->AddTask("updateCounter", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateCounter>(this), taskDependencies, "RenderEnd", "FrameEnd");
	}

	isRenderGroupReady.Initialize(32, GetPersistentAllocator());
	isMaterialReady.Initialize(32, GetPersistentAllocator());

	textures.Initialize(64, GetPersistentAllocator());
	texturesRefTable.Initialize(64, GetPersistentAllocator());
	
	perFrameBindingsUpdateData.Resize(MAX_CONCURRENT_FRAMES);
	for(auto& e : perFrameBindingsUpdateData)
	{
		e.Initialize(32, GetPersistentAllocator());
	}
	
	frame = 0;
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	GTSL::ForEach(renderGroups,	[&](RenderGroupData& renderGroup)
	{
		renderGroup.BindingsPool.Destroy(renderSystem->GetRenderDevice());
		renderGroup.BindingsSetLayout.Destroy(renderSystem->GetRenderDevice());
	});

	GTSL::ForEach(materials, [&](MaterialInstance& e)
	{
		e.Pipeline.Destroy(renderSystem->GetRenderDevice());
		e.BindingsPool.Destroy(renderSystem->GetRenderDevice());
		e.TextureParametersBindings.BindingsSetLayout.Destroy(renderSystem->GetRenderDevice());
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

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
		for(uint32 j = 0; j < globalState[i].GetLength(); ++j)
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ globalState[i][j], ShaderStage::ALL, 25/*max bindings, TODO: CHECK HOW TO UPDATE*/, BindingFlags::PARTIALLY_BOUND | BindingFlags::VARIABLE_DESCRIPTOR_COUNT });
		}

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Bindings set layout. Material system global state");
			bindingsSetLayoutCreateInfo.Name = name;
		}
		
		bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDescriptors;
		globalBindingsSetLayout.EmplaceBack(bindingsSetLayoutCreateInfo);
	}

	BindingsPool::CreateInfo bindingsPoolCreateInfo;
	bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

	if constexpr (_DEBUG)
	{
		GTSL::StaticString<64> name("Bindings pool. Global state");
		bindingsPoolCreateInfo.Name = name;
	}
	
	GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptor_pool_sizes;
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER_DYNAMIC, 6 });
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::COMBINED_IMAGE_SAMPLER, 16 });
	descriptor_pool_sizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::STORAGE_BUFFER_DYNAMIC, 16 });
	bindingsPoolCreateInfo.DescriptorPoolSizes = descriptor_pool_sizes;
	bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
	globalBindingsPool = BindingsPool(bindingsPoolCreateInfo);

	{
		BindingsPool::AllocateBindingsSetsInfo allocateBindingsSetsInfo;
		allocateBindingsSetsInfo.RenderDevice = renderSystem->GetRenderDevice();

		GTSL::Array<BindingsSet*, 16> bindingsSets; bindingsSets.EmplaceBack(&globalBindingsSets[0]); bindingsSets.EmplaceBack(&globalBindingsSets[1]);
		
		allocateBindingsSetsInfo.BindingsSets = bindingsSets;
		GTSL::Array<BindingsSetLayout, 6 * MAX_CONCURRENT_FRAMES> bindingsSetLayouts;
		for (uint32 i = 0; i < globalState.GetLength(); ++i)
		{
			for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
			{
				bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[i]);
			}
		}
		allocateBindingsSetsInfo.BindingsSetLayouts = bindingsSetLayouts;
		allocateBindingsSetsInfo.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>{ 2, 2 };

		{
			GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(MAX_CONCURRENT_FRAMES);

			if constexpr (_DEBUG)
			{
				for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
				{
					GTSL::StaticString<64> name("Bindings Set. Global state "); name += j;
					bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
					bindingsSetsCreateInfo[j].Name = name;
				}
			}
			
			allocateBindingsSetsInfo.BindingsSetCreateInfos = bindingsSetsCreateInfo;
		}
		
		globalBindingsPool.AllocateBindingsSets(allocateBindingsSetsInfo);
	}
	
	{
		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();
		
		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline Layout. Global state");
			pipelineLayout.Name = name;
		}

		pipelineLayout.BindingsSetLayouts = globalBindingsSetLayout;
		globalPipelineLayout.Initialize(pipelineLayout);
	}
}

void MaterialSystem::AddRenderGroup(GameInstance* gameInstance, const AddRenderGroupInfo& addRenderGroupInfo)
{
	RenderGroupData& renderGroupData = renderGroups.Emplace(addRenderGroupInfo.Name);

	RenderSystem* renderSystem = gameInstance->GetSystem<RenderSystem>("RenderSystem");

	BE_ASSERT(addRenderGroupInfo.Bindings.GetLength() < 2, "Only one binding set is supported");

	for (auto& e : perFrameBindingsUpdateData)
	{
		auto& updateData = e.RenderGroups.Emplace(addRenderGroupInfo.Name);

		updateData.BufferBindingDescriptorsUpdates.Initialize(2, GetPersistentAllocator());
		updateData.TextureBindingDescriptorsUpdates.Initialize(2, GetPersistentAllocator());
		updateData.BufferBindingTypes.Initialize(2, GetPersistentAllocator());
	}
	
	for (uint32 i = 0; i < addRenderGroupInfo.Bindings.GetLength(); ++i)
	{
		BindingsSetLayout::CreateInfo setLayout;
		setLayout.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Bindings set layout. Render group: "); name += addRenderGroupInfo.Name;
			setLayout.Name = name;
		}

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
		for (uint32 j = 0; j < addRenderGroupInfo.Bindings[i].GetLength(); ++j)
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ addRenderGroupInfo.Bindings[i][j], ShaderStage::ALL, 1, 0 });
		}
		
		setLayout.BindingsDescriptors = bindingDescriptors;

		renderGroupData.BindingsSetLayout = BindingsSetLayout(setLayout);
	}
	//Bindings set layout

	{
		BindingsPool::CreateInfo bindingsPoolCreateInfo;
		bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Bindings pool. Render group: "); name += addRenderGroupInfo.Name;
			bindingsPoolCreateInfo.Name = name;
		}

		GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptorPoolSizes;
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER_DYNAMIC, 6 });
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER, 6 });
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::COMBINED_IMAGE_SAMPLER, 16 });
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::STORAGE_BUFFER_DYNAMIC, 16 });
		bindingsPoolCreateInfo.DescriptorPoolSizes = descriptorPoolSizes;
		bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
		renderGroupData.BindingsPool = BindingsPool(bindingsPoolCreateInfo);
	}
	//Bindings pool

	{
		BindingsPool::AllocateBindingsSetsInfo allocateBindings;
		allocateBindings.RenderDevice = renderSystem->GetRenderDevice();

		GTSL::Array<BindingsSet*, 16> bindingsSets; bindingsSets.EmplaceBack(&renderGroupData.BindingsSets[0]); bindingsSets.EmplaceBack(&renderGroupData.BindingsSets[1]);
		
		allocateBindings.BindingsSets = bindingsSets;
		{
			GTSL::Array<BindingsSetLayout, 6 * MAX_CONCURRENT_FRAMES> bindingsSetLayouts;
			for (uint32 i = 0; i < addRenderGroupInfo.Bindings.GetLength(); ++i)
			{
				for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
				{
					bindingsSetLayouts.EmplaceBack(renderGroupData.BindingsSetLayout);
				}
			}

			allocateBindings.BindingsSetLayouts = bindingsSetLayouts;
			allocateBindings.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>{ 1, 1 }; //TODO: FIX

			{
				GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(MAX_CONCURRENT_FRAMES);

				if constexpr (_DEBUG)
				{
					for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
					{
						GTSL::StaticString<64> name("BindingsSet. Render Group: "); name += addRenderGroupInfo.Name;
						bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
						bindingsSetsCreateInfo[j].Name = name;
					}
				}

				allocateBindings.BindingsSetCreateInfos = bindingsSetsCreateInfo;
			}

			renderGroupData.BindingsPool.AllocateBindingsSets(allocateBindings);
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
			GTSL::StaticString<128> name("Pipeline layout. Render group: "); name += addRenderGroupInfo.Name;
			pipelineLayout.Name = name;
		}

		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		renderGroupData.PipelineLayout.Initialize(pipelineLayout);
	}

	for (uint32 i = 0; i < addRenderGroupInfo.Bindings.GetLength(); ++i)
	{
		BindingsSet::BindingsSetUpdateInfo bindingsSetUpdateInfo;
		bindingsSetUpdateInfo.RenderDevice = renderSystem->GetRenderDevice();

		for (uint32 j = 0; j < addRenderGroupInfo.Bindings[i].GetLength(); ++j)
		{
			if (addRenderGroupInfo.Bindings[i][j] == GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC)
			{
				Buffer::CreateInfo bufferInfo;
				bufferInfo.RenderDevice = renderSystem->GetRenderDevice();

				if constexpr (_DEBUG)
				{
					GTSL::StaticString<64> name("Uniform Buffer. Render group: "); name += addRenderGroupInfo.Name;
					bufferInfo.Name = name;
				}

				bufferInfo.Size = addRenderGroupInfo.Size[i][j];
				bufferInfo.BufferType = BufferType::UNIFORM;
				renderGroupData.Buffer = Buffer(bufferInfo);

				RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
				memoryAllocationInfo.Buffer = renderGroupData.Buffer;
				memoryAllocationInfo.Allocation = &renderGroupData.Allocation;
				renderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

				renderGroupData.BindingType = BindingType::UNIFORM_BUFFER_DYNAMIC;

				for (auto& e : perFrameBindingsUpdateData)
				{
					BindingsSet::BufferBindingsUpdateInfo bufferBindingsUpdateInfo;
					bufferBindingsUpdateInfo.Buffer = renderGroupData.Buffer;
					bufferBindingsUpdateInfo.Offset = 0;
					bufferBindingsUpdateInfo.Range = addRenderGroupInfo.Range[i][j];

					e.RenderGroups.At(addRenderGroupInfo.Name).BufferBindingDescriptorsUpdates.EmplaceBack(bufferBindingsUpdateInfo);
					e.RenderGroups.At(addRenderGroupInfo.Name).BufferBindingTypes.EmplaceBack(renderGroupData.BindingType);
				}
			}

			if (addRenderGroupInfo.Bindings[i][j] == GAL::VulkanBindingType::STORAGE_BUFFER_DYNAMIC)
			{
				Buffer::CreateInfo bufferInfo;
				bufferInfo.RenderDevice = renderSystem->GetRenderDevice();

				if constexpr (_DEBUG)
				{
					GTSL::StaticString<64> name("Storage buffer. Render group: "); name += addRenderGroupInfo.Name;
					bufferInfo.Name = name;
				}

				bufferInfo.Size = addRenderGroupInfo.Size[i][j];
				bufferInfo.BufferType = BufferType::STORAGE;
				renderGroupData.Buffer = Buffer(bufferInfo);

				RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
				memoryAllocationInfo.Buffer = renderGroupData.Buffer;
				memoryAllocationInfo.Allocation = &renderGroupData.Allocation;
				renderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

				renderGroupData.BindingType = BindingType::STORAGE_BUFFER_DYNAMIC;
				
				for (auto& e : perFrameBindingsUpdateData)
				{
					BindingsSet::BufferBindingsUpdateInfo bufferBindingsUpdateInfo;
					bufferBindingsUpdateInfo.Buffer = renderGroupData.Buffer;
					bufferBindingsUpdateInfo.Offset = 0;
					bufferBindingsUpdateInfo.Range = addRenderGroupInfo.Range[i][j];

					e.RenderGroups.At(addRenderGroupInfo.Name).BufferBindingDescriptorsUpdates.EmplaceBack(bufferBindingsUpdateInfo);
					e.RenderGroups.At(addRenderGroupInfo.Name).BufferBindingTypes.EmplaceBack(renderGroupData.BindingType);
				}
			}
		}
	}

	isRenderGroupReady.Emplace(addRenderGroupInfo.Name, false);
}

MaterialHandle MaterialSystem::CreateMaterial(const CreateMaterialInfo& info)
{
	uint32 material_size = 0;
	info.MaterialResourceManager->GetMaterialSize(info.MaterialName, material_size);

	GTSL::Buffer material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
	const auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE }, { "FrameManager", AccessType::READ } };
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = info.GameInstance;
	material_load_info.Name = info.MaterialName;
	material_load_info.DataBuffer = GTSL::Range<byte*>(material_buffer.GetCapacity(), material_buffer.GetData());
	auto* matLoadInfo = GTSL::New<MaterialLoadInfo>(GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), material, info.TextureResourceManager);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, matLoadInfo);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);

	return MaterialHandle{ info.MaterialName, material++ };
}

void MaterialSystem::SetDynamicMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(mat.Allocation.Data) + mat.DataSize * material.MaterialInstance;
	//
	////TODO: DEFER WRITING TO NOT OVERWRITE RUNNING FRAME
	//byte* FILL = matData + mat.DynamicParameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::PowerOf2RoundUp(mat.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

void MaterialSystem::SetMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	auto& mat = materials[material.MaterialInstance];

	auto* matData = static_cast<byte*>(mat.Allocation.Data) + mat.TextureParametersBindings.DataSize * material.MaterialInstance;

	byte* FILL = matData + mat.Parameters.At(parameterName);
	GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	FILL += GTSL::Math::PowerOf2RoundUp(mat.TextureParametersBindings.DataSize, static_cast<uint32>(minUniformBufferOffset));
	GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

System::ComponentReference MaterialSystem::createTexture(const CreateTextureInfo& info)
{
	TextureResourceManager::TextureLoadInfo textureLoadInfo;
	textureLoadInfo.GameInstance = info.GameInstance;
	textureLoadInfo.Name = info.TextureName;

	textureLoadInfo.OnTextureLoadInfo = GTSL::Delegate<void(TaskInfo, TextureResourceManager::OnTextureLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onTextureLoad>(this);

	const GTSL::Array<TaskDependency, 6> loadTaskDependencies{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };

	textureLoadInfo.ActsOn = loadTaskDependencies;

	auto component = textures.Emplace();

	{
		Buffer::CreateInfo scratchBufferCreateInfo;
		scratchBufferCreateInfo.RenderDevice = info.RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Scratch Buffer. Texture: "); name += info.TextureName.GetHash();
			scratchBufferCreateInfo.Name = name;
		}

		{
			uint32 textureSize; GAL::TextureFormat textureFormat; GTSL::Extent3D textureExtent;
			info.TextureResourceManager->GetTextureSizeFormatExtent(info.TextureName, &textureSize, &textureFormat, &textureExtent);

			RenderDevice::FindSupportedImageFormat findFormatInfo;
			findFormatInfo.TextureTiling = TextureTiling::OPTIMAL;
			findFormatInfo.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
			GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
			findFormatInfo.Candidates = candidates;
			const auto supportedFormat = info.RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormatInfo);

			scratchBufferCreateInfo.Size = textureExtent.Width * textureExtent.Depth * textureExtent.Height * FormatSize(supportedFormat);
		}

		scratchBufferCreateInfo.BufferType = BufferType::TRANSFER_SOURCE;

		auto scratchBuffer = Buffer(scratchBufferCreateInfo);

		HostRenderAllocation allocation;

		{
			RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryAllocation;
			scratchMemoryAllocation.Buffer = scratchBuffer;
			scratchMemoryAllocation.Allocation = &allocation;
			info.RenderSystem->AllocateScratchBufferMemory(scratchMemoryAllocation);
		}

		texturesRefTable.Emplace(info.TextureName, component);
		
		auto* loadInfo = GTSL::New<TextureLoadInfo>(GetPersistentAllocator(), component, GTSL::MoveRef(scratchBuffer), info.RenderSystem, allocation);

		textureLoadInfo.DataBuffer = GTSL::Range<byte*>(allocation.Size, static_cast<byte*>(allocation.Data));

		textureLoadInfo.UserData = DYNAMIC_TYPE(TextureLoadInfo, loadInfo);
	}

	info.TextureResourceManager->LoadTexture(textureLoadInfo);

	return component;
}

void MaterialSystem::UpdateRenderGroupData(const UpdateRenderGroupDataInfo& updateRenderGroupDataInfo)
{
}

void MaterialSystem::updateDescriptors(TaskInfo taskInfo)
{	
	BindingsSet::BindingsSetUpdateInfo bindingsUpdateInfo;
	bindingsUpdateInfo.RenderDevice = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem")->GetRenderDevice();

	{
		auto& bindingsUpdate = perFrameBindingsUpdateData[frame].Global;
		
		if (bindingsUpdate.BufferBindingDescriptorsUpdates.GetGroupCount() + bindingsUpdate.TextureBindingDescriptorsUpdates.GetGroupCount())
		{
			auto length = bindingsUpdate.BufferBindingDescriptorsUpdates.GetGroupCount() + bindingsUpdate.TextureBindingDescriptorsUpdates.GetGroupCount();
			
			Vector<BindingsSet::BindingUpdateInfo, BE::TAR> bindingUpdateInfos(4/*bindings sets*/, GetTransientAllocator());
			{
				for (uint32 i = 0; i < bindingsUpdate.TextureBindingDescriptorsUpdates.GetGroupCount(); ++i)
				{
					BindingsSet::BindingUpdateInfo bindingUpdateInfo;

					bindingUpdateInfo.Type = GAL::VulkanBindingType::COMBINED_IMAGE_SAMPLER;
					bindingUpdateInfo.ArrayElement = bindingsUpdate.TextureBindingDescriptorsUpdates[i].First;
					bindingUpdateInfo.Count = bindingsUpdate.TextureBindingDescriptorsUpdates[i].ElementCount;
					bindingUpdateInfo.BindingsUpdates = bindingsUpdate.TextureBindingDescriptorsUpdates[i].Elements;

					bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
				}

				for (uint32 i = 0; i < bindingsUpdate.BufferBindingDescriptorsUpdates.GetGroupCount(); ++i)
				{
					BindingsSet::BindingUpdateInfo bindingUpdateInfo;

					bindingUpdateInfo.Type = GAL::VulkanBindingType::UNIFORM_BUFFER_DYNAMIC;
					bindingUpdateInfo.ArrayElement = bindingsUpdate.BufferBindingDescriptorsUpdates[i].First;
					bindingUpdateInfo.Count = bindingsUpdate.BufferBindingDescriptorsUpdates[i].ElementCount;
					bindingUpdateInfo.BindingsUpdates = bindingsUpdate.BufferBindingDescriptorsUpdates[i].Elements;

					bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
				}
			}

			bindingsUpdateInfo.BindingUpdateInfos = bindingUpdateInfos;

			globalBindingsSets[frame].Update(bindingsUpdateInfo);

			GTSL::ForEachIndexed(perFrameBindingsUpdateData[frame].Materials, [&](uint32 index, BindingsUpdateData::Updates& updates)
				{

					isMaterialReady[index] = true;
				});

			//bindingsUpdate. += bindingsUpdate.BufferBindingDescriptorsUpdates.GetLength();
			bindingsUpdate.BufferBindingDescriptorsUpdates.Clear();
			bindingsUpdate.TextureBindingDescriptorsUpdates.Clear();
			bindingsUpdate.BufferBindingTypes.ResizeDown(0);
		}
	}

	{
		auto& bindingsUpdate = perFrameBindingsUpdateData[frame].RenderGroups;

		GTSL::PairForEach(bindingsUpdate, [&](uint64 key, BindingsUpdateData::Updates& updates)
		{
			Vector<BindingsSet::BindingUpdateInfo, BE::TAR> bindingUpdateInfos(16, updates.BufferBindingDescriptorsUpdates.GetLength(), GetTransientAllocator());
			for (uint32 i = 0; i < bindingUpdateInfos.GetLength(); ++i)
			{
				bindingUpdateInfos[i].Type = updates.BufferBindingTypes[i];
				bindingUpdateInfos[i].ArrayElement = 0;
				bindingUpdateInfos[i].Count = updates.BufferBindingDescriptorsUpdates.GetLength();
				bindingUpdateInfos[i].BindingsUpdates = updates.BufferBindingDescriptorsUpdates.GetData();
			}

			bindingsUpdateInfo.BindingUpdateInfos = bindingUpdateInfos;

			renderGroups.At(key).BindingsSets[frame].Update(bindingsUpdateInfo);
			isRenderGroupReady.At(key) = true;

			updates.BufferBindingDescriptorsUpdates.ResizeDown(0);
			updates.TextureBindingDescriptorsUpdates.ResizeDown(0);
			updates.BufferBindingTypes.ResizeDown(0);
		});
	}

	{
		auto& bindingsUpdate = perFrameBindingsUpdateData[frame].Materials;

		GTSL::ForEachIndexed(bindingsUpdate, [&](uint32 index, BindingsUpdateData::Updates& updates)
		{
			Vector<BindingsSet::BindingUpdateInfo, BE::TAR> bindingUpdateInfos(16, updates.BufferBindingDescriptorsUpdates.GetLength(), GetTransientAllocator());
			for (uint32 i = 0; i < bindingUpdateInfos.GetLength(); ++i)
			{
				bindingUpdateInfos[i].Type = updates.BufferBindingTypes[i];
				bindingUpdateInfos[i].ArrayElement = 0;
				bindingUpdateInfos[i].Count = updates.BufferBindingDescriptorsUpdates.GetLength();
				bindingUpdateInfos[i].BindingsUpdates = updates.BufferBindingDescriptorsUpdates.GetData();
			}

			bindingsUpdateInfo.BindingUpdateInfos = bindingUpdateInfos;

			materials[index].TextureParametersBindings.BindingsSets[frame].Update(bindingsUpdateInfo);
			//isMaterialReady[index] = true;
			
			updates.BufferBindingDescriptorsUpdates.ResizeDown(0);
			updates.TextureBindingDescriptorsUpdates.ResizeDown(0);
			updates.BufferBindingTypes.ResizeDown(0);
		});
	}
}

void MaterialSystem::updateCounter(TaskInfo taskInfo)
{
	frame = (frame + 1) % MAX_CONCURRENT_FRAMES;
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{	
	auto createMaterialInstance = [this](TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo, MaterialSystem* materialSystem)
	{		
		auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

		for (auto& e : materialSystem->perFrameBindingsUpdateData)
		{
			e.Materials.EmplaceAt(loadInfo->Component);
			auto& updateData = e.Materials[loadInfo->Component];

			updateData.BufferBindingDescriptorsUpdates.Initialize(2, materialSystem->GetPersistentAllocator());
			updateData.TextureBindingDescriptorsUpdates.Initialize(2, materialSystem->GetPersistentAllocator());
			updateData.BufferBindingTypes.Initialize(2, materialSystem->GetPersistentAllocator());
		}
		
		auto* renderSystem = loadInfo->RenderSystem;

		MaterialInstance instance;

		GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
		bindingsSetLayouts.PushBack(GTSL::Range<BindingsSetLayout*>(materialSystem->globalBindingsSetLayout)); //global bindings

		{
			auto& renderGroup = materialSystem->renderGroups.At(onMaterialLoadInfo.RenderGroup);
			bindingsSetLayouts.EmplaceBack(renderGroup.BindingsSetLayout); //render group bindings
		}

		GTSL::Array<BindingsPool::DescriptorPoolSize, 32> descriptorPoolSizes;

		if (onMaterialLoadInfo.Textures.GetLength())
		{
			// MATERIAL PARAMETERS
			{
				GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;

				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::UNIFORM_BUFFER, ShaderStage::ALL, 1, 0 });
				descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::UNIFORM_BUFFER, MAX_CONCURRENT_FRAMES });

				BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
				bindingsSetLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

				if constexpr (_DEBUG) {
					GTSL::StaticString<128> name("Bindings set layout. Material: "); name += onMaterialLoadInfo.ResourceName;
					bindingsSetLayoutCreateInfo.Name = name;
				}

				bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDescriptors;

				instance.TextureParametersBindings.BindingsSetLayout = BindingsSetLayout(bindingsSetLayoutCreateInfo);

				bindingsSetLayouts.EmplaceBack(instance.TextureParametersBindings.BindingsSetLayout);

				Buffer::CreateInfo bufferInfo;
				bufferInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
				if constexpr (_DEBUG) {
					GTSL::StaticString<64> name("Uniform Buffer. Material: "); name += onMaterialLoadInfo.ResourceName;
					bufferInfo.Name = name;
				}

				for (uint8 i = 0; i < onMaterialLoadInfo.Textures.GetLength(); ++i)
				{
					instance.Textures.Emplace(onMaterialLoadInfo.Textures[i], bufferInfo.Size);
					instance.TextureParametersBindings.DataSize += 4;
				}

				bufferInfo.Size += GTSL::Math::PowerOf2RoundUp(instance.TextureParametersBindings.DataSize, static_cast<uint32>(materialSystem->minUniformBufferOffset)) * 2;

				bufferInfo.BufferType = BufferType::UNIFORM;
				instance.Buffer = Buffer(bufferInfo);

				RenderSystem::BufferScratchMemoryAllocationInfo memoryAllocationInfo;
				memoryAllocationInfo.Buffer = instance.Buffer;
				memoryAllocationInfo.Allocation = &instance.Allocation;
				renderSystem->AllocateScratchBufferMemory(memoryAllocationInfo);

				instance.BindingType = BindingType::UNIFORM_BUFFER;

				for (uint32 i = 0; i < MAX_CONCURRENT_FRAMES; ++i)
				{
					auto& e = materialSystem->perFrameBindingsUpdateData[i];

					BindingsSet::BufferBindingsUpdateInfo bufferBindingsUpdateInfo;
					bufferBindingsUpdateInfo.Buffer = instance.Buffer;
					bufferBindingsUpdateInfo.Offset = GTSL::Math::PowerOf2RoundUp(instance.TextureParametersBindings.DataSize * i, static_cast<uint32>(materialSystem->minUniformBufferOffset));
					bufferBindingsUpdateInfo.Range = instance.TextureParametersBindings.DataSize;

					e.Materials[loadInfo->Component].BufferBindingDescriptorsUpdates.EmplaceBack(bufferBindingsUpdateInfo);
					e.Materials[loadInfo->Component].BufferBindingTypes.EmplaceBack(instance.BindingType);
				}
			}

			{
				BindingsPool::CreateInfo bindingsPoolCreateInfo;
				bindingsPoolCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
				if constexpr (_DEBUG) {
					GTSL::StaticString<64> name("Bindings pool. Material: "); name += onMaterialLoadInfo.ResourceName;
					bindingsPoolCreateInfo.Name = name;
				}

				BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
				bindingsSetLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

				bindingsPoolCreateInfo.DescriptorPoolSizes = descriptorPoolSizes;
				bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
				instance.BindingsPool = BindingsPool(bindingsPoolCreateInfo);

				BindingsPool::AllocateBindingsSetsInfo allocateBindingsSetsInfo;
				allocateBindingsSetsInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

#if (_DEBUG)
				GTSL::Array<GAL::VulkanCreateInfo, 32> bindingsSetsCreateInfo;
#endif

				GTSL::Array<BindingsSet*, 32> bindingsSetsToAllocate; GTSL::Array<BindingsSetLayout, 32> bindingsSetLayoutsToAllocate;

				if (onMaterialLoadInfo.Textures.GetLength())
				{
					for (uint8 i = 0; i < MAX_CONCURRENT_FRAMES; ++i)
					{
						bindingsSetsToAllocate.EmplaceBack(&instance.TextureParametersBindings.BindingsSets[i]);
						bindingsSetLayoutsToAllocate.EmplaceBack(instance.TextureParametersBindings.BindingsSetLayout);

						if constexpr (_DEBUG) {
							GTSL::StaticString<64> name("BindingsSet. Material: "); name += onMaterialLoadInfo.ResourceName;

							GAL::VulkanCreateInfo createInfo;
							createInfo.RenderDevice = renderSystem->GetRenderDevice();
							createInfo.Name = name;

							bindingsSetsCreateInfo.EmplaceBack(createInfo);
						}
					}
				}

				allocateBindingsSetsInfo.BindingsSets = bindingsSetsToAllocate;
				allocateBindingsSetsInfo.BindingsSetLayouts = bindingsSetLayoutsToAllocate;
				allocateBindingsSetsInfo.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>();

				allocateBindingsSetsInfo.BindingsSetCreateInfos = bindingsSetsCreateInfo;

				instance.BindingsPool.AllocateBindingsSets(allocateBindingsSetsInfo);
			}
		}

		{
			RasterizationPipeline::CreateInfo pipelineCreateInfo;
			pipelineCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name("Raster pipeline. Material: "); name += onMaterialLoadInfo.ResourceName;
				pipelineCreateInfo.Name = name;
			}

			{
				GTSL::Array<ShaderDataType, 10> vertexDescriptor(onMaterialLoadInfo.VertexElements.GetLength());

				for (uint32 i = 0; i < onMaterialLoadInfo.VertexElements.GetLength(); ++i)
				{
					vertexDescriptor[i] = ConvertShaderDataType(onMaterialLoadInfo.VertexElements[i]);
				}

				pipelineCreateInfo.VertexDescriptor = vertexDescriptor;
			}

			//pipelineCreateInfo.IsInheritable = true;

			{
				PipelineLayout::CreateInfo pipelineLayout;
				pipelineLayout.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

				if constexpr (_DEBUG) {
					GTSL::StaticString<128> name("Pipeline Layout. Material: "); name += onMaterialLoadInfo.ResourceName;
					pipelineLayout.Name = name;
				}

				pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
				instance.PipelineLayout.Initialize(pipelineLayout);
			}

			pipelineCreateInfo.PipelineDescriptor.BlendEnable = onMaterialLoadInfo.BlendEnable;
			pipelineCreateInfo.PipelineDescriptor.CullMode = onMaterialLoadInfo.CullMode;
			pipelineCreateInfo.PipelineDescriptor.DepthTest = onMaterialLoadInfo.DepthTest;
			pipelineCreateInfo.PipelineDescriptor.DepthWrite = onMaterialLoadInfo.DepthWrite;
			pipelineCreateInfo.PipelineDescriptor.StencilTest = onMaterialLoadInfo.StencilTest;
			pipelineCreateInfo.PipelineDescriptor.DepthCompareOperation = GAL::CompareOperation::LESS;
			pipelineCreateInfo.PipelineDescriptor.ColorBlendOperation = onMaterialLoadInfo.ColorBlendOperation;

			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.CompareOperation = onMaterialLoadInfo.Front.CompareOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.CompareMask = onMaterialLoadInfo.Front.CompareMask;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.DepthFailOperation = onMaterialLoadInfo.Front.DepthFailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.FailOperation = onMaterialLoadInfo.Front.FailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.PassOperation = onMaterialLoadInfo.Front.PassOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.Reference = onMaterialLoadInfo.Front.Reference;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Front.WriteMask = onMaterialLoadInfo.Front.WriteMask;

			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.CompareOperation = onMaterialLoadInfo.Back.CompareOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.CompareMask = onMaterialLoadInfo.Back.CompareMask;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.DepthFailOperation = onMaterialLoadInfo.Back.DepthFailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.FailOperation = onMaterialLoadInfo.Back.FailOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.PassOperation = onMaterialLoadInfo.Back.PassOperation;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.Reference = onMaterialLoadInfo.Back.Reference;
			pipelineCreateInfo.PipelineDescriptor.StencilOperations.Back.WriteMask = onMaterialLoadInfo.Back.WriteMask;

			pipelineCreateInfo.SurfaceExtent = { 1280, 720 };

			{
				GTSL::Array<Shader, 10> shaders; GTSL::Array<Pipeline::ShaderInfo, 16> shaderInfos;
				genShaderStages(loadInfo->RenderSystem->GetRenderDevice(), shaders, shaderInfos, onMaterialLoadInfo);
				
				pipelineCreateInfo.Stages = shaderInfos;

				auto* frameManager = taskInfo.GameInstance->GetSystem<FrameManager>("FrameManager");

				auto renderPassIndex = frameManager->GetRenderPassIndex(onMaterialLoadInfo.RenderPass);

				auto renderPass = frameManager->GetRenderPass(renderPassIndex);
				pipelineCreateInfo.SubPass = frameManager->GetSubPassIndex(renderPassIndex, onMaterialLoadInfo.SubPass);
				pipelineCreateInfo.RenderPass = &renderPass;
				pipelineCreateInfo.PipelineLayout = &instance.PipelineLayout;
				pipelineCreateInfo.PipelineCache = renderSystem->GetPipelineCache();
				instance.Pipeline = RasterizationPipeline(pipelineCreateInfo);
			}
		}

		bool materialIsReady = false;

		{
			uint32 offset = 0;

			for (auto& e : onMaterialLoadInfo.Textures)
			{
				uint32 textureComp;

				uint32* textureComponent;

				if (!materialSystem->texturesRefTable.Find(e, textureComponent))
				{
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.GameInstance;
					createTextureInfo.TextureResourceManager = loadInfo->TextureResourceManager;
					createTextureInfo.TextureName = e;
					createTextureInfo.MaterialHandle = MaterialHandle{ onMaterialLoadInfo.ResourceName, loadInfo->Component };
					textureComp = materialSystem->createTexture(createTextureInfo);
				}
				else
				{
					textureComp = *textureComponent;
				}

				auto* to = static_cast<byte*>(instance.Allocation.Data);

				for(uint32 frame = 0; frame < MAX_CONCURRENT_FRAMES; ++frame)
				{
					GTSL::MemCopy(4, &textureComp, (to + (materialSystem->minUniformBufferOffset * frame)) + offset);
				}

				offset += 4; //sizeof(uint32)
			}
		}

		materialSystem->isMaterialReady.EmplaceAt(loadInfo->Component, materialIsReady);
		materialSystem->materials.EmplaceAt(loadInfo->Component, instance);

		loadInfo->Buffer.Free(32, materialSystem->GetPersistentAllocator());
		GTSL::Delete(loadInfo, materialSystem->GetPersistentAllocator());
	};
	
	taskInfo.GameInstance->AddFreeDynamicTask(GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo, MaterialSystem*)>::Create(createMaterialInstance),
		GTSL::Array<TaskDependency, 2>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } }, GTSL::MoveRef(onMaterialLoadInfo), this);
}

void MaterialSystem::test()
{
	MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo{};
	MaterialLoadInfo* loadInfo = nullptr;

	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
	
	RayTracingPipeline::CreateInfo createInfo;
	createInfo.RenderDevice;
	if constexpr (_DEBUG) { createInfo.Name = GTSL::StaticString<32>("RayTracing Pipeline"); }
	createInfo.IsInheritable = false;

	//TODO: MOVE TO GLOBAL SETUP
	{
		PipelineLayout::CreateInfo pipelineLayoutCreateInfo;
		pipelineLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
		pipelineLayoutCreateInfo.Name = GTSL::StaticString<32>("RayTracing Pipeline Layout");
		
		{
			GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;

			bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[0]);
			
			pipelineLayoutCreateInfo.BindingsSetLayouts = bindingsSetLayouts;

			rayTracingPipelineLayout.Initialize(pipelineLayoutCreateInfo);
		}
	}
	
	createInfo.PipelineLayout = &rayTracingPipelineLayout;

	{
		bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[0]);
		
		createInfo.BindingsSetLayouts = bindingsSetLayouts;
	}

	GTSL::Vector<RayTracingPipeline::Group, BE::TAR> groups;
	{
		RayTracingPipeline::Group group;
		group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES;
		group.GeneralShader = 0;
		group.AnyHitShader = 0;
		group.ClosestHitShader = 0;
		group.IntersectionShader = RayTracingPipeline::Group::SHADER_UNUSED;

		groups.EmplaceBack(group);
	}
	
	createInfo.Groups = groups;
	createInfo.MaxRecursionDepth = 2;

	GTSL::Array<Shader, 10> shaders; GTSL::Array<Pipeline::ShaderInfo, 16> shaderInfos;
	
	{
		genShaderStages(loadInfo->RenderSystem->GetRenderDevice(), shaders, shaderInfos, onMaterialLoadInfo);
		createInfo.Stages = shaderInfos;
	}


	for (auto& e : shaders) { e.Destroy(loadInfo->RenderSystem->GetRenderDevice()); }
}

void MaterialSystem::onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	{
		auto* loadInfo = DYNAMIC_CAST(TextureLoadInfo, onTextureLoadInfo.UserData);

		RenderDevice::FindSupportedImageFormat findFormat;
		findFormat.TextureTiling = TextureTiling::OPTIMAL;
		findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
		GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(onTextureLoadInfo.TextureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
		findFormat.Candidates = candidates;
		auto supportedFormat = loadInfo->RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

		GAL::Texture::ConvertTextureFormat(onTextureLoadInfo.TextureFormat, GAL::TextureFormat::RGBA_I8, onTextureLoadInfo.Extent, GTSL::AlignedPointer<byte, 16>(onTextureLoadInfo.DataBuffer.begin()), 1);

		{
			const GTSL::Array<TaskDependency, 6> loadTaskDependencies{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } };

			taskInfo.GameInstance->AddFreeDynamicTask(GTSL::Delegate<void(TaskInfo, TextureResourceManager::OnTextureLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onTextureProcessed>(this),
				loadTaskDependencies, GTSL::MoveRef(onTextureLoadInfo));
		}
	}
}

void MaterialSystem::onTextureProcessed(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo onTextureLoadInfo)
{
	auto* loadInfo = DYNAMIC_CAST(TextureLoadInfo, onTextureLoadInfo.UserData);

	RenderDevice::FindSupportedImageFormat findFormat;
	findFormat.TextureTiling = TextureTiling::OPTIMAL;
	findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(onTextureLoadInfo.TextureFormat)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	findFormat.Candidates = candidates;
	auto supportedFormat = loadInfo->RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);
	
	TextureComponent textureComponent;

	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
		textureCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureCreateInfo.Format = static_cast<GAL::VulkanTextureFormat>(supportedFormat);
		textureCreateInfo.Extent = onTextureLoadInfo.Extent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;

		textureComponent.Texture = Texture(textureCreateInfo);
	}

	{
		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.Texture = textureComponent.Texture;

		loadInfo->RenderSystem->AllocateLocalTextureMemory(allocationInfo);
	}

	{
		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture view. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureViewCreateInfo.Name = name;
		}

		textureViewCreateInfo.Type = GAL::VulkanTextureType::COLOR;
		textureViewCreateInfo.Dimensions = ConvertDimension(onTextureLoadInfo.Dimensions);
		textureViewCreateInfo.Format = static_cast<GAL::VulkanTextureFormat>(supportedFormat);
		textureViewCreateInfo.Texture = textureComponent.Texture;
		textureViewCreateInfo.MipLevels = 1;

		textureComponent.TextureView = TextureView(textureViewCreateInfo);
	}

	{
		RenderSystem::TextureCopyData textureCopyData;
		textureCopyData.DestinationTexture = textureComponent.Texture;
		textureCopyData.SourceBuffer = loadInfo->Buffer;
		textureCopyData.Allocation = loadInfo->RenderAllocation;
		textureCopyData.Layout = TextureLayout::TRANSFER_DST;
		textureCopyData.Extent = onTextureLoadInfo.Extent;

		loadInfo->RenderSystem->AddTextureCopy(textureCopyData);
	}

	{
		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Texture sampler. Texture: "); name += onTextureLoadInfo.ResourceName;
			textureSamplerCreateInfo.Name = name;
		}

		textureSamplerCreateInfo.Anisotropy = 0;

		textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	}

	textures[loadInfo->Component] = textureComponent;

	BE_LOG_MESSAGE("Loaded texture ", onTextureLoadInfo.ResourceName)


	BindingsSet::TextureBindingsUpdateInfo textureBindingsUpdateInfo;

	textureBindingsUpdateInfo.TextureView = textureComponent.TextureView;
	textureBindingsUpdateInfo.Sampler = textureComponent.TextureSampler;
	textureBindingsUpdateInfo.TextureLayout = TextureLayout::SHADER_READ_ONLY;
	for (auto& e : perFrameBindingsUpdateData)
	{
		e.Global.TextureBindingDescriptorsUpdates.EmplaceAt(loadInfo->Component, textureBindingsUpdateInfo);
		//if(e.Global.TexturesToUpdateFrom < loadInfo->Component)
		//{
		//	e.Global.TexturesToUpdateFrom = loadInfo->Component;
		//}
	}

	GTSL::Delete(loadInfo, GetPersistentAllocator());
}
