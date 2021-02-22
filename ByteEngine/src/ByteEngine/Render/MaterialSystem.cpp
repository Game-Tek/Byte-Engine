#include "MaterialSystem.h"

#include "RenderSystem.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include <GTSL/SIMD/SIMD.hpp>
#include <GAL/Texture.h>

#include "RenderOrchestrator.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Application.h"

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
	auto* renderSystem = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessTypes::READ_WRITE }, { "RenderSystem", AccessTypes::READ } };
		//initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "FrameStart", "RenderStart");
		initializeInfo.GameInstance->AddTask("updateDescriptors", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateDescriptors>(this), taskDependencies, "RenderStartSetup", "RenderEndSetup");
	}

	{
		const GTSL::Array<TaskDependency, 6> taskDependencies{ { "MaterialSystem", AccessTypes::READ_WRITE }, };
		initializeInfo.GameInstance->AddTask("updateCounter", GTSL::Delegate<void(TaskInfo)>::Create<MaterialSystem, &MaterialSystem::updateCounter>(this), taskDependencies, "RenderEnd", "FrameEnd");
	}

	initializeInfo.GameInstance->AddEvent("MaterialSystem", GetOnMaterialLoadEventHandle());
	initializeInfo.GameInstance->AddEvent("MaterialSystem", GetOnMaterialInstanceLoadEventHandle());
	
	queuedFrames = BE::Application::Get()->GetOption("buffer");
	queuedFrames = GTSL::Math::Clamp(queuedFrames, (uint8)2, (uint8)3);

	buffers.Initialize(64, GetPersistentAllocator()); buffersByName.Initialize(32, GetPersistentAllocator());
	
	textures.Initialize(64, GetPersistentAllocator());
	texturesRefTable.Initialize(64, GetPersistentAllocator());

	queuedSetUpdates.Initialize(1, 2, GetPersistentAllocator());

	latestLoadedTextures.Initialize(8, GetPersistentAllocator());
	pendingMaterialsPerTexture.Initialize(16, GetPersistentAllocator());

	materials.Initialize(16, GetPersistentAllocator()); materialInstances.Initialize(32, GetPersistentAllocator());
	loadedMaterialsMap.Initialize(32, GetPersistentAllocator()); materialInstancesMap.Initialize(32, GetPersistentAllocator());
	readyMaterialHandles.Initialize(16, GetPersistentAllocator());

	privateMaterialHandlesByName.Initialize(32, GetPersistentAllocator());
	
	setHandlesByName.Initialize(16, GetPersistentAllocator());

	renderGroupsData.Initialize(4, GetPersistentAllocator());
	readyMaterialsPerRenderGroup.Initialize(8, GetPersistentAllocator());

	shaderGroupsByName.Initialize(16, GetPersistentAllocator());
	setLayoutDatas.Initialize(16, GetPersistentAllocator());
	
	sets.Initialize(16, GetPersistentAllocator());
	
	for (uint32 i = 0; i < queuedFrames; ++i)
	{
		descriptorsUpdates.EmplaceBack();
		descriptorsUpdates.back().Initialize(GetPersistentAllocator());
	}

	frame = 0;

	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "MaterialSystem", AccessTypes::READ_WRITE } };
		onTextureInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onTextureInfoLoad", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<MaterialSystem, &MaterialSystem::onTextureInfoLoad>(this), taskDependencies);
	}

	{

		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "MaterialSystem", AccessTypes::READ_WRITE } };
		onTextureLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("loadTexture", Task<TextureResourceManager*, TextureResourceManager::TextureInfo, TextureLoadInfo>::Create<MaterialSystem, &MaterialSystem::onTextureLoad>(this), taskDependencies);
	}

	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "MaterialSystem", AccessTypes::READ } };
		onShaderInfosLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onShaderInfosLoaded", Task<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo>::Create<MaterialSystem, &MaterialSystem::onShaderInfosLoaded>(this), taskDependencies);
	}
	
	{
		const auto taskDependencies = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "MaterialSystem", AccessTypes::READ_WRITE } };
		onShadersLoad = initializeInfo.GameInstance->StoreDynamicTask("onShadersLoaded", Task<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo>::Create<MaterialSystem, &MaterialSystem::onShadersLoaded>(this), taskDependencies);
	}
	
	{		
		GTSL::Array<SubSetInfo, 10> subSetInfos;

		{ //TEXTURES
			SubSetInfo subSetInfo;
			subSetInfo.Type = SubSetType::READ_TEXTURES;
			subSetInfo.Count = 16;
			subSetInfo.Handle = &textureSubsetsHandle;
			subSetInfos.EmplaceBack(subSetInfo);
		}
		
		if (BE::Application::Get()->GetOption("rayTracing"))
		{
			{ //TOP LEVEL AS
				SubSetInfo subSetInfo;
				subSetInfo.Type = SubSetType::ACCELERATION_STRUCTURE;
				subSetInfo.Handle = &topLevelAsHandle;
				subSetInfo.Count = 1;
				subSetInfos.EmplaceBack(subSetInfo);
			}
		}

		AddSetLayout(renderSystem, "GlobalData", Id(), subSetInfos);
		AddSet(renderSystem, "GlobalData", "GlobalData", subSetInfos);
	}

	if (BE::Application::Get()->GetOption("rayTracing"))
	{		
		auto* materialResorceManager = BE::Application::Get()->GetResourceManager<MaterialResourceManager>("MaterialResourceManager");

		Buffer::CreateInfo sbtCreateInfo;
		sbtCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) { sbtCreateInfo.Name = GTSL::StaticString<32>("SBT Buffer. Material System"); }
		sbtCreateInfo.Size = materialResorceManager->GetRayTraceShaderCount() * renderSystem->GetShaderGroupHandleSize();
		sbtCreateInfo.BufferType = BufferType::SHADER_BINDING_TABLE | BufferType::ADDRESS;
		RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryInfo;
		scratchMemoryInfo.Buffer = &shaderBindingTableBuffer;
		scratchMemoryInfo.CreateInfo = &sbtCreateInfo;
		scratchMemoryInfo.Allocation = &shaderBindingTableAllocation;
		renderSystem->AllocateScratchBufferMemory(scratchMemoryInfo);

		GTSL::Vector<RayTracingPipeline::Group, BE::TAR> groups(16, GetTransientAllocator());
		GTSL::Vector<Pipeline::ShaderInfo, BE::TAR> shaderInfos(16, GetTransientAllocator());
		GTSL::Vector<GTSL::Buffer<BE::TAR>, BE::TAR> shaderBuffers(16, GetTransientAllocator());

		for (uint32 i = 0; i < materialResorceManager->GetRayTraceShaderCount(); ++i)
		{
			uint32 bufferSize = 0;
			bufferSize = materialResorceManager->GetRayTraceShaderSize(materialResorceManager->GetRayTraceShaderHandle(i));
			shaderBuffers.EmplaceBack();
			shaderBuffers[i].Allocate(bufferSize, 8, GetTransientAllocator());

			shaderGroupsByName.Emplace(materialResorceManager->GetRayTraceShaderHandle(i)(), i);
			
			auto material = materialResorceManager->LoadRayTraceShaderSynchronous(materialResorceManager->GetRayTraceShaderHandle(i), GTSL::Range<byte*>(shaderBuffers[i].GetCapacity(), shaderBuffers[i].GetData())); //TODO: VIRTUAL BUFFER INTERFACE
			
			Pipeline::ShaderInfo shaderInfo;
			shaderInfo.Blob = GTSL::Range<const byte*>(material.BinarySize, shaderBuffers[i].GetData());
			shaderInfo.Type = ConvertShaderType(material.ShaderType);
			shaderInfos.EmplaceBack(shaderInfo);

			RayTracingPipeline::Group group{};

			group.GeneralShader = RayTracingPipeline::Group::SHADER_UNUSED; group.ClosestHitShader = RayTracingPipeline::Group::SHADER_UNUSED;
			group.AnyHitShader = RayTracingPipeline::Group::SHADER_UNUSED; group.IntersectionShader = RayTracingPipeline::Group::SHADER_UNUSED;

			switch (material.ShaderType)
			{
			case GAL::ShaderType::RAY_GEN: {
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++shaderCounts[GAL::RAY_GEN_TABLE_INDEX];
				break;
			}
			case GAL::ShaderType::MISS: {
				//generalShader is the index of the ray generation,miss, or callable shader from VkRayTracingPipelineCreateInfoKHR::pStages
				//in the group if the shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_GENERAL_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++shaderCounts[GAL::MISS_TABLE_INDEX];
				break;
			}
			case GAL::ShaderType::CALLABLE: {
				group.ShaderGroup = GAL::VulkanShaderGroupType::GENERAL; group.GeneralShader = i;
				++shaderCounts[GAL::CALLABLE_TABLE_INDEX];
				break;
			}
			case GAL::ShaderType::CLOSEST_HIT: {
				//closestHitShader is the optional index of the closest hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES; group.ClosestHitShader = i;
				++shaderCounts[GAL::HIT_TABLE_INDEX];
				break;
			}
			case GAL::ShaderType::ANY_HIT: {
				//anyHitShader is the optional index of the any-hit shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the
				//shader group has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_TRIANGLES_HIT_GROUP_KHR or VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR,
				//and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::TRIANGLES; group.AnyHitShader = i;
				++shaderCounts[GAL::HIT_TABLE_INDEX];
				break;
			}
			case GAL::ShaderType::INTERSECTION: {
				//intersectionShader is the index of the intersection shader from VkRayTracingPipelineCreateInfoKHR::pStages in the group if the shader group
				//has type of VK_RAY_TRACING_SHADER_GROUP_TYPE_PROCEDURAL_HIT_GROUP_KHR, and VK_SHADER_UNUSED_KHR otherwise.
				group.ShaderGroup = GAL::VulkanShaderGroupType::PROCEDURAL; group.IntersectionShader = i;
				++shaderCounts[GAL::HIT_TABLE_INDEX];
				break;
			}

			default: BE_LOG_MESSAGE("Non raytracing shader found in raytracing material");
			}

			groups.EmplaceBack(group);
		}
		
		RayTracingPipeline::CreateInfo createInfo;
		createInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<32> name("Ray Tracing Pipeline: "); createInfo.Name = name;
		}
		
		createInfo.MaxRecursionDepth = 3;
		createInfo.Stages = shaderInfos;
		
		createInfo.PipelineLayout = setLayoutDatas[Id("GlobalData")()].PipelineLayout;

		createInfo.Groups = groups;
		rayTracingPipeline.Initialize(createInfo);
		
		auto handleSize = renderSystem->GetShaderGroupHandleSize();
		auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupBaseAlignment());

		GTSL::Buffer<BE::TAR> handlesBuffer; handlesBuffer.Allocate(groups.GetLength() * alignedHandleSize, renderSystem->GetShaderGroupBaseAlignment(), GetTransientAllocator());

		rayTracingPipeline.GetShaderGroupHandles(renderSystem->GetRenderDevice(), 0, groups.GetLength(), GTSL::Range<byte*>(handlesBuffer.GetCapacity(), handlesBuffer.GetData()));

		auto* sbt = reinterpret_cast<byte*>(shaderBindingTableAllocation.Data);

		for (uint32 h = 0; h < groups.GetLength(); ++h)
		{
			GTSL::MemCopy(handleSize, handlesBuffer.GetData() + h * handleSize, sbt + alignedHandleSize * h);
		}

		for(auto& e : descriptorsUpdates)
		{
			e.AddAccelerationStructureUpdate(topLevelAsHandle, 0, BindingType::ACCELERATION_STRUCTURE, BindingsSet::AccelerationStructureBindingUpdateInfo{ renderSystem->GetTopLevelAccelerationStructure() });
		}
	}
}

