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

	textures.Initialize(64, GetPersistentAllocator());
	texturesRefTable.Initialize(64, GetPersistentAllocator());

	queuedBufferUpdates.Initialize(1, 2, GetPersistentAllocator());

	for(uint32 i = 0; i < MAX_CONCURRENT_FRAMES; ++i)
	{
		descriptorsUpdates.EmplaceBack();
		descriptorsUpdates.back().Initialize(GetPersistentAllocator());
	}
	
	frame = 0;
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
}

uint32 DataTypeSize(MaterialSystem::Member::DataType data)
{
	switch (data)
	{
	case MaterialSystem::Member::DataType::FLOAT32: return 4;
	case MaterialSystem::Member::DataType::INT32: return 4;
	case MaterialSystem::Member::DataType::MATRIX4: return 4 * 4 * 4;
	case MaterialSystem::Member::DataType::FVEC4: return 4 * 4;
	default: return 0;
	}
}

SetHandle MaterialSystem::AddSet(Id setName, Id parent, const SetInfo& setInfo)
{
	decltype(setsTree)::Node* parentNode;
	
	if(parent.GetHash())
	{
		parentNode = static_cast<decltype(setsTree)::Node*>(setNodes.At(parent));
	}
	else
	{
		parentNode = setsTree.GetRootNode();
	}
	
	auto* set = setsTree.AddChild(parentNode);

	set->Data.Name = setName;
	set->Data.Parent = parentNode;

	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
	//traverse tree to find parent's pipeline layouts

	auto* rootNode = setsTree.GetRootNode();
	{
		auto* iterParentNode = parentNode;
		
		while (iterParentNode->Data.Parent)
		{
			//should be insert at begin
			bindingsSetLayouts.EmplaceBack(iterParentNode->Data.BindingsSetLayout);

			iterParentNode = static_cast<decltype(setsTree)::Node*>(iterParentNode->Data.Parent);
		}
	}

	RenderSystem* renderSystem;
	
	{
		BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
		bindingsSetLayoutCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> bindingDescriptors;
		for (uint32 j = 0; j < 2; ++j)
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_BUFFER_DYNAMIC, ShaderStage::ALL, 25/*max bindings, TODO: CHECK HOW TO UPDATE*/, BindingFlags::PARTIALLY_BOUND | BindingFlags::VARIABLE_DESCRIPTOR_COUNT });
		}

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> bindingsSetLayoutName("Bindings set layout. Set: "); bindingsSetLayoutName += setName.GetString();
			bindingsSetLayoutCreateInfo.Name = bindingsSetLayoutName;
		}

		bindingsSetLayoutCreateInfo.BindingsDescriptors = bindingDescriptors;
		set->Data.BindingsSetLayout = BindingsSetLayout(bindingsSetLayoutCreateInfo);

		bindingsSetLayouts.EmplaceBack(set->Data.BindingsSetLayout);//TODO: FIX ORDER
	}

	{
		BindingsPool::CreateInfo bindingsPoolCreateInfo;
		bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<64> name("Bindings pool. Set: "); name += setName;
			bindingsPoolCreateInfo.Name = name;
		}

		GTSL::Array<BindingsPool::DescriptorPoolSize, 10> descriptorPoolSizes;
		descriptorPoolSizes.PushBack(BindingsPool::DescriptorPoolSize{ BindingType::STORAGE_BUFFER_DYNAMIC, 16 });
		bindingsPoolCreateInfo.DescriptorPoolSizes = descriptorPoolSizes;
		bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
		set->Data.BindingsPool = BindingsPool(bindingsPoolCreateInfo);
	}

	{
		BindingsPool::AllocateBindingsSetsInfo allocateBindings;
		allocateBindings.RenderDevice = renderSystem->GetRenderDevice();

		GTSL::Array<BindingsSet*, 16> bindingsSets;
		bindingsSets.EmplaceBack(&set->Data.BindingsSets[0]); bindingsSets.EmplaceBack(&set->Data.BindingsSets[1]);

		allocateBindings.BindingsSets = bindingsSets;
		
		{
			GTSL::Array<BindingsSetLayout, 6 * MAX_CONCURRENT_FRAMES> perFrameBindingsSetLayouts;
			
			for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
			{
				perFrameBindingsSetLayouts.PushBack(bindingsSetLayouts); //TODO: THINK ORDER
			}

			allocateBindings.BindingsSetLayouts = perFrameBindingsSetLayouts;
			allocateBindings.BindingsSetDynamicBindingsCounts = GTSL::Array<uint32, 2>{ 1, 1 }; //TODO: FIX

			{
				GTSL::Array<GAL::VulkanCreateInfo, MAX_CONCURRENT_FRAMES> bindingsSetsCreateInfo(MAX_CONCURRENT_FRAMES);

				if constexpr (_DEBUG)
				{
					for (uint32 j = 0; j < MAX_CONCURRENT_FRAMES; ++j)
					{
						GTSL::StaticString<64> name("BindingsSet. Set: "); name += setName;
						bindingsSetsCreateInfo[j].RenderDevice = renderSystem->GetRenderDevice();
						bindingsSetsCreateInfo[j].Name = name;
					}
				}

				allocateBindings.BindingsSetCreateInfos = bindingsSetsCreateInfo;
			}

			set->Data.BindingsPool.AllocateBindingsSets(allocateBindings);
		}
	}
	
	{
		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();
		
		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline layout. Set: "); name += setName;
			pipelineLayout.Name = name;
		}

		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		set->Data.PipelineLayout.Initialize(pipelineLayout);
	}

	//TODO: BUILD BUFFERS

	set->Data.BindingsSetLayouts = bindingsSetLayouts;

	auto place = setsBufferData.Emplace();

	{
		uint32 structSize = 0;
		
		for (auto& s : setInfo.Structs)
		{
			for (auto m : s.Members)
			{
				reinterpret_cast<byte*>(m.Handle)[0] = place;
				reinterpret_cast<byte*>(m.Handle)[1] = structSize;
				structSize += DataTypeSize(m.Type);
			}

			reinterpret_cast<byte*>(s.Handle)[0] = place;
			reinterpret_cast<byte*>(s.Handle)[1] = structSize;
		}

		setsBufferData[place].MemberSize = structSize;
	}

	
	return SetHandle(setName);
}

