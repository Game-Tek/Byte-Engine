#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/SparseVector.hpp>
#include <GTSL/PagedVector.h>

#include "RenderSystem.h"
#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include "ByteEngine/Handle.hpp"

class TextureResourceManager;
struct TaskInfo;

MAKE_HANDLE(uint32, Set)

struct SubSetDescription {
	SetHandle SetHandle; uint32 Subset;
	GAL::BindingType Type;
};

MAKE_HANDLE(SubSetDescription, SubSet)

template<typename T>
struct MemberHandle
{	
	uint32 BufferIndex = 0, MemberIndirectionIndex = 0;
};

MAKE_HANDLE(uint32, Buffer)

class MaterialSystem : public System
{	
public:
	MaterialSystem() : System("MaterialSystem")
	{}
	
	struct Member {
		enum class DataType : uint8 {
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, MATRIX3X4, FVEC4, FVEC2, STRUCT, PAD,
			SHADER_HANDLE
		};

		Member() = default;
		Member(const uint32 count, const DataType type) : Count(count), Type(type) {}
		
		uint32 Count = 1;
		DataType Type = DataType::PAD;
	};
	
	void Initialize(const InitializeInfo& initializeInfo) override {
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

		queuedFrames = renderSystem->GetPipelinedFrames();

		buffers.Initialize(64, GetPersistentAllocator());

		queuedSetUpdates.Initialize(1, 2, GetPersistentAllocator());

		setHandlesByName.Initialize(16, GetPersistentAllocator());
		setLayoutDatas.Initialize(16, GetPersistentAllocator());

		sets.Initialize(16, GetPersistentAllocator());

		for (uint32 i = 0; i < queuedFrames; ++i) {
			descriptorsUpdates.EmplaceBack();
			descriptorsUpdates.back().Initialize(GetPersistentAllocator());
		}

		frame = 0;
	}
	
