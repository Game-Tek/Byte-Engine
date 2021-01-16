#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.h>
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

struct MaterialHandle
{
	Id MaterialType;
	uint32 MaterialInstance = 0;
	uint32 Element = 0;
};

MAKE_HANDLE(uint32, Set)

struct SubSetDescription
{
	SetHandle SetHandle; uint32 Subset;
};

MAKE_HANDLE(SubSetDescription, SubSet)

struct MemberDescription
{
	SubSetDescription SubSet;
	uint32 MemberIndirectionIndex = 0;
};

MAKE_HANDLE(MemberDescription, Member)

class MaterialSystem : public System
{	
public:
	MaterialSystem() : System("MaterialSystem")
	{}

	struct Member
	{
		enum class DataType : uint8
		{
			FLOAT32, INT32, UINT32, UINT64, MATRIX4, FVEC4, FVEC2, STRUCT
		};

		uint32 Count = 1, Reference = 0xFFFFFFFF;
		DataType Type;
	};
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	struct BufferIterator { uint32 Set = 0, SubSet = 0, Level = 0, ByteOffset = 0, MemberIndex = 0; MemberHandle Member; };
	
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

	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, Id setName, uint32 index = 0)
	{
		BindSet(renderSystem, commandBuffer, setHandlesByName.At(setName()), index);
	}
	void BindSet(RenderSystem* renderSystem, CommandBuffer commandBuffer, SetHandle set, uint32 index = 0);
	
	bool BindMaterial(RenderSystem* renderSystem, CommandBuffer commandBuffer, MaterialHandle set);

	SetHandle GetSetHandleByName(const Id name) const { return setHandlesByName.At(name()); }
	
	void WriteSetTexture(SetHandle setHandle, uint32 index, Texture texture, TextureView textureView, TextureSampler textureSampler)
	{		
		for(uint8 f = 0; f < queuedFrames; ++f)
		{
			auto updateHandle = descriptorsUpdates[f].AddSetToUpdate(setHandle, GetPersistentAllocator());

			BindingsSet::TextureBindingUpdateInfo info;
			info.TextureView = textureView;
			info.Sampler = textureSampler;
			info.TextureLayout = TextureLayout::GENERAL;
			
			descriptorsUpdates[f].AddTextureUpdate(updateHandle, index, 1, BindingType::STORAGE_IMAGE, info);
		}
	}
	
	struct MemberInfo : Member
	{
		MemberHandle* Handle;
		GTSL::Range<MemberInfo*> MemberInfos;
	};

	enum class Frequency : uint8
	{
		PER_INSTANCE
	};

	enum class SubSetType : uint8
	{
		BUFFER, TEXTURES, RENDER_ATTACHMENT, ACCELERATION_STRUCTURE
	};
	
	struct StructInfo
	{
		Frequency Frequency;
		GTSL::Range<MemberInfo*> Members;
	};
	
	struct SetInfo
	{
		GTSL::Range<StructInfo*> Structs;
	};
	SetHandle AddSet(RenderSystem* renderSystem, Id setName, Id parent, const SetInfo& setInfo);
	
	struct SubSetInfo
	{
		SubSetType Type;
		SubSetHandle* Handle;
		uint32 Count;
	};
	
	struct SetXInfo
	{
		GTSL::Range<SubSetInfo*> SubSets;
	};
	SetHandle AddSetX(RenderSystem* renderSystem, Id setName, Id parent, const SetXInfo& setInfo);
	
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
	[[nodiscard]] MaterialHandle CreateMaterial(const CreateMaterialInfo& info);
	[[nodiscard]] MaterialHandle CreateRayTracingMaterial(const CreateMaterialInfo& info);

	void SetDynamicMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data);
	void SetMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data);

	[[nodiscard]] auto GetMaterialHandles() const { return readyMaterialHandles.GetRange(); }

	auto GetMaterialHandlesForRenderGroup(Id renderGroup) const
	{
		if (readyMaterialsPerRenderGroup.Find(renderGroup())) //TODO: MAYBE ADD DECLARATION OF RENDER GROUP UP AHEAD AND AVOID THIS
		{
			return readyMaterialsPerRenderGroup.At(renderGroup()).GetRange();
		}
		else
		{
			return GTSL::Range<const MaterialHandle*>();
		}
	}

	void TraceRays(GTSL::Extent2D rayGrid, CommandBuffer* commandBuffer, RenderSystem* renderSystem);

	void SetRayGenMaterial(Id rayGen) { rayGenMaterial = rayGen; }

	auto GetCameraMatricesHandle() const { return cameraMatricesHandle; }
	
	void UpdateIteratorMember(BufferIterator& iterator, MemberHandle member)
	{
		auto& set = sets[member().SubSet.SetHandle()]; auto& memberData = set.MemberData[member().MemberIndirectionIndex];
		iterator.Set = member().SubSet.SetHandle(); iterator.SubSet = member().SubSet.Subset;
		iterator.Level = memberData.Level;
		iterator.ByteOffset += memberData.ByteOffsetIntoStruct;
		iterator.Member = member;
		iterator.MemberIndex = 0;
	}
	
	void UpdateIteratorMemberIndex(BufferIterator& iterator, uint32 index)
	{
		auto& set = sets[iterator.Set]; auto& memberData = set.MemberData[iterator.Member().MemberIndirectionIndex];
		BE_ASSERT(memberData.Level == iterator.Level, "Not expected structure");
		BE_ASSERT(index < memberData.Count, "Advanced more elements than there are in this member!");
		int32 shiftedElements = index - iterator.MemberIndex;
		iterator.ByteOffset += shiftedElements * memberData.Size;
		iterator.MemberIndex = index;
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
		auto& set = sets[iterator.Set];
		auto& subSet = set.SubSets[iterator.SubSet];

		BE_ASSERT(DT == set.MemberData[iterator.Member().MemberIndirectionIndex].Type, "Type mismatch")
		//BE_ASSERT(index < s., "Requested sub set buffer member index greater than allocated instances count.")

		//												//BUFFER							//OFFSET TO STRUCT
		return reinterpret_cast<T*>(static_cast<byte*>(subSet.Allocations[frameToUpdate].Data) + iterator.ByteOffset);
	}
	
	Id rayGenMaterial;
	SubSetHandle cameraDataSubSetHandle;
	MemberHandle cameraMatricesHandle;
	MemberHandle materialTextureHandles;
	SubSetHandle materialsDataSubSetHandle;
	MemberHandle materialDataStructHandle;
	SubSetHandle instanceDataSubsetHandle;
	MemberHandle instanceMaterialReferenceHandle;
	MemberHandle instanceDataStructHandle;
	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	static constexpr BindingType BUFFER_BINDING_TYPE = BindingType::STORAGE_BUFFER;
	
	SubSetHandle textureSubsetsHandle;
	SubSetHandle attachmentsHandle;
	SubSetHandle topLevelAsHandle;
	SubSetHandle vertexBuffersSubSetHandle;
	SubSetHandle indexBuffersSubSetHandle;
	
	GTSL::FlatHashMap<uint32, BE::PAR> shaderGroupsByName;

	uint32 shaderCounts[4]{ 0 };

	void createBuffer(RenderSystem* renderSystem, SubSetHandle subSetHandle, GTSL::Range<MemberInfo*> members);
	
	uint32 matNum = 0;
	
	RayTracingPipeline rayTracingPipeline;
	Buffer shaderBindingTableBuffer;
	RenderAllocation shaderBindingTableAllocation;

	struct MaterialData
	{
		struct MaterialInstanceData
		{
			
		};
		
		GTSL::KeepVector<MaterialInstanceData, BE::PAR> MaterialInstances;

		SetHandle Set;
		RasterizationPipeline Pipeline;
		MemberHandle TextureHandles;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;

	struct PendingMaterialData : MaterialData
	{
		PendingMaterialData(uint32 targetValue, MaterialData&& materialData) : MaterialData(materialData), Target(targetValue) {}
		
		uint32 Counter = 0, Target = 0;
		MaterialHandle Material;
		Id RenderGroup;
	};
	GTSL::KeepVector<PendingMaterialData, BE::PAR> pendingMaterials;
	GTSL::FlatHashMap<uint32, BE::PAR> readyMaterialsMap;
	GTSL::FlatHashMap<GTSL::Vector<MaterialHandle, BE::PAR>, BE::PAR> readyMaterialsPerRenderGroup;
	GTSL::Vector<MaterialHandle, BE::PAR> readyMaterialHandles;
	
	void setMaterialAsLoaded(const MaterialHandle matIndex, const MaterialData material, const Id renderGroup);

	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
		MaterialHandle MaterialHandle;
	};
	ComponentReference createTexture(const CreateTextureInfo& createTextureInfo);

	struct DescriptorsUpdate
	{
		DescriptorsUpdate() = default;

		void Initialize(const BE::PAR& allocator)
		{
			setsToUpdate.Initialize(4, allocator);
			PerSetToUpdateBindingUpdate.Initialize(4, allocator);
			PerSetToUpdateData.Initialize(4, allocator);
		}

		[[nodiscard]] uint32 AddSetToUpdate(SetHandle set, const BE::PAR& allocator)
		{
			const auto handle = setsToUpdate.EmplaceBack(set());
			PerSetToUpdateBindingUpdate.EmplaceBack(4, allocator);
			PerSetToUpdateData.EmplaceBack(4, allocator);
			return handle;
		}

		void AddBufferUpdate(uint32 set, uint32 binding, uint32 subSet, BindingType bindingType, BindingsSet::BufferBindingUpdateInfo update)
		{
			PerSetToUpdateData[set].EmplaceBack(bindingType, subSet);
			PerSetToUpdateBindingUpdate[set].EmplaceAt(binding, update);
		}

		void AddTextureUpdate(uint32 set, uint32 binding, uint32 subSet, BindingType bindingType, BindingsSet::TextureBindingUpdateInfo update)
		{
			PerSetToUpdateData[set].EmplaceBack(bindingType, subSet);
			PerSetToUpdateBindingUpdate[set].EmplaceAt(binding, update);
		}

		void AddAccelerationStructureUpdate(uint32 set, uint32 binding, uint32 subSet, BindingType bindingType, BindingsSet::AccelerationStructureBindingUpdateInfo update)
		{
			PerSetToUpdateData[set].EmplaceBack(bindingType, subSet);
			PerSetToUpdateBindingUpdate[set].EmplaceAt(binding, update);
		}
		
		void Reset()
		{
			setsToUpdate.ResizeDown(0);
			PerSetToUpdateBindingUpdate.ResizeDown(0);
			PerSetToUpdateData.ResizeDown(0);
		}
		
		GTSL::Vector<SetHandle, BE::PAR> setsToUpdate;

		GTSL::Vector<GTSL::SparseVector<BindingsSet::BindingUpdateInfo, BE::PAR>, BE::PAR> PerSetToUpdateBindingUpdate;

		struct UpdateData
		{
			BindingType BindingType; uint32 SubSetIndex;
		};
		
		GTSL::Vector<GTSL::Vector<UpdateData, BE::PAR>, BE::PAR> PerSetToUpdateData;
	};
	GTSL::Array<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;
	
	struct RenderGroupData
	{
		uint32 SetReference;
	};
	GTSL::FlatHashMap<RenderGroupData, BE::PAR> renderGroupsData;

	struct Struct
	{
		GTSL::Array<Member, 8> Members;
	};

	struct StructData : Struct
	{
		
	};
	
	/**
	 * \brief Stores all data per binding set.
	 */
	struct SetData
	{
		Id Name;
		SetHandle Parent;
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
			RenderAllocation Allocations[MAX_CONCURRENT_FRAMES];
			Buffer Buffers[MAX_CONCURRENT_FRAMES];
						
			uint32 AllocatedBindings = 0;

			//GTSL::Array<StructData, 16> DefinedStructs;
		};
		GTSL::Array<SubSetData, 8> SubSets;
		
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
	
	GTSL::FlatHashMap<SetHandle, BE::PAR> setHandlesByName;
	GTSL::KeepVector<SetData, BE::PAR> sets;

	GTSL::PagedVector<SetHandle, BE::PAR> queuedSetUpdates;
	
	struct TextureLoadInfo
	{
		TextureLoadInfo(uint32 component, Buffer buffer, RenderSystem* renderSystem, RenderAllocation renderAllocation) : Component(component), Buffer(buffer), RenderSystem(renderSystem), RenderAllocation(renderAllocation)
		{
		}

		uint32 Component;
		Buffer Buffer;
		RenderSystem* RenderSystem;
		RenderAllocation RenderAllocation;
	};
	void onTextureLoad(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo loadInfo);
	
	void onTextureProcessed(TaskInfo taskInfo, TextureResourceManager::OnTextureLoadInfo loadInfo);
	
	struct TextureComponent
	{
		Texture Texture;
		TextureView TextureView;
		TextureSampler TextureSampler;
		RenderAllocation Allocation;
	};
	GTSL::KeepVector<TextureComponent, BE::PersistentAllocatorReference> textures;
	GTSL::FlatHashMap<uint32, BE::PersistentAllocatorReference> texturesRefTable;

	MAKE_HANDLE(uint32, PendingMaterial)
	
	GTSL::Vector<uint32, BE::PAR> latestLoadedTextures;
	GTSL::KeepVector<GTSL::Vector<PendingMaterialHandle, BE::PAR>, BE::PersistentAllocatorReference> pendingMaterialsPerTexture;
	
	void addPendingMaterialToTexture(uint32 texture, PendingMaterialHandle material)
	{
		pendingMaterialsPerTexture[texture].EmplaceBack(material);
	}
	
	struct MaterialLoadInfo
	{
		MaterialLoadInfo(RenderSystem* renderSystem, GTSL::Buffer&& buffer, uint32 index, TextureResourceManager* tRM) : RenderSystem(renderSystem), Buffer(MoveRef(buffer)), Component(index), TextureResourceManager(tRM)
		{

		}

		RenderSystem* RenderSystem = nullptr;
		GTSL::Buffer Buffer;
		uint32 Component;
		TextureResourceManager* TextureResourceManager;
	};
	void onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo);
	
	template<typename C, typename C2>
	void genShaderStages(RenderDevice* renderDevice, C& container, C2& shaderInfos, const MaterialResourceManager::OnMaterialLoadInfo& onMaterialLoadInfo)
	{
		uint32 offset = 0;
		
		for (uint32 i = 0; i < onMaterialLoadInfo.ShaderTypes.GetLength(); ++i)
		{
			Shader::CreateInfo create_info;
			create_info.RenderDevice = renderDevice;
			create_info.ShaderData = GTSL::Range<const byte*>(onMaterialLoadInfo.ShaderSizes[i], onMaterialLoadInfo.DataBuffer.begin() + offset);
			container.EmplaceBack(create_info);
			offset += onMaterialLoadInfo.ShaderSizes[i];
		}

		for (uint32 i = 0; i < container.GetLength(); ++i)
		{
			shaderInfos.PushBack({ ConvertShaderType(onMaterialLoadInfo.ShaderTypes[i]), container[i] });
		}
	}

	void createBuffers(RenderSystem* renderSystem, const uint32 bufferSet);
	
	uint8 frame;
	const uint8 queuedFrames = 2;

	SetHandle makeSetEx(RenderSystem* renderSystem, Id setName, Id parent, GTSL::Range<BindingsSetLayout::BindingDescriptor*> bindingDescriptors);
	
	void resizeSet(RenderSystem* renderSystem, SetHandle setHandle);
};