void MaterialSystem::AddObjects(RenderSystem* renderSystem, Id renderGroup, uint32 count)
{
	//GRAB ALL PER INSTANCE DATA
	//CALCULATE IF EXCEEDS CURRENT SIZE, IF IT DOES RESIZE

	auto& renderGroupData = renderGroupsData.At(renderGroup);
	auto& setBufferData = setsBufferData[renderGroupData.SetReference];

	const uint32 addedInstances = 1;
	
	if(setBufferData.UsedInstances + addedInstances > setBufferData.AllocatedInstances)
	{
		resizeSet(renderSystem, renderGroupData.SetReference);
		
		queuedBufferUpdates.EmplaceBack(renderGroupData.SetReference);
	}

	setBufferData.UsedInstances += addedInstances;
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
	auto* matLoadInfo = GTSL::New<MaterialLoadInfo>(GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), component, info.TextureResourceManager);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, matLoadInfo);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);

	return MaterialHandle{ info.MaterialName, component++ };
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
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(mat.Allocation.Data) + mat.TextureParametersBindings.DataSize * material.MaterialInstance;
	//
	//byte* FILL = matData + mat.Parameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::PowerOf2RoundUp(mat.TextureParametersBindings.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

ComponentReference MaterialSystem::createTexture(const CreateTextureInfo& info)
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

		Buffer scratchBuffer;

		HostRenderAllocation allocation;

		{
			RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryAllocation;
			scratchMemoryAllocation.Buffer = &scratchBuffer;
			scratchMemoryAllocation.CreateInfo = &scratchBufferCreateInfo;
			scratchMemoryAllocation.Allocation = &allocation;
			info.RenderSystem->AllocateScratchBufferMemory(scratchMemoryAllocation);
		}

		texturesRefTable.Emplace(info.TextureName, component);
		
		auto* loadInfo = GTSL::New<TextureLoadInfo>(GetPersistentAllocator(), component, GTSL::MoveRef(scratchBuffer), info.RenderSystem, allocation);

		textureLoadInfo.DataBuffer = GTSL::Range<byte*>(allocation.Size, static_cast<byte*>(allocation.Data));

		textureLoadInfo.UserData = DYNAMIC_TYPE(TextureLoadInfo, loadInfo);
	}

	info.TextureResourceManager->LoadTexture(textureLoadInfo);

	return ComponentReference(GetSystemId(), component);
}