void MaterialSystem::Shutdown(const ShutdownInfo& shutdownInfo)
{
	RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
}

void MaterialSystem::BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle setHandle, PipelineType pipelineType)
{
	if constexpr (_DEBUG)
	{
		//if(!setHandlesByName.Find(setHandle())) { BE_LOG_ERROR("Tried to bind set which doesn't exist at render time!. ", BE::FIX_OR_CRASH_STRING) }
	}

	auto& set = sets[setHandle()];

	if (set.BindingsSet[frame].GetHandle())
	{
		//FUTURE: if we ever support buffer dynamic offset intead of indexing remember to implement switch for descriptors and here to supply offsets

		CommandBuffer::BindBindingsSetInfo bindBindingsSetInfo;
		bindBindingsSetInfo.RenderDevice = renderSystem->GetRenderDevice();
		bindBindingsSetInfo.FirstSet = set.Level;
		bindBindingsSetInfo.BoundSets = 1;
		bindBindingsSetInfo.BindingsSets = GTSL::Range<BindingsSet*>(1, &set.BindingsSet[frame]);
		bindBindingsSetInfo.PipelineLayout = set.PipelineLayout;
		bindBindingsSetInfo.PipelineType = pipelineType;
		bindBindingsSetInfo.Offsets = GTSL::Range<const uint32*>();
		commandBuffer.BindBindingsSets(bindBindingsSetInfo);
	}
}

bool MaterialSystem::BindMaterial(RenderSystem* renderSystem, CommandBuffer commandBuffer, MaterialHandle materialHandle)
{	
	CommandBuffer::BindPipelineInfo bindPipelineInfo;
	bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
	bindPipelineInfo.PipelineType = PipelineType::RASTER;
	bindPipelineInfo.Pipeline = materials[loadedMaterialsMap[materialHandle()]].Pipeline;
	commandBuffer.BindPipeline(bindPipelineInfo);
		
	return true;
}

