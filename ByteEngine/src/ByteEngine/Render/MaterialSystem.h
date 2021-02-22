#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.hpp>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/StaticMap.hpp>
#include <GTSL/PagedVector.h>
#include <GTSL/Tree.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

#include "ByteEngine/Handle.hpp"

class TextureResourceManager;
struct TaskInfo;
class RenderSystem;

using MaterialInstanceHandle = Id;
using MaterialHandle = Id;

MAKE_HANDLE(uint32, Set)

struct SubSetDescription
{
	SetHandle SetHandle; uint32 Subset;
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

	MAKE_HANDLE(uint32, Texture);
	
	struct Member
	{
		enum class DataType : uint8
		{
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, FVEC4, FVEC2, STRUCT
		};

		uint32 Count = 1;
		DataType Type;
	};
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	Buffer GetBuffer(Id bufferName) const { return buffers[buffersByName[bufferName()]].Buffers[frame]; }
	Buffer GetBuffer(BufferHandle bufferHandle) const { return buffers[bufferHandle()].Buffers[frame]; }
	Texture GetTexture(TextureHandle textureHandle) const { return textures[textureHandle()].Texture; }

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

	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, Id setName, PipelineType pipelineType)
	{
		BindSet(renderSystem, commandBuffer, setHandlesByName.At(setName()), pipelineType);
	}
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle set, PipelineType pipelineType);
	
	bool BindMaterial(RenderSystem* renderSystem, CommandBuffer commandBuffer, MaterialHandle materialHandle);

	SetHandle GetSetHandleByName(const Id name) const { return setHandlesByName.At(name()); }

	void WriteSetTexture(SubSetHandle setHandle, uint32 index, Texture texture, TextureView textureView, TextureSampler textureSampler, bool writeAccess)
	{
		TextureLayout layout; BindingType bindingType;
		if (writeAccess) { layout = TextureLayout::GENERAL; bindingType = BindingType::STORAGE_IMAGE;  }
		else { layout = TextureLayout::SHADER_READ_ONLY; bindingType = BindingType::COMBINED_IMAGE_SAMPLER; }
		
		for(uint8 f = 0; f < queuedFrames; ++f)
		{
			BindingsSet::TextureBindingUpdateInfo info;
			info.TextureView = textureView;
			info.Sampler = textureSampler;
			info.TextureLayout = layout;
			
			descriptorsUpdates[f].AddTextureUpdate(setHandle, index, bindingType, info);
		}
	}
	
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
	[[nodiscard]] TextureHandle CreateTexture(RenderSystem* renderSystem, GAL::FormatDescriptor formatDescriptor, GTSL::Extent3D extent, TextureUses textureUses);

	void RecreateTexture(const TextureHandle textureHandle, RenderSystem* renderSystem, GTSL::Extent3D newExtent);
	
	TextureView GetTextureView(const TextureHandle textureHandle) const { return textures[textureHandle()].TextureView; }
	uint32 GetTextureReference(const TextureHandle textureHandle) const { return textureHandle(); }
	
	void BindBufferToName(const BufferHandle bufferHandle, const Id name) { buffersByName.Emplace(name(), bufferHandle()); }
	
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
	
	struct CreateMaterialInfo
	{
		Id MaterialName;
		MaterialResourceManager* MaterialResourceManager = nullptr;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager;
	};
	[[nodiscard]] MaterialInstanceHandle CreateMaterial(const CreateMaterialInfo& info);
	[[nodiscard]] MaterialInstanceHandle CreateRayTracingMaterial(const CreateMaterialInfo& info);

	MaterialInstanceHandle GetMaterialHandle(Id name) { return name; }

	static auto GetOnMaterialLoadEventHandle() { return EventHandle<MaterialHandle>("OnMaterialLoad"); }
	static auto GetOnMaterialInstanceLoadEventHandle() { return EventHandle<MaterialHandle, MaterialInstanceHandle>("OnMaterialInstanceLoad"); }
	
	void SetDynamicMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data);
	void SetMaterialParameter(const MaterialInstanceHandle material, GAL::ShaderDataType type, Id parameterName, void* data);

	[[nodiscard]] auto GetMaterialHandles() const { return readyMaterialHandles.GetRange(); }
	[[nodiscard]] auto GetPrivateMaterialHandles() const { return readyMaterialHandles.GetRange(); }

	auto GetMaterialHandlesForRenderGroup(Id renderGroup) const
	{
		if (readyMaterialsPerRenderGroup.Find(renderGroup())) //TODO: MAYBE ADD DECLARATION OF RENDER GROUP UP AHEAD AND AVOID THIS
		{
			return readyMaterialsPerRenderGroup.At(renderGroup()).GetRange();
		}
		else
		{
			return GTSL::Range<const MaterialInstanceHandle*>();
		}
	}

	void TraceRays(GTSL::Extent2D rayGrid, CommandBuffer* commandBuffer, RenderSystem* renderSystem);
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

	struct PrivateMaterialHandle
	{
		uint32 MaterialIndex = 0, MaterialInstance = 0;
	};
	
	PipelineLayout GetMaterialPipelineLayout(const MaterialHandle materialHandle)
	{
		return sets[loadedMaterialsMap[materialHandle()]].PipelineLayout;
	}

	PipelineLayout GetSetLayoutPipelineLayout(const Id setLayoutName) const
	{
		return setLayoutDatas[setLayoutName()].PipelineLayout;
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
	SubSetHandle topLevelAsHandle;

	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	static constexpr BindingType BUFFER_BINDING_TYPE = BindingType::STORAGE_BUFFER;
	
	SubSetHandle textureSubsetsHandle;
	
	GTSL::FlatHashMap<uint32, BE::PAR> shaderGroupsByName;

	uint32 shaderCounts[4]{ 0 };

	void updateSubBindingsCount(SubSetHandle subSetHandle, uint32 newCount);
	
	RayTracingPipeline rayTracingPipeline;
	Buffer shaderBindingTableBuffer;
	RenderAllocation shaderBindingTableAllocation;

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
	GTSL::FlatHashMap<uint32, BE::PAR> buffersByName;
	
	struct MaterialData
	{
		MaterialHandle Name;
		
		GTSL::KeepVector<uint32, BE::PAR> MaterialInstances;

		RasterizationPipeline Pipeline;
		Id RenderGroup;
		uint32 InstanceCount = 0;

		GTSL::StaticMap<MemberHandle, 16> ParametersHandles;
		
		GTSL::Array<MaterialResourceManager::Parameter, 16> Parameters;
		MemberHandle MaterialInstancesMemberHandle;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;

	struct MaterialInstanceData
	{
		MaterialInstanceHandle Name;
		
		uint32 Material = 0;
		uint8 Counter = 0, Target = 0;
	};
	GTSL::KeepVector<MaterialInstanceData, BE::PAR> materialInstances;

	struct SetLayoutData
	{
		uint8 Level = 0;

		Id Parent;
		BindingsSetLayout BindingsSetLayout;
		PipelineLayout PipelineLayout;
	};
	GTSL::FlatHashMap<SetLayoutData, BE::PAR> setLayoutDatas;
	GTSL::FlatHashMap<uint32, BE::PAR> loadedMaterialsMap;
	GTSL::FlatHashMap<uint32, BE::PAR> materialInstancesMap;
	GTSL::FlatHashMap<GTSL::Vector<MaterialInstanceHandle, BE::PAR>, BE::PAR> readyMaterialsPerRenderGroup;
	GTSL::Vector<PrivateMaterialHandle, BE::PAR> readyMaterialHandles;

	GTSL::FlatHashMap<PrivateMaterialHandle, BE::PAR> privateMaterialHandlesByName;
	PrivateMaterialHandle publicMaterialHandleToPrivateMaterialHandle(MaterialInstanceHandle materialHandle) const { return privateMaterialHandlesByName.At(materialHandle()); }
	
	void setMaterialAsLoaded(const PrivateMaterialHandle privateMaterialHandle);

	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
		PrivateMaterialHandle MaterialHandle;
	};
	TextureHandle createTexture(const CreateTextureInfo& createTextureInfo);

	struct DescriptorsUpdate
	{
		DescriptorsUpdate() = default;

		void Initialize(const BE::PAR& allocator)
		{
			sets.Initialize(16, allocator);
		}

		void AddBufferUpdate(SubSetHandle subSetHandle, uint32 binding, BindingType bindingType, BindingsSet::BufferBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, bindingType, BindingsSet::BindingUpdateInfo(update));
		}

		void AddTextureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingType bindingType, BindingsSet::TextureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, bindingType, BindingsSet::BindingUpdateInfo(update));
		}

		void AddAccelerationStructureUpdate(SubSetHandle subSetHandle, uint32 binding, BindingType bindingType, BindingsSet::AccelerationStructureBindingUpdateInfo update)
		{
			addUpdate(subSetHandle, binding, bindingType, BindingsSet::BindingUpdateInfo(update));
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
	
	struct RenderGroupData
	{
		uint32 SetReference;
	};
	GTSL::FlatHashMap<RenderGroupData, BE::PAR> renderGroupsData;

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
	
	GTSL::FlatHashMap<SetHandle, BE::PAR> setHandlesByName;
	GTSL::KeepVector<SetData, BE::PAR> sets;

	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;
	
	struct TextureLoadInfo
	{
		TextureLoadInfo() = default;
		
		TextureLoadInfo(uint32 component, Buffer buffer, RenderSystem* renderSystem, RenderAllocation renderAllocation) : Component(component), Buffer(buffer), RenderSystem(renderSystem), RenderAllocation(renderAllocation)
		{
		}

		uint32 Component;
		Buffer Buffer;
		RenderSystem* RenderSystem;
		RenderAllocation RenderAllocation;
	};
	void onTextureInfoLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager* resourceManager, TextureResourceManager::TextureInfo textureInfo, TextureLoadInfo loadInfo);

	struct ShaderLoadInfo
	{
		ShaderLoadInfo() = default;
		ShaderLoadInfo(ShaderLoadInfo&& other) noexcept : Buffer(GTSL::MoveRef(other.Buffer)), Component(other.Component) {}
		GTSL::Buffer<BE::PAR> Buffer; uint32 Component;
	};
	
	void onShaderInfosLoaded(TaskInfo taskInfo, MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaderInfos, ShaderLoadInfo shaderLoadInfo);
	void onShadersLoaded(TaskInfo taskInfo, ::MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8> shaders, GTSL::Range<byte*> buffer, ShaderLoadInfo shaderLoadInfo);
	
	struct TextureComponent
	{
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;
		RenderAllocation Allocation;
		
		GAL::FormatDescriptor FormatDescriptor;
		
	};
	GTSL::KeepVector<TextureComponent, BE::PersistentAllocatorReference> textures;
	GTSL::FlatHashMap<uint32, BE::PersistentAllocatorReference> texturesRefTable;
	
	GTSL::Vector<uint32, BE::PAR> latestLoadedTextures;
	GTSL::KeepVector<GTSL::Vector<PrivateMaterialHandle, BE::PAR>, BE::PersistentAllocatorReference> pendingMaterialsPerTexture;
	
	void addPendingMaterialToTexture(TextureHandle texture, PrivateMaterialHandle material)
	{
		pendingMaterialsPerTexture[texture()].EmplaceBack(material);
	}
	
	struct MaterialLoadInfo
	{
		MaterialLoadInfo(RenderSystem* renderSystem, GTSL::Buffer<BE::PAR>&& buffer, uint32 index, uint32 instanceIndex, TextureResourceManager* tRM) : RenderSystem(renderSystem), Buffer(MoveRef(buffer)), Component(index), InstanceIndex(instanceIndex), TextureResourceManager(tRM)
		{

		}

		RenderSystem* RenderSystem = nullptr;
		GTSL::Buffer<BE::PAR> Buffer;
		uint32 Component, InstanceIndex;
		TextureResourceManager* TextureResourceManager;
	};
	void onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo);

	void createBuffers(RenderSystem* renderSystem, const uint32 bufferSet);
	
	uint8 frame;
	uint8 queuedFrames = 2;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, Id setLayoutName, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors);
	
	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle);

	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, ShaderLoadInfo> onShaderInfosLoadHandle;
	DynamicTaskHandle<MaterialResourceManager*, GTSL::Array<MaterialResourceManager::ShaderInfo, 8>, GTSL::Range<byte*>, ShaderLoadInfo> onShadersLoad;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, MaterialSystem::TextureLoadInfo> onTextureInfoLoadHandle;
	DynamicTaskHandle<TextureResourceManager*, TextureResourceManager::TextureInfo, MaterialSystem::TextureLoadInfo> onTextureLoadHandle;
	
	friend class RenderSystem;
};