void MaterialSystem::updateDescriptors(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	for(uint32 p = 0; p < queuedBufferUpdates.GetReference().GetPageCount(); ++p)
	{
		for(uint32 i = 0; i < queuedBufferUpdates.GetReference().GetPage(p).ElementCount(); ++i)
		{
			resizeSet(renderSystem, queuedBufferUpdates.GetReference().GetPage(p)[i]);
		}
	}

	queuedBufferUpdates.Clear();
	
	BindingsSet::BindingsSetUpdateInfo bindingsUpdateInfo;
	bindingsUpdateInfo.RenderDevice = renderSystem->GetRenderDevice();

	{
		auto& descriptorsUpdate = descriptorsUpdates[frame];

		for(uint32 s = 0; s < descriptorsUpdate.setsToUpdate.GetLength(); ++s)
		{
			auto setToUpdate = descriptorsUpdate.setsToUpdate[s];

			auto& bufferBindingsUpdate = descriptorsUpdate.PerSetBufferBindingsUpdate[s];
			auto& textureBindingsUpdate = descriptorsUpdate.PerSetTextureBindingsUpdate[s];
			
			if (bufferBindingsUpdate.GetGroupCount() || textureBindingsUpdate.GetGroupCount())
			{				
				Vector<BindingsSet::BindingUpdateInfo, BE::TAR> bindingUpdateInfos(4/*bindings sets*/, GetTransientAllocator());
				{
					for (uint32 i = 0; i < bufferBindingsUpdate.GetGroupCount(); ++i)
					{
						BindingsSet::BindingUpdateInfo bindingUpdateInfo;

						bindingUpdateInfo.Type = BindingType::STORAGE_BUFFER_DYNAMIC;
						bindingUpdateInfo.ArrayElement = bufferBindingsUpdate[i].First;
						bindingUpdateInfo.Count = bufferBindingsUpdate[i].ElementCount;
						bindingUpdateInfo.BindingsUpdates = bufferBindingsUpdate[i].Elements;

						bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
					}

					for (uint32 i = 0; i < textureBindingsUpdate.GetGroupCount(); ++i)
					{
						BindingsSet::BindingUpdateInfo bindingUpdateInfo;

						bindingUpdateInfo.Type = BindingType::COMBINED_IMAGE_SAMPLER;
						bindingUpdateInfo.ArrayElement = textureBindingsUpdate[i].First;
						bindingUpdateInfo.Count = textureBindingsUpdate[i].ElementCount;
						bindingUpdateInfo.BindingsUpdates = textureBindingsUpdate[i].Elements;

						bindingUpdateInfos.EmplaceBack(bindingUpdateInfo);
					}
				}

				bindingsUpdateInfo.BindingUpdateInfos = bindingUpdateInfos;

				setsBufferData[setToUpdate].BindingsSet[frame].Update(bindingsUpdateInfo);
			}
		}

		descriptorsUpdate.Reset();
	}
}