void MaterialSystem::AddSetLayout(RenderSystem* renderSystem, Id layoutName, Id parentName, const GTSL::Range<SubSetDescriptor*> members)
{
	Id parentHandle;
	uint32 level;

	if (parentName()) {
		auto& parentSetLayout = setLayoutDatas[parentName()];
		
		parentHandle = parentName;
		level = parentSetLayout.Level + 1;
	}
	else {
		parentHandle = Id();
		level = 0;
	}

	auto& setLayoutData = setLayoutDatas.Emplace(layoutName());
	
	setLayoutData.Parent = parentHandle;
	setLayoutData.Level = level;

	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts(level); //"Pre-Allocate" _level_ elements as to be able to place them in order while traversing tree upwards

	// Traverse tree to find parent's pipeline layouts
	{
		auto lastSet = parentHandle;
	
		for (uint8 i = 0, l = level - 1; i < level; ++i, --l)
		{
			bindingsSetLayouts[l] = setLayoutDatas[lastSet()].BindingsSetLayout;
			lastSet = setLayoutDatas[lastSet()].Parent;
		}
	}

	{
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (auto e : members)
		{
			BindingType bindingType;

			switch (e.SubSetType)
			{
			case SubSetType::BUFFER: bindingType = BindingType::STORAGE_BUFFER; break;
			case SubSetType::READ_TEXTURES: bindingType = BindingType::COMBINED_IMAGE_SAMPLER; break;
			case SubSetType::WRITE_TEXTURES: bindingType = BindingType::STORAGE_IMAGE; break;
			case SubSetType::RENDER_ATTACHMENT: bindingType = BindingType::INPUT_ATTACHMENT; break;
			case SubSetType::ACCELERATION_STRUCTURE: bindingType = BindingType::ACCELERATION_STRUCTURE; break;
			}

			subSetDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindingType, ShaderStage::ALL, e.BindingsCount, BindingFlags::PARTIALLY_BOUND });
		}
		
		BindingsSetLayout::CreateInfo bindingsSetLayoutCreateInfo;
		bindingsSetLayoutCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		GTSL::StaticString<64> bindingsSetLayoutName("Set layout: "); bindingsSetLayoutName += layoutName.GetString();
		bindingsSetLayoutCreateInfo.Name = bindingsSetLayoutName;

		bindingsSetLayoutCreateInfo.BindingsDescriptors = subSetDescriptors;
		setLayoutData.BindingsSetLayout = BindingsSetLayout(bindingsSetLayoutCreateInfo);

		bindingsSetLayouts.EmplaceBack(setLayoutData.BindingsSetLayout);
	}

	{
		PipelineLayout::CreateInfo pipelineLayout;
		pipelineLayout.RenderDevice = renderSystem->GetRenderDevice();

		if constexpr (_DEBUG)
		{
			GTSL::StaticString<128> name("Pipeline layout: "); name += layoutName.GetString();
			pipelineLayout.Name = name;
		}

		PipelineLayout::PushConstant pushConstant;
		pushConstant.ShaderStages = ShaderStage::ALL;
		pushConstant.Offset = 0;
		pushConstant.Size = 128;

		pipelineLayout.PushConstant = &pushConstant;

		pipelineLayout.BindingsSetLayouts = bindingsSetLayouts;
		setLayoutData.PipelineLayout.Initialize(pipelineLayout);
	}
}

SetHandle MaterialSystem::AddSet(RenderSystem* renderSystem, Id setName, Id setLayoutName, const GTSL::Range<SubSetInfo*> setInfo)
{
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

	for(auto& ss : setInfo)
	{
		switch(ss.Type)
		{
		case SubSetType::BUFFER:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_BUFFER, ShaderStage::ALL, ss.Count, BindingFlags::PARTIALLY_BOUND });
			break;
		}
			
		case SubSetType::READ_TEXTURES:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::COMBINED_IMAGE_SAMPLER, ShaderStage::ALL, ss.Count, BindingFlags::PARTIALLY_BOUND });
			break;
		}

		case SubSetType::WRITE_TEXTURES:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::STORAGE_IMAGE, ShaderStage::ALL, ss.Count, BindingFlags::PARTIALLY_BOUND });
			break;
		}

		case SubSetType::RENDER_ATTACHMENT:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::INPUT_ATTACHMENT, ShaderStage::ALL, ss.Count, BindingFlags::PARTIALLY_BOUND });
			break;
		}
			
		case SubSetType::ACCELERATION_STRUCTURE:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ BindingType::ACCELERATION_STRUCTURE, ShaderStage::RAY_GEN, ss.Count, 0 });
			break;
		}
		}
	}
	
	auto setHandle = makeSetEx(renderSystem, setName, setLayoutName, bindingDescriptors);

	auto& set = sets[setHandle()];

	uint32 i = 0;
	
	for (auto& ss : setInfo)
	{
		*ss.Handle = SubSetHandle({ setHandle, i });
		++i;
	}

	return setHandle;
}

BufferHandle MaterialSystem::CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members)
{
	uint32 structSize = 0;

	auto bufferIndex = buffers.Emplace(); //this also essentially referes to the binding wince there's only a buffer per binding
	auto& bufferData = buffers[bufferIndex];
	
	auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32
	{
		uint32 offset = 0;

		for (uint8 m = 0; m < levelMembers.ElementCount(); ++m)
		{
			Member member;
			member.Type = levelMembers[m].Type; member.Count = levelMembers[m].Count;

			auto memberDataIndex = bufferData.MemberData.EmplaceBack();

			*levelMembers[m].Handle = MemberHandle(MemberDescription{ bufferIndex, memberDataIndex });

			bufferData.MemberData[memberDataIndex].ByteOffsetIntoStruct = offset;
			bufferData.MemberData[memberDataIndex].Level = level;
			bufferData.MemberData[memberDataIndex].Type = levelMembers[m].Type;
			bufferData.MemberData[memberDataIndex].Count = levelMembers[m].Count;

			if (levelMembers[m].Type == Member::DataType::STRUCT) { bufferData.MemberData[memberDataIndex].Size = self(self, levelMembers[m].MemberInfos, level + 1); }
			else
			{
				bufferData.MemberData[memberDataIndex].Size = dataTypeSize(levelMembers[m].Type);
				auto size = dataTypeSize(levelMembers[m].Type) * levelMembers[m].Count;
				offset += size;
				structSize += size;
			}
		}

		return offset;
	};

	parseMembers(parseMembers, members, 0);
	
	{
		Buffer::CreateInfo createInfo;
		createInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Buffer");
			createInfo.Name = name;
		}

		createInfo.Size = structSize;
		createInfo.BufferType = BufferType::ADDRESS; createInfo.BufferType |= BufferType::STORAGE;

		for (uint8 f = 0; f < queuedFrames; ++f) {
			RenderSystem::BufferScratchMemoryAllocationInfo allocationInfo;
			allocationInfo.CreateInfo = &createInfo;
			allocationInfo.Allocation = &bufferData.RenderAllocations[f];
			allocationInfo.Buffer = &bufferData.Buffers[f];
			renderSystem->AllocateScratchBufferMemory(allocationInfo);
		}
	}

	return BufferHandle(bufferIndex);
}

MaterialSystem::TextureHandle MaterialSystem::CreateTexture(RenderSystem* renderSystem, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, TextureUses textureUses)
{
	//RenderDevice::FindSupportedImageFormat findFormat;
	//findFormat.TextureTiling = TextureTiling::OPTIMAL;
	//findFormat.TextureUses = TextureUses::TRANSFER_DESTINATION | TextureUses::SAMPLE;
	//GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	//findFormat.Candidates = candidates;
	//auto supportedFormat = renderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

	//GAL::Texture::ConvertTextureFormat(textureInfo.Format, GAL::TextureFormat::RGBA_I8, textureInfo.Extent, GTSL::AlignedPointer<byte, 16>(buffer.begin()), 1);

	TextureComponent textureComponent;

	textureComponent.FormatDescriptor = formatDescriptor;
	auto format = static_cast<TextureFormat>(GAL::FormatToVkFomat(GAL::MakeFormatFromFormatDescriptor(formatDescriptor)));

	auto textureDimensions = GAL::VulkanDimensionsFromExtent(extent);
	
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture.");
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = textureUses | TextureUse::SAMPLE;
		textureCreateInfo.Dimensions = textureDimensions;
		textureCreateInfo.Format = format;
		textureCreateInfo.Extent = extent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;

		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.CreateInfo = &textureCreateInfo;
		allocationInfo.Texture = &textureComponent.Texture;
		renderSystem->AllocateLocalTextureMemory(allocationInfo);
	}

	{
		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture view.");
			textureViewCreateInfo.Name = name;
		}

		textureViewCreateInfo.Type = TextureAspectToVkImageAspectFlags(formatDescriptor.Type);
		textureViewCreateInfo.Dimensions = textureDimensions;
		textureViewCreateInfo.Format = format;
		textureViewCreateInfo.Texture = textureComponent.Texture;
		textureViewCreateInfo.MipLevels = 1;

		textureComponent.TextureView = TextureView(textureViewCreateInfo);
	}

	{
		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture sampler.");
			textureSamplerCreateInfo.Name = name;
		}

		textureSamplerCreateInfo.Anisotropy = 0;

		textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	}

	auto textureIndex = textures.Emplace(textureComponent);

	BindingsSet::TextureBindingUpdateInfo textureBindingUpdateInfo;

	textureBindingUpdateInfo.TextureView = textureComponent.TextureView;
	textureBindingUpdateInfo.Sampler = textureComponent.TextureSampler;
	textureBindingUpdateInfo.TextureLayout = TextureLayout::SHADER_READ_ONLY;

	//for (uint8 f = 0; f < queuedFrames; ++f)
	//{
	//	descriptorsUpdates[f].AddTextureUpdate(textureSubsetsHandle, textureIndex, BindingType::COMBINED_IMAGE_SAMPLER, textureBindingUpdateInfo);
	//}

	//latestLoadedTextures.EmplaceBack(textureIndex);

	return TextureHandle(textureIndex);
}

