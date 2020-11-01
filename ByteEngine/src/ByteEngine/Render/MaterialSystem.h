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
};

MAKE_HANDLE(Id, Set)

class MaterialSystem : public System
{
public:
	MaterialSystem() : System("MaterialSystem")
	{}
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	
	byte* GetMemberPointer(uint64 member, uint32 index)
	{
		byte* data = reinterpret_cast<byte*>(&member);

		auto& setBufferData = setsBufferData[data[0]];
		auto memberSize = setBufferData.MemberSize;
		return static_cast<byte*>(setBufferData.Allocations[frame].Data) + (index * memberSize) + data[1];
	}

	struct Member
	{
		enum class DataType : uint8
		{
			FLOAT32, INT32, MATRIX4, FVEC4
		};

		DataType Type;
	};

	struct MemberInfo : Member
	{
		uint64* Handle;
	};

	enum class Frequency : uint8
	{
		PER_INSTANCE
	};
	
	struct StructInfo
	{
		Frequency Frequency;
		GTSL::Range<MemberInfo*> Members;
		uint64* Handle;
	};
	
	struct SetInfo
	{
		GTSL::Range<StructInfo*> Structs;
	};
	SetHandle AddSet(Id setName, Id parent, const SetInfo& setInfo);

	void AddObjects(RenderSystem* renderSystem, Id renderGroup, uint32 count);


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
	MaterialHandle CreateMaterial(const CreateMaterialInfo& info);

	void SetDynamicMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data);
	void SetMaterialParameter(const MaterialHandle material, GAL::ShaderDataType type, Id parameterName, void* data);
	
private:
	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	struct MaterialData
	{
		struct MaterialInstanceData
		{
			
		};
		
		GTSL::KeepVector<MaterialInstanceData, BE::PAR> MaterialInstances;

		PipelineLayout PipelineLayout;
		RasterizationPipeline Pipeline;
	};
	GTSL::KeepVector<MaterialData, BE::PAR> materials;
	GTSL::FlatHashMap<uint32, BE::PAR> materialsMap;
	
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
		DescriptorsUpdate();

		void Initialize(const BE::PAR& allocator)
		{
			setsToUpdate.Initialize(4, allocator);
			PerSetBufferBindingsUpdate.Initialize(4, allocator);
			PerSetTextureBindingsUpdate.Initialize(4, allocator);
		}

		void AddSetToUpdate(const BE::PAR& allocator)
		{
			PerSetBufferBindingsUpdate.EmplaceBack(4, allocator);
			PerSetTextureBindingsUpdate.EmplaceBack(4, allocator);
		}

		void Reset()
		{
			setsToUpdate.ResizeDown(0);
			PerSetBufferBindingsUpdate.ResizeDown(0);
			PerSetTextureBindingsUpdate.ResizeDown(0);
		}
		
		GTSL::Vector<uint32, BE::PAR> setsToUpdate;

		GTSL::Vector<GTSL::SparseVector<BindingsSet::BufferBindingsUpdateInfo, BE::PAR>, BE::PAR> PerSetBufferBindingsUpdate;
		GTSL::Vector<GTSL::SparseVector<BindingsSet::TextureBindingsUpdateInfo, BE::PAR>, BE::PAR> PerSetTextureBindingsUpdate;
	};
	GTSL::Array<DescriptorsUpdate, MAX_CONCURRENT_FRAMES> descriptorsUpdates;
	
	struct RenderGroupData
	{
		uint32 SetReference;
	};
	GTSL::FlatHashMap<RenderGroupData, BE::PAR> renderGroupsData;
	
	struct SetData
	{
		Id Name;
		void* Parent;
		PipelineLayout PipelineLayout;
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		GTSL::Array<::BindingsSetLayout, 16> BindingsSetLayouts;
	};
	
	GTSL::Tree<SetData, BE::PAR> setsTree;
	GTSL::FlatHashMap<decltype(setsTree)::Node*, BE::PAR> setNodes;

	struct Struct
	{
		enum class Frequency : uint8
		{
			PER_INSTANCE
		} Frequency;
		
		GTSL::Array<Member, 8> Members;
	};
	
	struct SetBufferData
	{
		/**
		 * \brief Size (in bytes) of the structure this set has. Right now is only one "Member" but could be several.
		 */
		uint32 MemberSize = 0;
		HostRenderAllocation Allocations[MAX_CONCURRENT_FRAMES];
		Buffer Buffers[MAX_CONCURRENT_FRAMES];

		uint32 UsedInstances = 0, AllocatedInstances = 0;
		
		GTSL::Array<uint32, 8> AllocatedStructsPerInstance;
		GTSL::Array<uint16, 8> StructsSizes;
		GTSL::Array<Struct, 8> Structs;

		BindingsSet BindingsSet[MAX_CONCURRENT_FRAMES];
	};
	GTSL::KeepVector<SetBufferData, BE::PAR> setsBufferData;

	GTSL::PagedVector<uint32, BE::PAR> queuedBufferUpdates;
	
	struct TextureLoadInfo
	{
		TextureLoadInfo(uint32 component, Buffer buffer, RenderSystem* renderSystem, HostRenderAllocation renderAllocation) : Component(component), Buffer(buffer), RenderSystem(renderSystem), RenderAllocation(renderAllocation)
		{
		}

		uint32 Component;
		Buffer Buffer;
		RenderSystem* RenderSystem;
		HostRenderAllocation RenderAllocation;
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

	uint16 component = 0;
	
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
			shaderInfos.PushBack({ ConvertShaderType(onMaterialLoadInfo.ShaderTypes[i]), &container[i] });
		}
	}

	PipelineLayout rayTracingPipelineLayout;
	
	uint16 minUniformBufferOffset = 0;
	
	uint8 frame;

	void resizeSet(RenderSystem* renderSystem, uint32 set);
};
