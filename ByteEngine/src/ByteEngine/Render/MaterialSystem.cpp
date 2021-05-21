#include "MaterialSystem.h"

#include "RenderSystem.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include <GTSL/SIMD/SIMD.hpp>
#include <GAL/Texture.h>

#include "RenderOrchestrator.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Application/Application.h"

const char* BindingTypeString(const GAL::BindingType binding)
{
	switch (binding)
	{
	case GAL::BindingType::UNIFORM_BUFFER_DYNAMIC: return "UNIFORM_BUFFER_DYNAMIC";
	case GAL::BindingType::COMBINED_IMAGE_SAMPLER: return "COMBINED_IMAGE_SAMPLER";
	case GAL::BindingType::UNIFORM_BUFFER: return "UNIFORM_BUFFER";
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

void MaterialSystem::BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle setHandle, GAL::ShaderStage shaderStage)
{
	if constexpr (_DEBUG)
	{
		//if(!setHandlesByName.Find(setHandle())) { BE_LOG_ERROR("Tried to bind set which doesn't exist at render time!. ", BE::FIX_OR_CRASH_STRING) }
	}

	auto& set = sets[setHandle()];

	if (set.BindingsSet[frame].GetHandle()) {
		//FUTURE: if we ever support buffer dynamic offset intead of indexing remember to implement switch for descriptors and here to supply offsets
	
		commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), shaderStage, GTSL::Range<BindingsSet*>(1, &set.BindingsSet[frame]),
			GTSL::Range<const uint32*>(), set.PipelineLayout, set.Level);
	}
}

void MaterialSystem::WriteSetTexture(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex)
{
	GAL::TextureLayout layout; GAL::BindingType bindingType;
	if (setHandle().Type == GAL::BindingType::STORAGE_IMAGE)
	{
		layout = GAL::TextureLayout::GENERAL;
		bindingType = GAL::BindingType::STORAGE_IMAGE;
	}
	else
	{
		layout = GAL::TextureLayout::SHADER_READ;
		bindingType = GAL::BindingType::COMBINED_IMAGE_SAMPLER;
	}

	for (uint8 f = 0; f < queuedFrames; ++f)
	{
		BindingsSet::TextureBindingUpdateInfo info;
		info.TextureView = renderSystem->GetTextureView(textureHandle);
		info.Sampler = renderSystem->GetTextureSampler(textureHandle);
		info.TextureLayout = layout;

		descriptorsUpdates[f].AddTextureUpdate(setHandle, bindingIndex, info);
	}
}

void MaterialSystem::AddSetLayout(RenderSystem* renderSystem, Id layoutName, Id parentName, const GTSL::Range<SubSetDescriptor*> members)
{
	Id parentHandle;
	uint32 level;

	if (parentName()) {
		auto& parentSetLayout = setLayoutDatas[parentName];
		
		parentHandle = parentName;
		level = parentSetLayout.Level + 1;
	}
	else {
		parentHandle = Id();
		level = 0;
	}

	auto& setLayoutData = setLayoutDatas.Emplace(layoutName);
	
	setLayoutData.Parent = parentHandle;
	setLayoutData.Level = level;

	GTSL::Array<BindingsSetLayout, 16> bindingsSetLayouts;

	// Traverse tree to find parent's pipeline layouts
	{
		auto lastSet = parentHandle;
	
		for (uint8 i = 0; i < level; ++i) { bindingsSetLayouts.EmplaceBack(); }
		
		for (uint8 i = 0, l = level - 1; i < level; ++i, --l) {
			bindingsSetLayouts[l] = setLayoutDatas[lastSet].BindingsSetLayout;
			lastSet = setLayoutDatas[lastSet].Parent;
		}
	}

	setLayoutData.Stage = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::COMPUTE | GAL::ShaderStages::RAY_GEN;
	
	{
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (auto e : members) {
			GAL::ShaderStage shaderStage = setLayoutData.Stage;
			GAL::BindingFlag bindingFlags = 0;
			
			GAL::BindingType bindingType = {};

			if (e.BindingsCount != 1) { bindingFlags = GAL::BindingFlags::PARTIALLY_BOUND; }
			
			switch (e.SubSetType)
			{
			case SubSetType::BUFFER: bindingType = GAL::BindingType::STORAGE_BUFFER; break;
			case SubSetType::READ_TEXTURES: bindingType = GAL::BindingType::COMBINED_IMAGE_SAMPLER; break;
			case SubSetType::WRITE_TEXTURES: bindingType = GAL::BindingType::STORAGE_IMAGE; break;
			case SubSetType::RENDER_ATTACHMENT: bindingType = GAL::BindingType::INPUT_ATTACHMENT; break;
			case SubSetType::ACCELERATION_STRUCTURE: 
				bindingType = GAL::BindingType::ACCELERATION_STRUCTURE;
				shaderStage = GAL::ShaderStages::RAY_GEN;
				setLayoutData.Stage |= shaderStage;
				break;
			}

			subSetDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ bindingType, shaderStage, e.BindingsCount, bindingFlags });
		}
		
		//GTSL::StaticString<64> bindingsSetLayoutName("Set layout: "); bindingsSetLayoutName += layoutName.GetString();
		
		setLayoutData.BindingsSetLayout.Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);
		bindingsSetLayouts.EmplaceBack().Initialize(renderSystem->GetRenderDevice(), subSetDescriptors);
	}

	{
		if constexpr (_DEBUG) {
			GTSL::StaticString<128> name("Pipeline layout: "); name += layoutName.GetString();
			//pipelineLayout.Name = name;
		}

		GAL::PushConstant pushConstant;
		pushConstant.Stage = setLayoutData.Stage;
		pushConstant.NumberOf4ByteSlots = 32;
		setLayoutData.PipelineLayout.Initialize(renderSystem->GetRenderDevice(), &pushConstant, bindingsSetLayouts);
	}
}