void MaterialSystem::RecreateTexture(const TextureHandle textureHandle, RenderSystem* renderSystem,	GTSL::Extent3D newExtent)
{
	auto& textureComponent = textures[textureHandle()];

	auto format = static_cast<TextureFormat>(GAL::FormatToVkFomat(GAL::MakeFormatFromFormatDescriptor(textureComponent.FormatDescriptor)));
	
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture.");
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = TextureUse::SAMPLE | TextureUse::TRANSFER_DESTINATION;
		textureCreateInfo.Dimensions = Dimensions::SQUARE;
		textureCreateInfo.Format = format;
		textureCreateInfo.Extent = newExtent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;

		if(textureComponent.Allocation.Size)
		{
			renderSystem->DeallocateLocalTextureMemory(textureComponent.Allocation);
		}
		
		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.CreateInfo = &textureCreateInfo;
		allocationInfo.Texture = &textureComponent.Texture;
		renderSystem->AllocateLocalTextureMemory(allocationInfo);
	}

	{
		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture view.");
			textureViewCreateInfo.Name = name;
		}

		textureViewCreateInfo.Type = GAL::TextureAspectToVkImageAspectFlags(textureComponent.FormatDescriptor.Type);
		textureViewCreateInfo.Dimensions = Dimensions::SQUARE;
		textureViewCreateInfo.Format = format;
		textureViewCreateInfo.Texture = textureComponent.Texture;
		textureViewCreateInfo.MipLevels = 1;

		textureComponent.TextureView = TextureView(textureViewCreateInfo);
	}

	{
		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture sampler.");
			textureSamplerCreateInfo.Name = name;
		}

		textureSamplerCreateInfo.Anisotropy = 0;

		textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	}

	BindingsSet::TextureBindingUpdateInfo textureBindingUpdateInfo;

	textureBindingUpdateInfo.TextureView = textureComponent.TextureView;
	textureBindingUpdateInfo.Sampler = textureComponent.TextureSampler;
	textureBindingUpdateInfo.TextureLayout = TextureLayout::SHADER_READ_ONLY;

	//for (uint8 f = 0; f < queuedFrames; ++f)
	//{
	//	descriptorsUpdates[f].AddTextureUpdate(textureSubsetsHandle, textureHandle(), BindingType::COMBINED_IMAGE_SAMPLER, textureBindingUpdateInfo);
	//}
}

void MaterialSystem::UpdateObjectCount(RenderSystem* renderSystem, MemberHandle memberHandle, uint32 count)
{
	auto& bufferData = buffers[memberHandle().BufferIndex];

	if (bufferData.MemberData.GetLength())
	{
		if (count > bufferData.MemberData[0].Count)
		{
			BE_ASSERT(false, "OOOO");
			//resizeSet(renderSystem, setHandle); // Resize now

			//queuedSetUpdates.EmplaceBack(setHandle); //Defer resizing
		}
	}
}

MaterialInstanceHandle MaterialSystem::CreateMaterial(const CreateMaterialInfo& info)
{
	uint32 material_size = 0;
	info.MaterialResourceManager->GetMaterialSize(info.MaterialName, material_size);

	auto materialIndex = materials.Emplace();
	
	GTSL::Buffer<BE::PAR> material_buffer; material_buffer.Allocate(material_size, 32, GetPersistentAllocator());
	
	const auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "MaterialSystem", AccessTypes::READ_WRITE } };
	MaterialResourceManager::MaterialLoadInfo material_load_info;
	material_load_info.ActsOn = acts_on;
	material_load_info.GameInstance = info.GameInstance;
	material_load_info.Name = info.MaterialName;
	material_load_info.DataBuffer = GTSL::Range<byte*>(material_buffer.GetCapacity(), material_buffer.GetData());
	auto* matLoadInfo = GTSL::New<MaterialLoadInfo>(GetPersistentAllocator(), info.RenderSystem, MoveRef(material_buffer), materialIndex, 0, info.TextureResourceManager);
	material_load_info.UserData = DYNAMIC_TYPE(MaterialLoadInfo, matLoadInfo);
	material_load_info.OnMaterialLoad = GTSL::Delegate<void(TaskInfo, MaterialResourceManager::OnMaterialLoadInfo)>::Create<MaterialSystem, &MaterialSystem::onMaterialLoaded>(this);
	info.MaterialResourceManager->LoadMaterial(material_load_info);
	
	return info.MaterialName;
}

MaterialInstanceHandle MaterialSystem::CreateRayTracingMaterial(const CreateMaterialInfo& info)
{
	RayTracingPipeline rayTracingPipeline;

	return MaterialInstanceHandle();
}

void MaterialSystem::SetDynamicMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(setsBufferData[mat.Set()].Allocations[frame].Data) + mat.DataSize * material.MaterialInstance;
	//
	////TODO: DEFER WRITING TO NOT OVERWRITE RUNNING FRAME
	//byte* FILL = matData + mat.DynamicParameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::RoundUpByPowerOf2(mat.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

void MaterialSystem::SetMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data)
{
	//auto& mat = materials[material.MaterialInstance];
	//
	//auto* matData = static_cast<byte*>(mat.Allocation.Data) + mat.TextureParametersBindings.DataSize * material.MaterialInstance;
	//
	//byte* FILL = matData + mat.Parameters.At(parameterName);
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
	//FILL += GTSL::Math::RoundUpByPowerOf2(mat.TextureParametersBindings.DataSize, static_cast<uint32>(minUniformBufferOffset));
	//GTSL::MemCopy(ShaderDataTypesSize(type), data, FILL);
}

MaterialSystem::TextureHandle MaterialSystem::createTexture(const CreateTextureInfo& info)
{
	auto component = textures.Emplace();

	pendingMaterialsPerTexture.EmplaceAt(component, GetPersistentAllocator());
	pendingMaterialsPerTexture[component].Initialize(4, GetPersistentAllocator());

	texturesRefTable.Emplace(info.TextureName(), component);

	auto textureLoadInfo = TextureLoadInfo(component, Buffer(), info.RenderSystem, RenderAllocation());

	info.TextureResourceManager->LoadTextureInfo(info.GameInstance, info.TextureName, onTextureInfoLoadHandle, GTSL::MoveRef(textureLoadInfo));

	return TextureHandle(component);
}

