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

	//initializeInfo.GameInstance->AddEvent("MaterialSystem", GetOnMaterialLoadEventHandle());
	//initializeInfo.GameInstance->AddEvent("MaterialSystem", GetOnMaterialInstanceLoadEventHandle());
	
	queuedFrames = BE::Application::Get()->GetOption("buffer");
	queuedFrames = GTSL::Math::Clamp(queuedFrames, (uint8)2, (uint8)3);

	buffers.Initialize(64, GetPersistentAllocator()); buffersByName.Initialize(32, GetPersistentAllocator());

	queuedSetUpdates.Initialize(1, 2, GetPersistentAllocator());
	
	setHandlesByName.Initialize(16, GetPersistentAllocator());
	setLayoutDatas.Initialize(16, GetPersistentAllocator());

	textures.Initialize(32, GetPersistentAllocator());
	
	sets.Initialize(16, GetPersistentAllocator());
	
	for (uint32 i = 0; i < queuedFrames; ++i)
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
	
		commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), pipelineType, GTSL::Range<BindingsSet*>(1, &set.BindingsSet[frame]),
			GTSL::Range<const uint32*>(), set.PipelineLayout, set.Level);
	}
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
		*ss.Handle = SubSetHandle({ setHandle, i, bindingDescriptors[i].BindingType });
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

	textureComponent.Uses = textureUses | TextureUse::SAMPLE;
	if (formatDescriptor.Type == GAL::TextureType::COLOR) { textureComponent.Uses |= TextureUse::STORAGE; }
	
	{
		Texture::CreateInfo textureCreateInfo;
		textureCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Texture.");
			textureCreateInfo.Name = name;
		}

		textureCreateInfo.Tiling = TextureTiling::OPTIMAL;
		textureCreateInfo.Uses = textureComponent.Uses;
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
		textureCreateInfo.Uses = textureComponent.Uses;
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

void MaterialSystem::Dispatch(GTSL::Extent2D workGroups, CommandBuffer* commandBuffer, RenderSystem* renderSystem)
{
	commandBuffer->BindPipeline(renderSystem->GetRenderDevice(), Pipeline(), PipelineType::COMPUTE);
	commandBuffer->Dispatch(renderSystem->GetRenderDevice(), GTSL::Extent3D(workGroups));
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