#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/StaticMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/Tree.hpp>

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
	
	struct Member
	{
		enum class DataType : uint8
		{
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, MATRIX3X4, FVEC4, FVEC2, STRUCT, PAD,
			SHADER_HANDLE
		};

		Member() = default;
		Member(const uint32 count, const DataType type) : Count(count), Type(type) {}
		
		uint32 Count = 1;
		DataType Type = DataType::PAD;
	};
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	Buffer GetBuffer(Id bufferName) const { return buffers[buffersByName[bufferName]].Buffers[frame]; }
	Buffer GetBuffer(BufferHandle bufferHandle) const { return buffers[bufferHandle()].Buffers[frame]; }
	PipelineLayout GetSetLayoutPipelineLayout(Id id) const { return setLayoutDatas[id].PipelineLayout; }
	
	void UpdateSet(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure)
	{
		for (uint8 f = 0; f < queuedFrames; ++f) { descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure }); }
	}

	void UpdateSet2(SubSetHandle subSetHandle, uint32 bindingIndex, AccelerationStructure accelerationStructure, uint8 f)
	{
		descriptorsUpdates[f].AddAccelerationStructureUpdate(subSetHandle, bindingIndex, { accelerationStructure });
	}

	GTSL::uint64 GetBufferAddress(RenderSystem* renderSystem, const BufferHandle bufferHandle) const
	{
		GTSL::uint64 address = 0;
		if (buffers[bufferHandle()].Buffers[frame].GetVkBuffer()) {
			address = buffers[bufferHandle()].Buffers[frame].GetAddress(renderSystem->GetRenderDevice());
		}
		return address;
	}
	RenderSystem::BufferAddress GetBufferAddress(RenderSystem* renderSystem, const Id bufferName) const { return RenderSystem::BufferAddress(buffers[buffersByName[bufferName]].Buffers[frame].GetAddress(renderSystem->GetRenderDevice())); }
	
	bool DoesBufferExist(const Id buffer) const { return buffersByName.Find(buffer); }
	
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
	T* GetMemberPointer(BufferIterator& iterator, MemberHandle<T> memberHandle, uint32 i = 0)
	{
		//static_assert(T != (void*), "Type can not be struct.");
		
		auto& bufferData = buffers[memberHandle.BufferIndex];
		auto& member = bufferData.MemberData[memberHandle.MemberIndirectionIndex];
		BE_ASSERT(i < member.Count, "Requested sub set buffer member index greater than allocated instances count.")
		
		//												//BUFFER							//OFFSET TO STRUCT
		return reinterpret_cast<T*>(static_cast<byte*>(bufferData.RenderAllocations[frame].Data) + iterator.ByteOffset + member.ByteOffsetIntoStruct + member.Size * i);
	}

	template<typename T>
	void WriteMultiBuffer(BufferIterator& iterator, MemberHandle<T> memberHandle, T* data, uint32 i = 0)
	{
		auto& bufferData = buffers[memberHandle.BufferIndex]; auto& member = bufferData.MemberData[memberHandle.MemberIndirectionIndex];
		for(uint8 f = 0; f < queuedFrames; ++f) {
			*reinterpret_cast<T*>(static_cast<byte*>(bufferData.RenderAllocations[f].Data) + iterator.ByteOffset + member.ByteOffsetIntoStruct + member.Size * i) = *data;
		}
	}

	template<>
	void WriteMultiBuffer(BufferIterator& iterator, MemberHandle<GAL::ShaderHandle> memberHandle, GAL::ShaderHandle* data, uint32 i)
	{
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
	
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle set, GAL::ShaderStage shaderStage);

	SetHandle GetSetHandleByName(const Id name) const { return setHandlesByName.At(name); }

	void WriteSetTexture(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex);

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

	enum class SubSetType : uint8
	{
		BUFFER, READ_TEXTURES, WRITE_TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE
	};

	struct SubSetDescriptor
	{
		SubSetType SubSetType; uint32 BindingsCount;
	};
	void AddSetLayout(RenderSystem* renderSystem, Id layoutName, Id parentName, const GTSL::Range<SubSetDescriptor*> subsets);
	
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
		
	SetHandle AddSet(RenderSystem* renderSystem, Id setName, Id setLayoutName, const GTSL::Range<SubSetInfo*> setInfo);

	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, GTSL::Range<MemberInfo*> members);
	[[nodiscard]] BufferHandle CreateBuffer(RenderSystem* renderSystem, MemberInfo member) {
		return CreateBuffer(renderSystem, GTSL::Range<MemberInfo*>(1, &member));
	}
	
	void BindBufferToName(const BufferHandle bufferHandle, const Id name) { buffersByName.Emplace(name, bufferHandle()); }
	
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

	void Dispatch(GTSL::Extent2D workGroups, CommandBuffer* commandBuffer, RenderSystem* renderSystem);

	uint32 CreateComputePipeline(Id materialName, MaterialResourceManager* materialResourceManager, GameInstance* gameInstance);
	
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
	}

private:
	
	uint32 dataTypeSize(MaterialSystem::Member::DataType data)
	{
		switch (data)
		{
		case MaterialSystem::Member::DataType::FLOAT32: return 4;
		case MaterialSystem::Member::DataType::UINT32: return 4;
		case MaterialSystem::Member::DataType::UINT64: return 8;
		case MaterialSystem::Member::DataType::MATRIX4: return 4 * 4 * 4;
		case MaterialSystem::Member::DataType::MATRIX3X4: return 4 * 3 * 4;
		case MaterialSystem::Member::DataType::FVEC4: return 4 * 4;
		case MaterialSystem::Member::DataType::INT32: return 4;
		case MaterialSystem::Member::DataType::FVEC2: return 4 * 2;
		case MaterialSystem::Member::DataType::SHADER_HANDLE:
		{
			if constexpr (API == GAL::RenderAPI::VULKAN) { return 32; } //aligned size
		}
		default: BE_ASSERT(false, "Unknown value!")
		}
	}

	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	static constexpr GAL::BindingType BUFFER_BINDING_TYPE = GAL::BindingType::STORAGE_BUFFER;

	void updateSubBindingsCount(SubSetHandle subSetHandle, uint32 newCount);

	struct BufferData
	{
		RenderAllocation RenderAllocations[MAX_CONCURRENT_FRAMES];
		Buffer Buffers[MAX_CONCURRENT_FRAMES];

		struct MemberData
		{
			uint16 ByteOffsetIntoStruct;
			uint16 Count = 0;
			uint8 Level = 0;
			Member::DataType Type;
			uint16 Size;
		};
		GTSL::Array<MemberData, 16> MemberData;
	};
	GTSL::KeepVector<BufferData, BE::PAR> buffers;
	GTSL::FlatHashMap<Id, uint32, BE::PAR> buffersByName;

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
			}
			else { //there isn't set
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
	struct SetData
	{
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
		struct SubSetData
		{				
			uint32 AllocatedBindings = 0;
		};
		GTSL::Array<SubSetData, 16> SubSets;
	};
	
	GTSL::FlatHashMap<Id, SetHandle, BE::PAR> setHandlesByName;
	GTSL::KeepVector<SetData, BE::PAR> sets;

	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;

	struct SetLayoutData
	{
		uint8 Level = 0;

		Id Parent;
		BindingsSetLayout BindingsSetLayout;
		PipelineLayout PipelineLayout;
		GAL::ShaderStage Stage;
	};
	GTSL::FlatHashMap<Id, SetLayoutData, BE::PAR> setLayoutDatas;
	
	uint8 frame;
	uint8 queuedFrames = 2;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, Id setLayoutName, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors);
	
	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle);
};