void MaterialSystem::TraceRays(GTSL::Extent2D rayGrid, CommandBuffer* commandBuffer, RenderSystem* renderSystem)
{
	CommandBuffer::BindPipelineInfo bindPipelineInfo;
	bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
	bindPipelineInfo.PipelineType = PipelineType::RAY_TRACING;
	bindPipelineInfo.Pipeline = rayTracingPipeline;
	commandBuffer->BindPipeline(bindPipelineInfo);

	auto handleSize = renderSystem->GetShaderGroupHandleSize();
	auto alignedHandleSize = GTSL::Math::RoundUpByPowerOf2(handleSize, renderSystem->GetShaderGroupBaseAlignment());

	auto bufferAddress = shaderBindingTableBuffer.GetAddress(renderSystem->GetRenderDevice());
	
	uint32 offset = 0;
	
	CommandBuffer::TraceRaysInfo traceRaysInfo;
	traceRaysInfo.RenderDevice = renderSystem->GetRenderDevice();
	traceRaysInfo.DispatchSize = GTSL::Extent3D(rayGrid);

	for(uint8 i = 0; i < 4; ++i)
	{
		traceRaysInfo.ShaderTableDescriptors[i].Size = shaderCounts[i] * alignedHandleSize;
		traceRaysInfo.ShaderTableDescriptors[i].Address = bufferAddress + offset;
		traceRaysInfo.ShaderTableDescriptors[i].Stride = alignedHandleSize;
		offset += traceRaysInfo.ShaderTableDescriptors[i].Size;
	}

	commandBuffer->TraceRays(traceRaysInfo);
}

void MaterialSystem::Dispatch(GTSL::Extent2D workGroups, CommandBuffer* commandBuffer, RenderSystem* renderSystem)
{
	CommandBuffer::BindPipelineInfo bindPipelineInfo;
	bindPipelineInfo.RenderDevice = renderSystem->GetRenderDevice();
	bindPipelineInfo.PipelineType = PipelineType::COMPUTE;
	bindPipelineInfo.Pipeline = Pipeline();
	commandBuffer->BindPipeline(bindPipelineInfo);

	CommandBuffer::DispatchInfo dispatchInfo; dispatchInfo.RenderDevice = renderSystem->GetRenderDevice();
	dispatchInfo.WorkGroups = workGroups;
	commandBuffer->Dispatch(dispatchInfo);
}

uint32 MaterialSystem::CreateComputePipeline(Id shaderName, MaterialResourceManager* materialResourceManager, GameInstance* gameInstance)
{
	ShaderLoadInfo shaderLoadInfo; shaderLoadInfo.Component = 0;
	
	GTSL::Array<Id, 8> shaderNames; shaderNames.EmplaceBack(shaderName);
	materialResourceManager->LoadShaderInfos(gameInstance, shaderNames, onShaderInfosLoadHandle, GTSL::MoveRef(shaderLoadInfo));

	return 0;
}

void MaterialSystem::updateDescriptors(TaskInfo taskInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

	for (uint32 p = 0; p < queuedSetUpdates.GetReference().GetPageCount(); ++p)
	{
		for (uint32 i = 0; i < queuedSetUpdates.GetReference().GetPage(p).ElementCount(); ++i)
		{
			resizeSet(renderSystem, queuedSetUpdates.GetReference().GetPage(p)[i]);
		}
	}

	queuedSetUpdates.Clear();

	for (auto e : latestLoadedTextures) {
		for (auto b : pendingMaterialsPerTexture[e]) {
			auto& material = materials[b.MaterialIndex];
			auto& materialInstance = materialInstances[b.MaterialInstance];
			if (++materialInstance.Counter == materialInstance.Target) {
				setMaterialAsLoaded(b);
				taskInfo.GameInstance->DispatchEvent("MaterialSystem", GetOnMaterialInstanceLoadEventHandle(), GTSL::MoveRef(material.Name), GTSL::MoveRef(materialInstance.Name));
			}
		}
	}

	latestLoadedTextures.ResizeDown(0);

	auto addedMeshes = renderSystem->GetAddedMeshes();

	//BufferIterator bufferIterator;
	//UpdateIteratorMember(bufferIterator, instanceMaterialReferenceHandle);
	//
	//for (auto e : addedMeshes)
	//{
	//	UpdateIteratorMemberIndex(bufferIterator, e);
	//
	//	for (uint8 f = 0; f < queuedFrames; ++f)
	//	{
	//		BindingsSet::BufferBindingUpdateInfo bufferBindingUpdate;
	//		bufferBindingUpdate.Buffer = renderSystem->GetMeshVertexBuffer(e);
	//		bufferBindingUpdate.Range = renderSystem->GetMeshVertexBufferSize(e);
	//		bufferBindingUpdate.Offset = renderSystem->GetMeshVertexBufferOffset(e);
	//		descriptorsUpdates[f].AddBufferUpdate(vertexBuffersSubSetHandle, e, BUFFER_BINDING_TYPE, bufferBindingUpdate);
	//
	//		bufferBindingUpdate.Buffer = renderSystem->GetMeshIndexBuffer(e);
	//		bufferBindingUpdate.Range = renderSystem->GetMeshIndexBufferSize(e);
	//		bufferBindingUpdate.Offset = renderSystem->GetMeshIndexBufferOffset(e);
	//		descriptorsUpdates[f].AddBufferUpdate(indexBuffersSubSetHandle, e, BUFFER_BINDING_TYPE, bufferBindingUpdate);
	//
	//		auto materialInstanceIndex = publicMaterialHandleToPrivateMaterialHandle(renderSystem->GetMeshMaterialHandle(e)).MaterialInstance;
	//		*getSetMemberPointer<uint32, Member::DataType::UINT32>(bufferIterator, f) = materialInstanceIndex;
	//	}
	//}

	renderSystem->ClearAddedMeshes();

	BindingsSet::BindingsSetUpdateInfo bindingsUpdateInfo;
	bindingsUpdateInfo.RenderDevice = renderSystem->GetRenderDevice();

	{
		auto& descriptorsUpdate = descriptorsUpdates[frame];

		for (auto& set : descriptorsUpdate.sets)  {
			Vector<BindingsSet::BindingsUpdateInfo, BE::TAR> bindingsUpdateInfos(16/*bindings sets*/, GetTransientAllocator());

			for (auto& subSet : set.GetElements()) {
				for (auto& b : subSet) {
					for (auto& a : b.GetElements()) {
						BindingsSet::BindingsUpdateInfo bindingsUpdateInfo;
						bindingsUpdateInfo.Type = a.First;
						bindingsUpdateInfo.SubsetIndex = b.First;

						for (auto& t : a.Second) {
							bindingsUpdateInfo.BindingIndex = t.First;
							bindingsUpdateInfo.BindingUpdateInfos = t.GetElements();
							bindingsUpdateInfos.EmplaceBack(bindingsUpdateInfo);
						}
					}
				}

				bindingsUpdateInfo.BindingsUpdateInfos = bindingsUpdateInfos;
				sets[set.First].BindingsSet[frame].Update(bindingsUpdateInfo, GetTransientAllocator());
			}
		}
		
		descriptorsUpdate.Reset();
	}
}

void MaterialSystem::updateCounter(TaskInfo taskInfo)
{
	frame = (frame + 1) % queuedFrames;
}
void MaterialSystem::updateSubBindingsCount(SubSetHandle subSetHandle, uint32 newCount)
{
	auto& set = sets[subSetHandle().SetHandle()];
	auto& subSet = set.SubSets[subSetHandle().Subset];

	RenderSystem* renderSystem;
	
	if (subSet.AllocatedBindings < newCount)
	{
		BE_ASSERT(false, "OOOO");
	}
}