void MaterialSystem::updateCounter(TaskInfo taskInfo)
{
	frame = (frame + 1) % MAX_CONCURRENT_FRAMES;
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{	
	auto createMaterialInstance = [](TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo, MaterialSystem* materialSystem)
	{		
		auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

		materialSystem->materials.EmplaceAt(loadInfo->Component);
		auto& material = materialSystem->materials[loadInfo->Component];

		materialSystem->materialsMap.Emplace(onMaterialLoadInfo.ResourceName, loadInfo->Component);
		
		//TODO: FLAG READY MATERIALS
		
		auto* renderSystem = loadInfo->RenderSystem;

		//GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;

		GTSL::Array<BindingsPool::DescriptorPoolSize, 32> descriptorPoolSizes;

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
			auto& renderGroup = materialSystem->setNodes.At(onMaterialLoadInfo.RenderGroup)->Data;

			{
				PipelineLayout::CreateInfo pipelineLayout;
				pipelineLayout.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();

				if constexpr (_DEBUG) {
					GTSL::StaticString<128> name("Pipeline Layout. Material: "); name += onMaterialLoadInfo.ResourceName;
					pipelineLayout.Name = name;
				}

				pipelineLayout.BindingsSetLayouts = renderGroup.BindingsSetLayouts;
				material.PipelineLayout.Initialize(pipelineLayout);
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

			pipelineCreateInfo.SurfaceExtent = { 1, 1 };

			{
				GTSL::Array<Shader, 10> shaders; GTSL::Array<Pipeline::ShaderInfo, 16> shaderInfos;
				materialSystem->genShaderStages(loadInfo->RenderSystem->GetRenderDevice(), shaders, shaderInfos, onMaterialLoadInfo);
				
				pipelineCreateInfo.Stages = shaderInfos;

				auto* frameManager = taskInfo.GameInstance->GetSystem<FrameManager>("FrameManager");

				auto renderPassIndex = frameManager->GetRenderPassIndex(onMaterialLoadInfo.RenderPass);

				auto renderPass = frameManager->GetRenderPass(renderPassIndex);
				pipelineCreateInfo.SubPass = frameManager->GetSubPassIndex(renderPassIndex, onMaterialLoadInfo.SubPass);
				pipelineCreateInfo.RenderPass = &renderPass;
				pipelineCreateInfo.PipelineLayout = &material.PipelineLayout;
				pipelineCreateInfo.PipelineCache = renderSystem->GetPipelineCache();
				material.Pipeline = RasterizationPipeline(pipelineCreateInfo);
			}
		}

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
					textureComp = materialSystem->createTexture(createTextureInfo).Component;
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

		loadInfo->Buffer.Free(32, materialSystem->GetPersistentAllocator());
		GTSL::Delete(loadInfo, materialSystem->GetPersistentAllocator());
	};
	
	taskInfo.GameInstance->AddFreeDynamicTask(GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo, MaterialSystem*)>::Create(createMaterialInstance),
		GTSL::Array<TaskDependency, 2>{ { "RenderSystem", AccessType::READ_WRITE }, { "MaterialSystem", AccessType::READ_WRITE } }, GTSL::MoveRef(onMaterialLoadInfo), this);
}