	void Shutdown(const ShutdownInfo& shutdownInfo) override {
		//RenderSystem* renderSystem = shutdownInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	}

	[[nodiscard]] GPUBuffer GetBuffer(BufferHandle bufferHandle) const { return buffers[bufferHandle()].Buffers[frame]; }
	[[nodiscard]] PipelineLayout GetSetLayoutPipelineLayout(Id id) const { return setLayoutDatas[id].PipelineLayout; }
	
	void WriteBinding(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure) {
		for (uint8 f = 0; f < queuedFrames; ++f) {
			descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
		}
	}

	void WriteBinding(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure, uint8 f) {
		descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
	}

	GTSL::uint64 GetBufferAddress(RenderSystem* renderSystem, const BufferHandle bufferHandle) const {
		GTSL::uint64 address = 0;
		if (buffers[bufferHandle()].Buffers[frame].GetVkBuffer()) {
			address = buffers[bufferHandle()].Buffers[frame].GetAddress(renderSystem->GetRenderDevice());
		}
		return address;
	}
	
	void PushConstant(const RenderSystem* renderSystem, CommandBuffer commandBuffer, Id layout, uint32 offset, GTSL::Range<const byte*> range) const {
		const auto& set = setLayoutDatas[layout];
		commandBuffer.UpdatePushConstant(renderSystem->GetRenderDevice(), set.PipelineLayout, offset, range, set.Stage);
	}

	struct BufferIterator {
		BufferIterator()
		{
			Levels.EmplaceBack(0);
		}
		
		GTSL::Array<uint32, 16> Levels;
		uint32 ByteOffset = 0;
	};

	template<typename T>
	T* GetMemberPointer(BufferIterator& iterator, MemberHandle<T> memberHandle, uint32 i = 0) {
		//static_assert(T != (void*), "Type can not be struct.");
		
		auto& bufferData = buffers[memberHandle.BufferIndex];
		auto& member = bufferData.MemberData[memberHandle.MemberIndirectionIndex];
		BE_ASSERT(i < member.Count, "Requested sub set buffer member index greater than allocated instances count.")
		
		//												//BUFFER							//OFFSET TO STRUCT
		return reinterpret_cast<T*>(static_cast<byte*>(bufferData.RenderAllocations[frame].Data) + iterator.ByteOffset + member.ByteOffsetIntoStruct + member.Size * i);
	}

	template<typename T>
	void WriteMultiBuffer(BufferIterator& iterator, MemberHandle<T> memberHandle, T* data, uint32 i = 0) {
		auto& bufferData = buffers[memberHandle.BufferIndex]; auto& member = bufferData.MemberData[memberHandle.MemberIndirectionIndex];
		for(uint8 f = 0; f < queuedFrames; ++f) {
			*reinterpret_cast<T*>(static_cast<byte*>(bufferData.RenderAllocations[f].Data) + iterator.ByteOffset + member.ByteOffsetIntoStruct + member.Size * i) = *data;
		}
	}

	template<>
	void WriteMultiBuffer(BufferIterator& iterator, MemberHandle<GAL::ShaderHandle> memberHandle, GAL::ShaderHandle* data, uint32 i) {
		auto& bufferData = buffers[memberHandle.BufferIndex]; auto& member = bufferData.MemberData[memberHandle.MemberIndirectionIndex];
		for(uint8 f = 0; f < queuedFrames; ++f) {
			for (uint8 t = 0; t < data->AlignedSize / 4; ++t) {
				reinterpret_cast<uint32*>(static_cast<byte*>(bufferData.RenderAllocations[f].Data) + iterator.ByteOffset + member.ByteOffsetIntoStruct)[t] = static_cast<uint32*>(data->Data)[t];
			}
		}
	}
	
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, Id setName, GAL::ShaderStage shaderStage) {
		BindSet(renderSystem, commandBuffer, setHandlesByName.At(setName), shaderStage);
	}
	
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle setHandle, GAL::ShaderStage shaderStage) {
		if (auto& set = sets[setHandle()]; set.BindingsSet[frame].GetHandle()) {
			commandBuffer.BindBindingsSets(renderSystem->GetRenderDevice(), shaderStage, GTSL::Range<BindingsSet*>(1, &set.BindingsSet[frame]),
				GTSL::Range<const uint32*>(), set.PipelineLayout, set.Level);
		}
	}

	SetHandle GetSetHandleByName(const Id name) const { return setHandlesByName.At(name); }

	void WriteBinding(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex) {
		GAL::TextureLayout layout; GAL::BindingType bindingType;

		if (setHandle().Type == GAL::BindingType::STORAGE_IMAGE) {
			layout = GAL::TextureLayout::GENERAL;
			bindingType = GAL::BindingType::STORAGE_IMAGE;
		} else {
			layout = GAL::TextureLayout::SHADER_READ;
			bindingType = GAL::BindingType::COMBINED_IMAGE_SAMPLER;
		}

		for (uint8 f = 0; f < queuedFrames; ++f) {
			BindingsSet::TextureBindingUpdateInfo info;
			info.TextureView = renderSystem->GetTextureView(textureHandle);
			info.Sampler = renderSystem->GetTextureSampler(textureHandle);
			info.TextureLayout = layout;

			descriptorsUpdates[f].AddTextureUpdate(setHandle, bindingIndex, info);
		}
	}

	struct MemberInfo : Member
	{
		MemberInfo() = default;
		MemberInfo(const uint32 count) : Member(count, Member::DataType::PAD) {}
		MemberInfo(MemberHandle<uint32>* memberHandle, const uint32 count) : Member(count, Member::DataType::UINT32), Handle(memberHandle) {}
		MemberInfo(MemberHandle<RenderSystem::BufferAddress>* memberHandle, const uint32 count) : Member(count, Member::DataType::UINT32), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix4>* memberHandle, const uint32 count) : Member(count, Member::DataType::MATRIX4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GTSL::Matrix3x4>* memberHandle, const uint32 count) : Member(count, Member::DataType::MATRIX3X4), Handle(memberHandle) {}
		MemberInfo(MemberHandle<GAL::ShaderHandle>* memberHandle, const uint32 count) : Member(count, Member::DataType::SHADER_HANDLE), Handle(memberHandle) {}
		MemberInfo(MemberHandle<void*>* memberHandle, const uint32 count, GTSL::Range<MemberInfo*> memberInfos) : Member(count, Member::DataType::STRUCT), Handle(memberHandle), MemberInfos(memberInfos) {}
		
		void* Handle = nullptr;
		GTSL::Range<MemberInfo*> MemberInfos;
	};

	enum class SubSetType : uint8 {
		BUFFER, READ_TEXTURES, WRITE_TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE
	};

	struct SubSetDescriptor
	{
		SubSetType SubSetType; uint32 BindingsCount;
	};
	void AddSetLayout(RenderSystem* renderSystem, Id layoutName, Id parentName, const GTSL::Range<SubSetDescriptor*> subsets) {
		Id parentHandle;
		uint32 level;

		if (parentName()) {
			auto& parentSetLayout = setLayoutDatas[parentName];

			parentHandle = parentName;
			level = parentSetLayout.Level + 1;
		} else {
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

		GTSL::Array<BindingsSetLayout::BindingDescriptor, 10> subSetDescriptors;

		for (auto e : subsets) {
			GAL::ShaderStage shaderStage = setLayoutData.Stage;
			GAL::BindingFlag bindingFlags = 0;

			GAL::BindingType bindingType = {};

			if (e.BindingsCount != 1) { bindingFlags = GAL::BindingFlags::PARTIALLY_BOUND; }

			switch (e.SubSetType) {
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

		if constexpr (_DEBUG) {
			GTSL::StaticString<128> name("Pipeline layout: "); name += layoutName.GetString();
			//pipelineLayout.Name = name;
		}

		GAL::PushConstant pushConstant;
		pushConstant.Stage = setLayoutData.Stage;
		pushConstant.NumberOf4ByteSlots = 32;
		setLayoutData.PipelineLayout.Initialize(renderSystem->GetRenderDevice(), &pushConstant, bindingsSetLayouts);
	}
	
	struct SubSetInfo
	{
		SubSetType Type;
		SubSetHandle* Handle;
		uint32 Count;
	};
	
	void AddSetLayout(RenderSystem* renderSystem, Id layoutName, Id parentName, const GTSL::Range<SubSetInfo*> subsets)
	{
		GTSL::Array<SubSetDescriptor, 16> subSetInfos;
		for (auto e : subsets) { subSetInfos.EmplaceBack(e.Type, e.Count); }
		AddSetLayout(renderSystem, layoutName, parentName, subSetInfos);
	}
		
	SetHandle AddSet(RenderSystem* renderSystem, Id setName, Id setLayoutName, const GTSL::Range<SubSetInfo*> setInfo) {
		GTSL::Array<BindingsSetLayout::BindingDescriptor, 16> bindingDescriptors;

		for (auto& ss : setInfo)
		{
			GAL::ShaderStage enabledShaderStages = GAL::ShaderStages::VERTEX | GAL::ShaderStages::FRAGMENT | GAL::ShaderStages::RAY_GEN | GAL::ShaderStages::CLOSEST_HIT | GAL::ShaderStages::ANY_HIT | GAL::ShaderStages::MISS | GAL::ShaderStages::CALLABLE | GAL::ShaderStages::COMPUTE;

			switch (ss.Type)
			{
			case SubSetType::BUFFER:
			{
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_BUFFER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}

			case SubSetType::READ_TEXTURES:
			{
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::COMBINED_IMAGE_SAMPLER, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}

			case SubSetType::WRITE_TEXTURES:
			{
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::STORAGE_IMAGE, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}

			case SubSetType::RENDER_ATTACHMENT:
			{
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::INPUT_ATTACHMENT, enabledShaderStages, ss.Count, GAL::BindingFlags::PARTIALLY_BOUND });
				break;
			}

			case SubSetType::ACCELERATION_STRUCTURE:
			{
				bindingDescriptors.PushBack(BindingsSetLayout::BindingDescriptor{ GAL::BindingType::ACCELERATION_STRUCTURE, enabledShaderStages, ss.Count, 0 });
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

	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members) {
		GAL::BufferUse bufferUses, notBufferFlags;

		auto bufferIndex = buffers.Emplace(); auto& bufferData = buffers[bufferIndex];

		auto parseMembers = [&](auto&& self, GTSL::Range<MemberInfo*> levelMembers, uint16 level) -> uint32 {
			uint32 offset = 0;

			for (uint8 m = 0; m < levelMembers.ElementCount(); ++m) {
				if (levelMembers[m].Type == Member::DataType::PAD) { offset += levelMembers[m].Count; continue; }

				auto memberDataIndex = bufferData.MemberData.GetLength();
				auto& member = bufferData.MemberData.EmplaceBack();

				member.ByteOffsetIntoStruct = offset;
				member.Level = level;
				member.Type = levelMembers[m].Type;
				member.Count = levelMembers[m].Count;

				*static_cast<MemberHandle<byte>*>(levelMembers[m].Handle) = MemberHandle<byte>(bufferIndex, memberDataIndex);

				if (levelMembers[m].Type == Member::DataType::STRUCT) {
					member.Size = self(self, levelMembers[m].MemberInfos, level + 1);
				}
				else {
					if (levelMembers[m].Type == Member::DataType::SHADER_HANDLE) {
						bufferUses |= GAL::BufferUses::SHADER_BINDING_TABLE;
						notBufferFlags |= GAL::BufferUses::ACCELERATION_STRUCTURE; notBufferFlags |= GAL::BufferUses::STORAGE;
					}

					member.Size = dataTypeSize(levelMembers[m].Type);
				}

				offset += member.Size * member.Count;
			}

			return offset;
		};

		uint32 bufferSize = parseMembers(parseMembers, members, 0);

		if (bufferSize != 0) {
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name("Buffer");
				//createInfo.Name = name;
			}

			bufferUses |= GAL::BufferUses::ADDRESS; bufferUses |= GAL::BufferUses::STORAGE;

			for (uint8 f = 0; f < queuedFrames; ++f) {
				renderSystem->AllocateScratchBufferMemory(bufferSize, bufferUses & ~notBufferFlags, &bufferData.Buffers[f], &bufferData.RenderAllocations[f]);
				bufferData.Size[f] = bufferSize;
			}
		}

		return BufferHandle(bufferIndex);
	}
	
	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, MemberInfo member) {
		return CreateBuffer(renderSystem, GTSL::Range<MemberInfo*>(1, &member));
	}
	
	/**
	 * \brief Update the member instance count to be able to fit at least count requested elements.
	 * \param renderSystem Pointer to the active RenderSystem instance.
	 * \param memberHandle Handle to the member whose count is going to be updated.
	 * \param count Number to check against if enough instances are allocated. Doesn't have to be incremental, can be any index as long as it represents the new boundary of the collection(in terms of indeces) or any index inside the range.
	 */
	template<typename T>
	void UpdateObjectCount(RenderSystem* renderSystem, MemberHandle<T> memberHandle, uint32 count)
	{
		auto& bufferData = buffers[memberHandle.BufferIndex];

		if (bufferData.MemberData.GetLength()) {
			if (count > bufferData.MemberData[0].Count) {
				BE_ASSERT(false, "OOOO");
				//resizeSet(renderSystem, setHandle); // Resize now

				//queuedSetUpdates.EmplaceBack(setHandle); //Defer resizing
			}
		}
	}

	struct BindingsSetData
	{
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};
	
	/**
	 * \brief Updates the iterator hierarchy level to index the specified member.
	 * \param iterator BufferIterator object to update.
	 * \param member MemberHandle that refers to the struct that we want the iterator to point to.
	 */
	void UpdateIteratorMember(BufferIterator& iterator, MemberHandle<void*> member, const uint32 index = 0)
	{
		//static_assert(T == (void*), "Type can only be struct!");
		
		auto& bufferData = buffers[member.BufferIndex]; auto& memberData = bufferData.MemberData[member.MemberIndirectionIndex];

		for (uint32 i = iterator.Levels.GetLength(); i < memberData.Level + 1; ++i) {
			iterator.Levels.EmplaceBack(0);
		}

		for (uint32 i = iterator.Levels.GetLength(); i > memberData.Level + 1; --i) {
			iterator.Levels.PopBack();
		}
		
		int32 shiftedElements = index - iterator.Levels.back();
		
		iterator.Levels.back() = index;
		
		iterator.ByteOffset += shiftedElements * memberData.Size;

		bufferData.Written[frame] = true;
	}

	void CopyWrittenBuffers(RenderSystem* renderSystem) {
		for (auto& e : buffers) {
			if(e.Written[frame]) {
			} else {
				auto beforeFrame = uint8(frame - uint8(1)) % renderSystem->GetPipelinedFrames();
				if(e.Written[beforeFrame]) {
					GTSL::MemCopy(e.Size[frame], e.RenderAllocations[beforeFrame].Data, e.RenderAllocations[frame].Data);
				}
			}

			e.Written[frame] = false;
		}
	}

private:
	
	uint32 dataTypeSize(MaterialSystem::Member::DataType data)
	{
		switch (data)
		{
		case Member::DataType::FLOAT32: return 4;
		case Member::DataType::UINT32: return 4;
		case Member::DataType::UINT64: return 8;
		case Member::DataType::MATRIX4: return 4 * 4 * 4;
		case Member::DataType::MATRIX3X4: return 4 * 3 * 4;
		case Member::DataType::FVEC4: return 4 * 4;
		case Member::DataType::INT32: return 4;
		case Member::DataType::FVEC2: return 4 * 2;
		case Member::DataType::SHADER_HANDLE: {
			if constexpr (API == GAL::RenderAPI::VULKAN) { return 32; } //aligned size
		}
		default: BE_ASSERT(false, "Unknown value!")
		}
	}

	void updateDescriptors(TaskInfo taskInfo) {
		auto* renderSystem = taskInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");

		for (auto& e : queuedSetUpdates) {
			resizeSet(renderSystem, e);
		}

		queuedSetUpdates.Clear();

		auto& descriptorsUpdate = descriptorsUpdates[frame];

		for (auto& set : descriptorsUpdate.sets) {
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
	
	void updateCounter(TaskInfo taskInfo) {
		frame = (frame + 1) % queuedFrames;
	}

	static constexpr GAL::BindingType BUFFER_BINDING_TYPE = GAL::BindingType::STORAGE_BUFFER;

	void updateSubBindingsCount(SubSetHandle subSetHandle, uint32 newCount) {
		auto& set = sets[subSetHandle().SetHandle()];
		auto& subSet = set.SubSets[subSetHandle().Subset];

		RenderSystem* renderSystem;

		if (subSet.AllocatedBindings < newCount)
		{
			BE_ASSERT(false, "OOOO");
		}
	}

	struct BufferData {
		RenderAllocation RenderAllocations[MAX_CONCURRENT_FRAMES];
		GPUBuffer Buffers[MAX_CONCURRENT_FRAMES];
		//GTSL::Bitfield<128> WrittenAreas[MAX_CONCURRENT_FRAMES];
		bool Written[MAX_CONCURRENT_FRAMES]{ false };
		uint32 Size[MAX_CONCURRENT_FRAMES]{ 0 };
		
		struct MemberData {
			uint16 ByteOffsetIntoStruct;
			uint16 Count = 0;
			uint8 Level = 0;
			Member::DataType Type;
			uint16 Size;
		};
		GTSL::Array<MemberData, 16> MemberData;
	};
	GTSL::KeepVector<BufferData, BE::PAR> buffers;

	struct DescriptorsUpdate
	{
		DescriptorsUpdate() = default;

		void Initialize(const BE::PAR& allocator)
		{
			sets.Initialize(16, allocator);
		}

		void AddBufferUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsSet::BufferBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, subSetHandle().Type, BindingsSet::BindingUpdateInfo(update));
		}

		void AddTextureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsSet::TextureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, subSetHandle().Type, BindingsSet::BindingUpdateInfo(update));
		}

		void AddAccelerationStructureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingsSet::AccelerationStructureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, subSetHandle().Type, BindingsSet::BindingUpdateInfo(update));
		}
		
		void Reset()
		{
			sets.Clear();
		}

		GTSL::SparseVector<GTSL::SparseVector<GTSL::Pair<GAL::BindingType, GTSL::SparseVector<BindingsSet::BindingUpdateInfo, BE::PAR>>, BE::PAR>, BE::PAR> sets;

	private:
		void addUpdate(SubSetHandle subSetHandle, uint32 binding, GAL::BindingType bindingType, BindingsSet::BindingUpdateInfo update)
		{			
			if (sets.IsSlotOccupied(subSetHandle().SetHandle())) {
				auto& set = sets[subSetHandle().SetHandle()];
				
				if (set.IsSlotOccupied(subSetHandle().Subset)) {
					auto& subSet = set[subSetHandle().Subset];
					
					if (subSet.Second.IsSlotOccupied(binding)) {
						subSet.Second[binding] = update;
					}
					else { //there isn't binding
						subSet.Second.EmplaceAt(binding, update);
					}
				}
				else //there isn't sub set
				{
					auto& subSet = set.EmplaceAt(subSetHandle().Subset);
					subSet.First = bindingType;

					auto& bindings = subSet.Second;
					bindings.Initialize(32, sets.GetAllocator());
					bindings.EmplaceAt(binding, update);
				}
			} else { //there isn't set
				auto& set = sets.EmplaceAt(subSetHandle().SetHandle());
				
				set.Initialize(16, sets.GetAllocator());
				auto& subSet = set.EmplaceAt(subSetHandle().Subset);
				subSet.First = bindingType;
				
				auto& bindings = subSet.Second;
				bindings.Initialize(32, sets.GetAllocator()); //TODO: RIGHT NOW WE NEED MORE BINDINGS SINCE GROUPS ARE NOT DYNAMICALLY RESIZED, MAY NOT NEED TO ALLOCATE MUCH LATER DOWN THE ROAD
				bindings.EmplaceAt(binding, update);
			}
		}
	};
	
	GTSL::Array<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;

	/**
	 * \brief Stores all data per binding set.
	 */
	struct SetData {
		Id Name;
		//SetHandle Parent;
		uint32 Level = 0;
		PipelineLayout PipelineLayout;
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool;
		BindingsSet BindingsSet[MAX_CONCURRENT_FRAMES];

		/**
		 * \brief Stores all data per sub set, and manages managed buffers.
		 * Each struct instance is pointed to by one binding. But a big per sub set buffer is used to store all instances.
		 */
		struct SubSetData {
			uint32 AllocatedBindings = 0;
		};
		GTSL::Array<SubSetData, 16> SubSets;
	};
	
	GTSL::FlatHashMap<Id, SetHandle, BE::PAR> setHandlesByName;
	GTSL::KeepVector<SetData, BE::PAR> sets;

	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;

	struct SetLayoutData {
		uint8 Level = 0;

		Id Parent;
		BindingsSetLayout BindingsSetLayout;
		PipelineLayout PipelineLayout;
		GAL::ShaderStage Stage;
	};
	GTSL::FlatHashMap<Id, SetLayoutData, BE::PAR> setLayoutDatas;
	
	uint8 frame;
	uint8 queuedFrames = 2;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, Id setLayoutName, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors) {
		auto setHandle = SetHandle(sets.Emplace());
		auto& set = sets[setHandle()];

		setHandlesByName.Emplace(setName, setHandle);

		auto& setLayout = setLayoutDatas[setLayoutName];

		set.Level = setLayout.Level;
		set.BindingsSetLayout = setLayout.BindingsSetLayout;
		set.PipelineLayout = setLayout.PipelineLayout;

		if (bindingDescriptors.ElementCount()) {
			if constexpr (_DEBUG) {
				GTSL::StaticString<64> name("Bindings pool. Set: "); name += setName.GetString();
				//bindingsPoolCreateInfo.Name = name;
			}

			GTSL::Array<BindingsPool::BindingsPoolSize, 10> bindingsPoolSizes;

			for (auto e : bindingDescriptors) {
				bindingsPoolSizes.PushBack(BindingsPool::BindingsPoolSize{ e.BindingType, e.BindingsCount * queuedFrames });
			}

			set.BindingsPool.Initialize(renderSystem->GetRenderDevice(), bindingsPoolSizes, MAX_CONCURRENT_FRAMES);

			for (uint8 f = 0; f < queuedFrames; ++f) {
				if constexpr (_DEBUG) {
					GTSL::StaticString<64> name("BindingsSet. Set: "); name += setName.GetString();
				}

				set.BindingsSet[f].Initialize(renderSystem->GetRenderDevice(), set.BindingsPool, setLayout.BindingsSetLayout);
			}

			for (auto& e : bindingDescriptors) {
				set.SubSets.EmplaceBack(); auto& subSet = set.SubSets.back();
				subSet.AllocatedBindings = e.BindingsCount;
			}
		}

		return setHandle;
	}
	
	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle) {
		
	}
};