void MaterialSystem::onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo)
{
	auto loadInfo = DYNAMIC_CAST(MaterialLoadInfo, onMaterialLoadInfo.UserData);

	taskInfo.GameInstance->DispatchEvent("GameInstance", GetOnMaterialLoadEventHandle(), Id(onMaterialLoadInfo.ResourceName));
	
	auto materialIndex = loadInfo->Component;
	auto& material = materials[materialIndex];

	loadedMaterialsMap.Emplace(onMaterialLoadInfo.ResourceName(), materialIndex);
	
	auto* renderSystem = loadInfo->RenderSystem;

	material.MaterialInstances.Initialize(8, GetPersistentAllocator());
	
	material.RenderGroup = onMaterialLoadInfo.RenderGroup;
	material.Parameters = onMaterialLoadInfo.Parameters;
	material.Name = onMaterialLoadInfo.ResourceName;

	//auto setHandle = makeSetEx(renderSystem, onMaterialLoadInfo.ResourceName, onMaterialLoadInfo.RenderGroup, {});
	
	{
		RasterizationPipeline::CreateInfo createInfo;
		createInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Raster pipeline. Material: "); name += onMaterialLoadInfo.ResourceName();
			createInfo.Name = name;
		}

		GTSL::Array<ShaderDataType, 10> vertexDescriptor;
		for (auto e : onMaterialLoadInfo.VertexElements) { vertexDescriptor.EmplaceBack(ConvertShaderDataType(e)); }
		createInfo.VertexDescriptor = vertexDescriptor;
		
		createInfo.PipelineDescriptor.BlendEnable = onMaterialLoadInfo.BlendEnable; createInfo.PipelineDescriptor.CullMode = onMaterialLoadInfo.CullMode;
		createInfo.PipelineDescriptor.DepthTest = onMaterialLoadInfo.DepthTest; createInfo.PipelineDescriptor.DepthWrite = onMaterialLoadInfo.DepthWrite;
		createInfo.PipelineDescriptor.StencilTest = onMaterialLoadInfo.StencilTest; createInfo.PipelineDescriptor.DepthCompareOperation = GAL::CompareOperation::LESS;
		createInfo.PipelineDescriptor.ColorBlendOperation = onMaterialLoadInfo.ColorBlendOperation;

		auto transStencil = [](const MaterialResourceManager::StencilState& stencilState, GAL::StencilState& sS) {
			sS.CompareOperation = stencilState.CompareOperation; sS.CompareMask = stencilState.CompareMask;
			sS.DepthFailOperation = stencilState.DepthFailOperation; sS.FailOperation = stencilState.FailOperation;
			sS.PassOperation = stencilState.PassOperation; sS.Reference = stencilState.Reference; sS.WriteMask = stencilState.WriteMask;
		};
		
		transStencil(onMaterialLoadInfo.Front, createInfo.PipelineDescriptor.StencilOperations.Front);
		transStencil(onMaterialLoadInfo.Back, createInfo.PipelineDescriptor.StencilOperations.Back);

		createInfo.SurfaceExtent = { 1, 1 }; // Will be updated dynamically on render time

		GTSL::Array<Pipeline::ShaderInfo, 8> shaderInfos;

		{
			uint32 offset = 0;
			
			for (uint8 i = 0; i < onMaterialLoadInfo.ShaderSizes.GetLength(); ++i) {
				Pipeline::ShaderInfo shaderInfo;
				shaderInfo.Type = ConvertShaderType(onMaterialLoadInfo.ShaderTypes[i]);
				shaderInfo.Blob = GTSL::Range<const byte*>(onMaterialLoadInfo.ShaderSizes[i], loadInfo->Buffer.GetData() + offset);
				shaderInfos.EmplaceBack(shaderInfo);
				
				offset += onMaterialLoadInfo.ShaderSizes[i];
			}
			
			createInfo.Stages = shaderInfos;
		}
		
		{
			auto* renderOrchestrator = taskInfo.GameInstance->GetSystem<RenderOrchestrator>("RenderOrchestrator");

			createInfo.RenderPass = renderOrchestrator->getAPIRenderPass(onMaterialLoadInfo.RenderPass);
			createInfo.SubPass = renderOrchestrator->getAPISubPassIndex(onMaterialLoadInfo.RenderPass);
			createInfo.AttachmentCount = renderOrchestrator->GetRenderPassColorWriteAttachmentCount(onMaterialLoadInfo.RenderPass);
		}

		createInfo.PipelineLayout = setLayoutDatas[Id("GlobalData")()].PipelineLayout;
		createInfo.PipelineCache = renderSystem->GetPipelineCache();
		material.Pipeline = RasterizationPipeline(createInfo);
	}

	{
		GTSL::Array<MemberInfo, 16> materialParameters;

		for (auto& e : material.Parameters) {
			material.ParametersHandles.Emplace(e.Name());
			
			Member::DataType memberType;

			switch (e.Type)
			{
			case MaterialResourceManager::ParameterType::UINT32: memberType = Member::DataType::UINT32; break;
			case MaterialResourceManager::ParameterType::VEC4: memberType = Member::DataType::FVEC4; break;
			case MaterialResourceManager::ParameterType::TEXTURE_REFERENCE: memberType = Member::DataType::UINT32; break;
			case MaterialResourceManager::ParameterType::BUFFER_REFERENCE: memberType = Member::DataType::UINT64; break;
			}

			materialParameters.EmplaceBack(MemberInfo{ 1, memberType, &material.ParametersHandles.At(e.Name()) });
		}

		GTSL::Array<MemberInfo, 1> materialInstanceParametersStruct;
		materialInstanceParametersStruct.EmplaceBack(MemberInfo{ 16, Member::DataType::STRUCT, &material.MaterialInstancesMemberHandle, materialParameters });
		
		auto bufferHandle = CreateBuffer(renderSystem, materialInstanceParametersStruct);
		BindBufferToName(bufferHandle, Id(onMaterialLoadInfo.ResourceName));
	}

	uint32 i = 0;
	
	for(auto& resourceMaterialInstance : onMaterialLoadInfo.MaterialInstances)
	{
		++material.InstanceCount;
		//UpdateObjectCount(renderSystem, material., material.InstanceCount); //assuming every material uses the same set instance, not index
		material.MaterialInstances.Emplace();

		auto materialInstanceIndex = materialInstances.Emplace();
		materialInstancesMap.Emplace(resourceMaterialInstance.Name(), materialInstanceIndex);
		
		auto instanceMaterialHandle = PrivateMaterialHandle{ loadInfo->Component, materialInstanceIndex };
		
		auto& materialInstance = materialInstances[materialInstanceIndex];
		materialInstance.Material = materialIndex;
		materialInstance.Name = resourceMaterialInstance.Name;

		privateMaterialHandlesByName.Emplace(resourceMaterialInstance.Name(), instanceMaterialHandle);
		
		for(auto& resourceMaterialInstanceParameter : resourceMaterialInstance.Parameters)
		{
			auto materialParameter = material.Parameters.LookFor([&](const MaterialResourceManager::Parameter& parameter) { return parameter.Name == resourceMaterialInstanceParameter.First; }); //get parameter description from name

			BE_ASSERT(materialParameter.State(), "No parameter by that name found. Data must be invalid");
			
			if (material.Parameters[materialParameter.Get()].Type == MaterialResourceManager::ParameterType::TEXTURE_REFERENCE) //if parameter is texture reference, load texture
			{				
				uint32 textureComponentIndex;

				auto textureReference = texturesRefTable.TryGet(resourceMaterialInstanceParameter.Second.TextureReference());
				
				if (!textureReference.State())
				{
					CreateTextureInfo createTextureInfo;
					createTextureInfo.RenderSystem = renderSystem;
					createTextureInfo.GameInstance = taskInfo.GameInstance;
					createTextureInfo.TextureResourceManager = loadInfo->TextureResourceManager;
					createTextureInfo.TextureName = resourceMaterialInstanceParameter.Second.TextureReference;
					createTextureInfo.MaterialHandle = instanceMaterialHandle;
					auto textureComponent = createTexture(createTextureInfo);
					
					addPendingMaterialToTexture(textureComponent, instanceMaterialHandle);

					textureComponentIndex = textureComponent();
				}
				else
				{
					textureComponentIndex = textureReference.Get();
					++materialInstance.Counter; //since we up the target for every texture, up the counter for every already existing texture
				}

				++materialInstance.Target;

				BufferIterator bufferIterator;
				//UpdateIteratorMember(bufferIterator, material.MaterialInstancesMemberHandle.Parameters.At(resourceMaterialInstanceParameter.First));
				UpdateIteratorMember(bufferIterator, material.MaterialInstancesMemberHandle);
				UpdateIteratorMemberIndex(bufferIterator, i);

				UpdateIteratorMember(bufferIterator, material.ParametersHandles.At(resourceMaterialInstanceParameter.First()));
				
				for (uint8 f = 0; f < queuedFrames; ++f) {
					*getSetMemberPointer<uint32, Member::DataType::UINT32>(bufferIterator, f) = textureComponentIndex;
				}
			}
		}

		++i;
	}

	//BE_LOG_WARNING("No ", paramName.GetString(), " parameter found on material ", materialHandle.MaterialType.GetString(), " for ", textureName.GetString(), " found. Skipping update.")
	GTSL::Delete(loadInfo, GetPersistentAllocator());
}