//void MaterialSystem::test()
//{
//	MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo{};
//	MaterialLoadInfo* loadInfo = nullptr;
//
//	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
//	
//	RayTracingPipeline::CreateInfo createInfo;
//	createInfo.RenderDevice;
//	if constexpr (_DEBUG) { createInfo.Name = GTSL::StaticString<32>("RayTracing Pipeline"); }
//	createInfo.IsInheritable = false;
//
//	//TODO: MOVE TO GLOBAL SETUP
//	{
//		PipelineLayout::CreateInfo pipelineLayoutCreateInfo;
//		pipelineLayoutCreateInfo.RenderDevice = loadInfo->RenderSystem->GetRenderDevice();
//		pipelineLayoutCreateInfo.Name = GTSL::StaticString<32>("RayTracing Pipeline Layout");
//		
//		{
//			GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;
//
//			bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[0]);
//			
//			pipelineLayoutCreateInfo.BindingsSetLayouts = bindingsSetLayouts;
//
//			rayTracingPipelineLayout.Initialize(pipelineLayoutCreateInfo);
//		}
//	}
//	
//	createInfo.PipelineLayout = &rayTracingPipelineLayout;
//
//	{
//		bindingsSetLayouts.EmplaceBack(globalBindingsSetLayout[0]);
//		
//		createInfo.BindingsSetLayouts = bindingsSetLayouts;
//	}
//
//	GTSL::Vector<RayTracingPipeline::Group, BE::TAR> groups;
//	{
//		RayTracingPipeline::Group group;
//		group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES;
//		group.GeneralShader = 0;
//		group.AnyHitShader = 0;
//		group.ClosestHitShader = 0;
//		group.IntersectionShader = RayTracingPipeline::Group::SHADER_UNUSED;
//
//		groups.EmplaceBack(group);
//	}
//	
//	createInfo.Groups = groups;
//	createInfo.MaxRecursionDepth = 2;
//
//	GTSL::Array<Shader, 10> shaders; GTSL::Array<Pipeline::ShaderInfo, 16> shaderInfos;
//	
//	{
//		genShaderStages(loadInfo->RenderSystem->GetRenderDevice(), shaders, shaderInfos, onMaterialLoadInfo);
//		createInfo.Stages = shaderInfos;
//	}
//
//
//	for (auto& e : shaders) { e.Destroy(loadInfo->RenderSystem->GetRenderDevice()); }
//}

void MaterialSystem::resizeSet(RenderSystem* renderSystem, uint32 set)
{
	auto& setBufferData = setsBufferData[set];
	
	//REALLOCATE
	uint32 newBufferSize = 0;
	Buffer newBuffer; HostRenderAllocation newAllocation;

	for (uint32 i = 0; i < setBufferData.StructsSizes.GetLength(); ++i)
	{
		auto newStructSize = setBufferData.StructsSizes[i] * setBufferData.AllocatedInstances * 2;
		newBufferSize += newStructSize;
	}

	Buffer::CreateInfo createInfo;
	createInfo.RenderDevice = renderSystem->GetRenderDevice();
	createInfo.Name = GTSL::StaticString<64>("undefined");
	createInfo.Size = newBufferSize;
	createInfo.BufferType = BufferType::ADDRESS;
	createInfo.BufferType |= newBufferSize > 65535 ? BufferType::STORAGE : BufferType::UNIFORM;

	RenderSystem::BufferScratchMemoryAllocationInfo allocationInfo;
	allocationInfo.CreateInfo = &createInfo;
	allocationInfo.Allocation = &newAllocation;
	allocationInfo.Buffer = &newBuffer;
	renderSystem->AllocateScratchBufferMemory(allocationInfo);

	uint32 oldOffset = 0, newOffset = 0;

	for (uint32 i = 0; i < setBufferData.StructsSizes.GetLength(); ++i)
	{
		auto oldStructSize = setBufferData.StructsSizes[i] * setBufferData.AllocatedInstances;
		auto newStructSize = setBufferData.StructsSizes[i] * setBufferData.AllocatedInstances * 2;

		GTSL::MemCopy(oldStructSize, static_cast<byte*>(setBufferData.Allocations[frame].Data) + oldOffset, static_cast<byte*>(newAllocation.Data) + newOffset);

		oldOffset += oldStructSize;
		newOffset += newStructSize;
	}

	renderSystem->DeallocateScratchBufferMemory(setBufferData.Allocations[frame]);
	//TODO: FLAG UPDATE DESCRIPTORS

	setBufferData.AllocatedInstances *= 2;
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
		
		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.CreateInfo = &textureCreateInfo;
		allocationInfo.Texture = &textureComponent.Texture;

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
	}

	GTSL::Delete(loadInfo, GetPersistentAllocator());
}