SetHandle MaterialSystem::AddSet(RenderSystem* renderSystem, Id setName, Id setLayoutName, const GTSL::Range<SubSetInfo*> setInfo)
{
	GTSL::Array<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

	for(auto& ss : setInfo)
	{
		GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::COMPUTE;	
		
		switch(ss.Type)
		{
		case SubSetType::BUFFER:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
			break;
		}
			
		case SubSetType::READ_TEXTURES:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{GAL::BindingType::COMBINED_IMAGE_SAMPLER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
			break;
		}

		case SubSetType::WRITE_TEXTURES:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
			break;
		}

		case SubSetType::RENDER_ATTACHMENT:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
			break;
		}
			
		case SubSetType::ACCELERATION_STRUCTURE:
		{
			bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.Count, 0 });
			break;
		}
		}
	}
	
	auto setHandle = makeSetEx(renderSystem, setName, setLayoutName, bindingDescriptors);

	auto& set = sets[setHandle()];

	uint32 i = 0;
	
	for (auto& ss : setInfo) {
		*ss.Handle = SubSetHandle({ setHandle, i, bindingDescriptors[i].BindingType });
		++i;
	}

	return setHandle;
}

BufferHandle MaterialSystem::CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members)
{
	GAL::BufferUse bufferUses, notBufferFlags;

	auto bufferIndex = buffers.Emplace(); //this also essentially referes to the binding wince there's only a buffer per binding
	auto& bufferData = buffers[bufferIndex];
	
	auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
		uint32 offset = 0;

		for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
			if(levelMembers[m].Type == Member::DataType::PAD) { offset += levelMembers[m].Count; }
			else {
				auto memberDataIndex = bufferData.MemberData.GetLength();
				auto& member = bufferData.MemberData.EmplaceBack();

				member.ByteOffsetIntoStruct = offset;
				member.Level = level;
				member.Type = levelMembers[m].Type;
				member.Count = levelMembers[m].Count;
				
				*reinterpret_cast<MemberHandle<byte>*>(levelMembers[m].Handle) = MemberHandle<byte>(bufferIndex, memberDataIndex);

				if (levelMembers[m].Type == Member::DataType::STRUCT) {
					member.Size = self(self, levelMembers[m].MemberInfos, level + 1);
				} else {
					if (levelMembers[m].Type == Member::DataType::SHADER_HANDLE) {
						bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
						notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
					}
					
					member.Size = dataTypeSize(levelMembers[m].Type);
				}

				offset += member.Size * member.Count;
			}
		}

		return offset;
	};

	uint32 bufferSize = parseMembers(parseMembers, members, 0);

	if(bufferSize != 0) {
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Buffer");
			//createInfo.Name = name;
		}

		bufferUses |= GAL::BufferUses::ADDRESS; bufferUses |= GAL::BufferUses::STORAGE;

		for (uint8 f = 0; f < queuedFrames; ++f) {
			renderSystem->AllocateScratchBufferMemory(bufferSize, bufferUses & ~notBufferFlags, &bufferData.Buffers[f], &bufferData.RenderAllocations[f]);
		}
	}

	return BufferHandle(bufferIndex);
}

void MaterialSystem::Dispatch(GTSL::Extent2D workGroups, CommandBuffer* commandBuffer, RenderSystem* renderSystem)
{
	commandBuffer->BindPipeline(renderSystem->GetRenderDevice(), Pipeline(), GAL::ShaderStages::COMPUTE);
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

				sets[set.First].BindingsSet[frame].Update(renderSystem->GetRenderDevice(), bindingsUpdateInfos, GetTransientAllocator());
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
	
	setHandlesByName.Emplace(setName, setHandle);

	auto& setLayout = setLayoutDatas[setLayoutName];

	set.Level = setLayout.Level;

	set.BindingsSetLayout = setLayout.BindingsSetLayout;
	set.PipelineLayout = setLayout.PipelineLayout;
	
	//GTSL::Array<BindingsSetLayout, 16> setLayouts(set.Level + 1);
	
	if (bindingDesc.ElementCount()) {		
		
		if constexpr (_DEBUG) {
			GTSL::StaticString<64> name("Bindings pool. Set: "); name += setName.GetString();
			//bindingsPoolCreateInfo.Name = name;
		}

		GTSL::Array<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

		for (auto e : bindingDesc) {
			bindingsPoolSizes.PushBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * queuedFrames });
		}

		set.BindingsPool.Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, MAX_CONCURRENT_FRAMES);
		
		for (uint8 f = 0; f < queuedFrames; ++f) {

			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name("BindingsSet. Set: "); name += setName.GetString();
				//auto& bindingsSetCreateInfo = bindingsSetsCreateInfos.EmplaceBack();
				//bindingsSetCreateInfo.RenderDevice = renderSystem->GetRenderDevice();
				//bindingsSetCreateInfo.Name = name;
			}
			
			set.BindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.BindingsPool, setLayout.BindingsSetLayout);
		}

		for(auto& e : bindingDesc) {
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