void MaterialSystem::setMaterialAsLoaded(const MaterialSystem::PrivateMaterialHandle privateMaterialHandle)
{
	readyMaterialHandles.EmplaceBack(privateMaterialHandle);

	const auto& material = materials[privateMaterialHandle.MaterialIndex];
	const auto& materialInstance = materialInstances[privateMaterialHandle.MaterialInstance];

	auto materialsPerRenderGroup = readyMaterialsPerRenderGroup.TryGet(material.RenderGroup());
	
	if (!materialsPerRenderGroup.State())
	{
		auto& collection = readyMaterialsPerRenderGroup.Emplace(material.RenderGroup());
		collection.Initialize(8, GetPersistentAllocator());
		collection.EmplaceBack(materialInstance.Name);
	}
	else
	{
		materialsPerRenderGroup.Get().EmplaceBack(materialInstance.Name);
	}
}

SetHandle MaterialSystem::makeSetEx(RenderSystem* renderSystem, Id setName, Id setLayoutName, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDesc)
{
	auto setIndex = sets.Emplace();
	
	auto setHandle = SetHandle(setIndex);
	auto& set = sets[setIndex];
	
	setHandlesByName.Emplace(setName(), setHandle);

	auto& setLayout = setLayoutDatas[setLayoutName()];

	set.Level = setLayout.Level;

	set.BindingsSetLayout = setLayout.BindingsSetLayout;
	set.PipelineLayout = setLayout.PipelineLayout;
	
	//GTSL::Array<BindingsSetLayout, 16> setLayouts(set.Level + 1);
	
	if (bindingDesc.ElementCount())
	{
		//{
		//	auto lastSet = setLayoutName;
		//
		//	for (uint8 i = 0, l = set.Level; i < set.Level + 1; ++i, --l)
		//	{
		//		setLayouts[l] = setLayoutDatas[lastSet()].BindingsSetLayout;
		//		lastSet = setLayoutDatas[lastSet()].Parent;
		//	}
		//}
		
		{
			BindingsPool::CreateInfo bindingsPoolCreateInfo;
			bindingsPoolCreateInfo.RenderDevice = renderSystem->GetRenderDevice();

			if constexpr (_DEBUG)
			{
				GTSL::StaticString<64> name("Bindings pool. Set: "); name += setName.GetString();
				bindingsPoolCreateInfo.Name = name;
			}

			GTSL::Array<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto e : bindingDesc)
			{
				bindingsPoolSizes.PushBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * queuedFrames });
			}

			bindingsPoolCreateInfo.BindingsPoolSizes = bindingsPoolSizes;
			bindingsPoolCreateInfo.MaxSets = MAX_CONCURRENT_FRAMES;
			set.BindingsPool = BindingsPool(bindingsPoolCreateInfo);
		}
		
		{
			BindingsPool::AllocateBindingsSetsInfo allocateBindings;
			allocateBindings.RenderDevice = renderSystem->GetRenderDevice();

			for (uint8 f = 0; f < queuedFrames; ++f)
			{
				GTSL::Array<BindingsSet*, 8> bindingsSets;
				bindingsSets.EmplaceBack(&set.BindingsSet[f]);

				allocateBindings.BindingsSets = bindingsSets;

				{
					allocateBindings.BindingsSetLayouts = GTSL::Range<BindingsSetLayout*>(1, &setLayout.BindingsSetLayout);

					GTSL::Array<GAL::VulkanCreateInfo, 1> bindingsSetsCreateInfo(1);

					if constexpr (_DEBUG)
					{
						GTSL::StaticString<64> name("BindingsSet. Set: "); name += setName.GetString();
						bindingsSetsCreateInfo[0].RenderDevice = renderSystem->GetRenderDevice();
						bindingsSetsCreateInfo[0].Name = name;
					}

					allocateBindings.BindingsSetCreateInfos = bindingsSetsCreateInfo;

					set.BindingsPool.AllocateBindingsSets(allocateBindings);
				}
			}
		}

		for(auto& e : bindingDesc)
		{
			set.SubSets.EmplaceBack(); auto& subSet = set.SubSets.back();
			subSet.AllocatedBindings = e.BindingsCount;
		}
	}
	
	return setHandle;
}

void MaterialSystem::resizeSet(RenderSystem* renderSystem, SetHandle setHandle)
{
	//auto& set = sets[setHandle()];
	//
	////REALLOCATE
	//uint32 newBufferSize = 0;
	//Buffer newBuffer; RenderAllocation newAllocation;
	//
	//for (uint32 i = 0; i < set.StructsSizes.GetLength(); ++i)
	//{
	//	auto newStructSize = set.StructsSizes[i] * set.AllocatedInstances * 2;
	//	newBufferSize += newStructSize;
	//}
	//
	//Buffer::CreateInfo createInfo;
	//createInfo.RenderDevice = renderSystem->GetRenderDevice();
	//createInfo.Name = GTSL::StaticString<64>("undefined");
	//createInfo.Size = newBufferSize;
	//createInfo.BufferType = BufferType::ADDRESS;
	//createInfo.BufferType |= BufferType::STORAGE;
	//
	//RenderSystem::BufferScratchMemoryAllocationInfo allocationInfo;
	//allocationInfo.CreateInfo = &createInfo;
	//allocationInfo.Allocation = &newAllocation;
	//allocationInfo.Buffer = &newBuffer;
	//renderSystem->AllocateScratchBufferMemory(allocationInfo);
	//
	//uint32 oldOffset = 0, newOffset = 0;
	//
	//for (uint32 i = 0; i < set.StructsSizes.GetLength(); ++i)
	//{
	//	auto oldStructSize = set.StructsSizes[i] * set.AllocatedInstances;
	//	auto newStructSize = set.StructsSizes[i] * set.AllocatedInstances * 2;
	//
	//	GTSL::MemCopy(oldStructSize, static_cast<byte*>(set.Allocations[frame].Data) + oldOffset, static_cast<byte*>(newAllocation.Data) + newOffset);
	//
	//	oldOffset += oldStructSize;
	//	newOffset += newStructSize;
	//}
	//
	//renderSystem->DeallocateScratchBufferMemory(set.Allocations[frame]);
	//
	//set.AllocatedInstances *= 2;
	//set.Buffers[frame].Destroy(renderSystem->GetRenderDevice());
	//set.Buffers[frame] = newBuffer;
	//
	//const auto setUpdateHandle = descriptorsUpdates[frame].AddSetToUpdate(setHandle, GetPersistentAllocator());
	//
	//BindingsSet::BufferBindingUpdateInfo bufferBindingUpdate;
	//bufferBindingUpdate.Buffer = set.Buffers[frame];
	//bufferBindingUpdate.Offset = 0;
	//bufferBindingUpdate.Range = newBufferSize;
	//descriptorsUpdates[frame].AddBufferUpdate(setUpdateHandle, 0, 0, BUFFER_BINDING_TYPE, bufferBindingUpdate);
}

