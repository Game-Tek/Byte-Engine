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

using MaterialInstanceHandle = Id;
using MaterialHandle = Id;

MAKE_HANDLE(uint32, Set)

struct SubSetDescription
{
	SetHandle SetHandle; uint32 Subset; BindingType Type;
};

MAKE_HANDLE(SubSetDescription, SubSet)

struct MemberDescription
{
	uint32 BufferIndex = 0;
	uint32 MemberIndirectionIndex = 0;
};

MAKE_HANDLE(MemberDescription, Member)

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
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, FVEC4, FVEC2, STRUCT, PAD
		};

		uint32 Count = 1;
		DataType Type;
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

	GTSL::uint64 GetBufferAddress(RenderSystem* renderSystem, const BufferHandle bufferHandle) const { return buffers[bufferHandle()].Buffers[frame].GetAddress(renderSystem->GetRenderDevice()); }
	struct BufferIterator { uint32 Level = 0, ByteOffset = 0, MemberIndex = 0; MemberHandle Member; };
	
	template<typename T>
	T* GetMemberPointer(BufferIterator iterator);

	template<>
	GTSL::Matrix4* GetMemberPointer(BufferIterator iterator)
	{
		return getSetMemberPointer<GTSL::Matrix4, Member::DataType::MATRIX4>(iterator, frame);
	}

	template<>
	GTSL::Vector4* GetMemberPointer(BufferIterator iterator)
	{
		return getSetMemberPointer<GTSL::Vector4, Member::DataType::FVEC4>(iterator, frame);
	}
	
	template<>
	int32* GetMemberPointer(BufferIterator iterator)
	{
		return getSetMemberPointer<int32, Member::DataType::INT32>(iterator, frame);
	}
	
	template<>
	uint32* GetMemberPointer(BufferIterator iterator)
	{
		return getSetMemberPointer<uint32, Member::DataType::UINT32>(iterator, frame);
	}
	
	template<>
	uint64* GetMemberPointer(BufferIterator iterator)
	{
		return getSetMemberPointer<uint64, Member::DataType::UINT64>(iterator, frame);
	}

	void WriteMultiBuffer(BufferIterator iterator, const uint32* data)
	{
		for(uint8 f = 0; f < queuedFrames; ++f) {
			*getSetMemberPointer<uint32, Member::DataType::UINT32>(iterator, f) = *data;
		}
	}
	
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, Id setName, PipelineType pipelineType)
	{
		BindSet(renderSystem, commandBuffer, setHandlesByName.At(setName), pipelineType);
	}
	
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle set, PipelineType pipelineType);

	SetHandle GetSetHandleByName(const Id name) const { return setHandlesByName.At(name); }

	void WriteSetTexture(const RenderSystem* renderSystem, SubSetHandle setHandle, RenderSystem::TextureHandle textureHandle, uint32 bindingIndex);

	struct MemberInfo : Member
	{
		MemberHandle* Handle;
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
	
	void BindBufferToName(const BufferHandle bufferHandle, const Id name) { buffersByName.Emplace(name, bufferHandle()); }
	
	/**
	 * \brief Update the member instance count to be able to fit at least count requested elements.
	 * \param renderSystem Pointer to the active RenderSystem instance.
	 * \param memberHandle Handle to the member whose count is going to be updated.
	 * \param count Number to check against if enough instances are allocated. Doesn't have to be incremental, can be any index as long as it represents the new boundary of the collection(in terms of indeces) or any index inside the range.
	 */
	void UpdateObjectCount(RenderSystem* renderSystem, MemberHandle memberHandle, uint32 count);

	struct BindingsSetData
	{
		BindingsSetLayout BindingsSetLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		uint32 DataSize = 0;
	};

	static auto GetOnMaterialLoadEventHandle() { return EventHandle<MaterialHandle>("OnMaterialLoad"); }
	static auto GetOnMaterialInstanceLoadEventHandle() { return EventHandle<MaterialHandle, MaterialInstanceHandle>("OnMaterialInstanceLoad"); }
	
	void SetDynamicMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data);
	void SetMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data);

	void Dispatch(GTSL::Extent2D workGroups, CommandBuffer* commandBuffer, RenderSystem* renderSystem);

	uint32 CreateComputePipeline(Id materialName, MaterialResourceManager* materialResourceManager, GameInstance* gameInstance);
	
	void SetRayGenMaterial(Id rayGen) { rayGenMaterial = rayGen; }
	
	/**
	 * \brief Updates the iterator hierarchy level to index the specified member.
	 * \param iterator BufferIterator object to update.
	 * \param member MemberHandle that refers to the struct that we want the iterator to point to.
	 */
	void UpdateIteratorMember(BufferIterator& iterator, MemberHandle member)
	{
		auto& bufferData = buffers[member().BufferIndex]; auto& memberData = bufferData.MemberData[member().MemberIndirectionIndex];
		iterator.Level = memberData.Level;
		iterator.ByteOffset += memberData.ByteOffsetIntoStruct;
		iterator.Member = member;
		iterator.MemberIndex = 0;
	}
	
	/**
	 * \brief Updates the iterator to reference the previously indicated member, at index.
	 * \param iterator BufferIterator object to update.
	 * \param index Index of the member we want to address.
	 */
	void UpdateIteratorMemberIndex(BufferIterator& iterator, uint32 index)
	{
		auto& bufferData = buffers[iterator.Member().BufferIndex]; auto& memberData = bufferData.MemberData[iterator.Member().MemberIndirectionIndex];
		BE_ASSERT(memberData.Level == iterator.Level, "Not expected structure");
		BE_ASSERT(index < memberData.Count, "Advanced more elements than there are in this member!");
		int32 shiftedElements = index - iterator.MemberIndex;
		iterator.ByteOffset += shiftedElements * memberData.Size;
		BE_ASSERT(iterator.ByteOffset < bufferData.RenderAllocations[0].Size, "");
		iterator.MemberIndex = index;
	}

	void WriteInstance(const uint32 instanceIndex, const uint32 vertexBuffer, const uint32 indexBuffer, const uint32 materialInstance, const uint32 renderGroupIndex)
	{
		//BufferIterator iterator;
		//UpdateIteratorMember(iterator, instanceDataHandle);
		//UpdateIteratorMemberIndex(iterator, instanceIndex);
		//UpdateIteratorMember(iterator, instanceElementsHandle);
		//WriteMultiBuffer(iterator, &vertexBuffer);
		//UpdateIteratorMemberIndex(iterator, 1);
		//WriteMultiBuffer(iterator, &indexBuffer);
		//UpdateIteratorMemberIndex(iterator, 2);
		//WriteMultiBuffer(iterator, &materialInstance);
		//UpdateIteratorMemberIndex(iterator, 3);
		//WriteMultiBuffer(iterator, &renderGroupIndex);
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
		case MaterialSystem::Member::DataType::FVEC4: return 4 * 4;
		case MaterialSystem::Member::DataType::INT32: return 4;
		case MaterialSystem::Member::DataType::FVEC2: return 4 * 2;
		default: BE_ASSERT(false, "Unknown value!")
		}
	}
	
	template<typename T, Member::DataType DT>
	T* getSetMemberPointer(BufferIterator iterator, uint8 frameToUpdate)
	{
		auto& bufferData = buffers[iterator.Member().BufferIndex];

		BE_ASSERT(DT == bufferData.MemberData[iterator.Member().MemberIndirectionIndex].Type, "Type mismatch")
		//BE_ASSERT(index < s., "Requested sub set buffer member index greater than allocated instances count.")

		//												//BUFFER							//OFFSET TO STRUCT
		return reinterpret_cast<T*>(static_cast<byte*>(bufferData.RenderAllocations[frameToUpdate].Data) + iterator.ByteOffset);
	}
	
	Id rayGenMaterial;

	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	static constexpr BindingType BUFFER_BINDING_TYPE = BindingType::STORAGE_BUFFER;

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

		GTSL::SparseVector<GTSL::SparseVector<GTSL::Pair<BindingType, GTSL::SparseVector<BindingsSet::BindingUpdateInfo, BE::PAR>>, BE::PAR>, BE::PAR> sets;

	private:
		void addUpdate(SubSetHandle subSetHandle, uint32 binding, BindingType bindingType, BindingsSet::BindingUpdateInfo update)
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
	};
	GTSL::FlatHashMap<Id, SetLayoutData, BE::PAR> setLayoutDatas;
	
	void createBuffers(RenderSystem* renderSystem, const uint32 bufferSet);
	
	uint8 frame;
	uint8 queuedFrames = 2;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, Id setLayoutName, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors);
	
	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle);
};
