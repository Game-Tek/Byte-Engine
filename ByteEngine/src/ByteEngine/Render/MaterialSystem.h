#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.h>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/StaticMap.hpp>
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
		return static_cast<byte*>(setBufferData.Allocation[frame].Data) + (index * memberSize) + data[1];
	}

	struct Member
	{
		enum class DataType : uint8
		{
			FLOAT32, INT32, MATRIX4, FVEC4
		};

		DataType Type;
		uint64* Handle;
	};

	struct Struct
	{
		enum class Frequency : uint8
		{
			PER_INSTANCE
		} Frequency;
		GTSL::Range<Member*> Members;
	};
	
	struct SetInfo
	{
		GTSL::Range<Struct*> Structs;
	};
	SetHandle AddSet(Id setName, Id parent, const SetInfo& setInfo);

	void AddObject(Id renderGroup, ComponentReference mesh, MaterialHandle material);
	
	struct MaterialData
	{
		uint16 TextureIndices[8];
		uint8 Parameters[32];
	};

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

	struct CreateTextureInfo
	{
		Id TextureName;
		GameInstance* GameInstance = nullptr;
		RenderSystem* RenderSystem = nullptr;
		TextureResourceManager* TextureResourceManager = nullptr;
		MaterialHandle MaterialHandle;
	};
	ComponentReference createTexture(const CreateTextureInfo& createTextureInfo);

	struct BindingsUpdateData
	{
		struct GlobalUpdates
		{			
			GTSL::SparseVector<BindingsSet::TextureBindingsUpdateInfo, BE::PersistentAllocatorReference> TextureBindingDescriptorsUpdates;
			GTSL::SparseVector<BindingsSet::BufferBindingsUpdateInfo, BE::PersistentAllocatorReference> BufferBindingDescriptorsUpdates;
			Vector<BindingType> BufferBindingTypes;
		};

		struct Updates
		{
			Vector<BindingsSet::TextureBindingsUpdateInfo> TextureBindingDescriptorsUpdates;
			uint32 StartWrittenTextures = 0, EndWrittenTextures = 0, StartWrittenBuffers = 0, EndWrittenBuffers = 0;
			Vector<BindingsSet::BufferBindingsUpdateInfo> BufferBindingDescriptorsUpdates;
			Vector<BindingType> BufferBindingTypes;
		};

		GlobalUpdates Global;
		GTSL::FlatHashMap<Updates, BE::PersistentAllocatorReference> RenderGroups;
		GTSL::KeepVector<Updates, BE::PersistentAllocatorReference> Materials;

		BindingsUpdateData() = default;
		void Initialize(const uint32 num, const BE::PersistentAllocatorReference& allocator)
		{
			Global.BufferBindingDescriptorsUpdates.Initialize(8, allocator);
			Global.TextureBindingDescriptorsUpdates.Initialize(8, allocator);
			Global.BufferBindingTypes.Initialize(8, allocator);
			RenderGroups.Initialize(8, allocator);
			Materials.Initialize(32, allocator);
		}
	};
	GTSL::Array<BindingsUpdateData, MAX_CONCURRENT_FRAMES> perFrameBindingsUpdateData;

	struct RenderGroupData
	{
		
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

	struct SetBufferData
	{
		uint32 MemberSize = 0;
		HostRenderAllocation Allocation[MAX_CONCURRENT_FRAMES];
		Buffer Buffers[MAX_CONCURRENT_FRAMES];
	};
	GTSL::KeepVector<SetBufferData, BE::PAR> setsBufferData;
	
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

	void test();

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
};