void MaterialSystem::onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	Buffer::CreateInfo scratchBufferCreateInfo;
	scratchBufferCreateInfo.RenderDevice = loadInfo.RenderSystem->GetRenderDevice();
	if constexpr (_DEBUG) {
		GTSL::StaticString<64> name("Scratch Buffer. Texture: "); name += textureInfo.Name;
		scratchBufferCreateInfo.Name = name;
	}

	{
		RenderDevice::FindSupportedImageFormat findFormatInfo;
		findFormatInfo.TextureTiling = TextureTiling::OPTIMAL;
		findFormatInfo.TextureUses = TextureUse::TRANSFER_DESTINATION | TextureUse::SAMPLE;
		GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
		findFormatInfo.Candidates = candidates;
		const auto supportedFormat = loadInfo.RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormatInfo);

		scratchBufferCreateInfo.Size = textureInfo.GetTextureSize();
	}

	scratchBufferCreateInfo.BufferType = BufferType::TRANSFER_SOURCE;

	{
		RenderSystem::BufferScratchMemoryAllocationInfo scratchMemoryAllocation;
		scratchMemoryAllocation.Buffer = &loadInfo.Buffer;
		scratchMemoryAllocation.CreateInfo = &scratchBufferCreateInfo;
		scratchMemoryAllocation.Allocation = &loadInfo.RenderAllocation;
		loadInfo.RenderSystem->AllocateScratchBufferMemory(scratchMemoryAllocation);
	}

	auto dataBuffer = GTSL::Range<byte*>(loadInfo.RenderAllocation.Size, static_cast<byte*>(loadInfo.RenderAllocation.Data));

	resourceManager->LoadTexture(taskInfo.GameInstance, textureInfo, dataBuffer, onTextureLoadHandle, GTSL::MoveRef(loadInfo));
}

void MaterialSystem::onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo)
{
	RenderDevice::FindSupportedImageFormat findFormat;
	findFormat.TextureTiling = TextureTiling::OPTIMAL;
	findFormat.TextureUses = TextureUse::TRANSFER_DESTINATION | TextureUse::SAMPLE;
	GTSL::Array<TextureFormat, 16> candidates; candidates.EmplaceBack(ConvertFormat(textureInfo.Format)); candidates.EmplaceBack(TextureFormat::RGBA_I8);
	findFormat.Candidates = candidates;
	auto supportedFormat = loadInfo.RenderSystem->GetRenderDevice()->FindNearestSupportedImageFormat(findFormat);

	//GAL::Texture::ConvertTextureFormat(textureInfo.Format, GAL::TextureFormat::RGBA_I8, textureInfo.Extent, GTSL::AlignedPointer<byte, 16>(buffer.begin()), 1);

	TextureComponent textureComponent;

	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = loadInfo.RenderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture. Texture: "); name += textureInfo.Name;
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = TextureUse::TRANSFER_DESTINATION | TextureUse::SAMPLE;
		textureCreateInfo.Dimensions = ConvertDimension(textureInfo.Dimensions);
		textureCreateInfo.Format = static_cast<GAL::VulkanTextureFormat>(supportedFormat);
		textureCreateInfo.Extent = textureInfo.Extent;
		textureCreateInfo.InitialLayout = TextureLayout::UNDEFINED;
		textureCreateInfo.MipLevels = 1;

		RenderSystem::AllocateLocalTextureMemoryInfo allocationInfo;
		allocationInfo.Allocation = &textureComponent.Allocation;
		allocationInfo.CreateInfo = &textureCreateInfo;
		allocationInfo.Texture = &textureComponent.Texture;

		loadInfo.RenderSystem->AllocateLocalTextureMemory(allocationInfo);
	}

	{
		TextureView::CreateInfo textureViewCreateInfo;
		textureViewCreateInfo.RenderDevice = loadInfo.RenderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture view. Texture: "); name += textureInfo.Name;
			textureViewCreateInfo.Name = name;
		}

		textureViewCreateInfo.Type = TextureType::COLOR;
		textureViewCreateInfo.Dimensions = ConvertDimension(textureInfo.Dimensions);
		textureViewCreateInfo.Format = supportedFormat;
		textureViewCreateInfo.Texture = textureComponent.Texture;
		textureViewCreateInfo.MipLevels = 1;

		textureComponent.TextureView = TextureView(textureViewCreateInfo);
	}

	{
		RenderSystem::TextureCopyData textureCopyData;
		textureCopyData.DestinationTexture = textureComponent.Texture;
		textureCopyData.SourceBuffer = loadInfo.Buffer;
		textureCopyData.Allocation = loadInfo.RenderAllocation;
		textureCopyData.Layout = TextureLayout::TRANSFER_DST;
		textureCopyData.Extent = textureInfo.Extent;

		loadInfo.RenderSystem->AddTextureCopy(textureCopyData);
	}

	{
		TextureSampler::CreateInfo textureSamplerCreateInfo;
		textureSamplerCreateInfo.RenderDevice = loadInfo.RenderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture sampler. Texture: "); name += textureInfo.Name;
			textureSamplerCreateInfo.Name = name;
		}

		textureSamplerCreateInfo.Anisotropy = 0;

		textureComponent.TextureSampler = TextureSampler(textureSamplerCreateInfo);
	}

	textures[loadInfo.Component] = textureComponent;

	BE_LOG_MESSAGE("Loaded texture ", textureInfo.Name);

	BindingsSet::TextureBindingUpdateInfo textureBindingUpdateInfo;

	textureBindingUpdateInfo.TextureView = textureComponent.TextureView;
	textureBindingUpdateInfo.Sampler = textureComponent.TextureSampler;
	textureBindingUpdateInfo.TextureLayout = TextureLayout::SHADER_READ_ONLY;

	for (uint8 f = 0; f < queuedFrames; ++f)
	{
		descriptorsUpdates[f].AddTextureUpdate(textureSubsetsHandle, loadInfo.Component, BindingType::COMBINED_IMAGE_SAMPLER, textureBindingUpdateInfo);
	}

	latestLoadedTextures.EmplaceBack(loadInfo.Component);
}

void MaterialSystem::onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager* materialResourceManager, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo)
{	
	uint32 totalSize = 0;

	for (auto e : shaderInfos) { totalSize += e.Size; }
	
	shaderLoadInfo.Buffer.Allocate(totalSize, 8, GetPersistentAllocator());
	
	materialResourceManager->LoadShaders(taskInfo.GameInstance, shaderInfos, onShadersLoad, shaderLoadInfo.Buffer.GetRange(), GTSL::MoveRef(shaderLoadInfo));
}

void MaterialSystem::onShadersLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo)
{
	auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	
	shaderLoadInfo.Component;

	ComputePipeline pipeline;
	ComputePipeline::CreateInfo createInfo;
	createInfo.RenderDevice = renderSystem->GetRenderDevice();
	createInfo.PipelineLayout;
	createInfo.ShaderInfo.Blob = GTSL::Range<const byte*>(shaders[0].Size, shaderLoadInfo.Buffer.GetData());
	createInfo.ShaderInfo.Type = ShaderType::COMPUTE;
	pipeline.Initialize(createInfo);
}
