#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.h>
#include <GTSL/SparseVector.hpp>
#include <GTSL/Vector.hpp>
#include <GTSL/StaticMap.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"
#include "ByteEngine/Resources/TextureResourceManager.h"

class TextureResourceManager;
struct TaskInfo;
class RenderSystem;

struct MaterialHandle
{
	Id MaterialType;
	uint32 MaterialInstance = 0;
};

class MaterialSystem : public System
{
public:
	MaterialSystem() : System("MaterialSystem")
	{}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	void SetGlobalState(GameInstance* gameInstance, const GTSL::Array<GTSL::Array<BindingType, 6>, 6>& globalState);

	struct AddRenderGroupInfo
	{
		Id Name;
		GTSL::Array<GTSL::Array<BindingType, 6>, 6> Bindings;
		GTSL::Array<GTSL::Array<uint32, 6>, 6> Size;
		GTSL::Array<GTSL::Array<uint32, 6>, 6> Range;
	};
	void AddRenderGroup(GameInstance* gameInstance, const AddRenderGroupInfo& addRenderGroupInfo);

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
	
	struct MaterialInstance
	{
		PipelineLayout PipelineLayout;
		RasterizationPipeline Pipeline;
		
		BindingsPool BindingsPool;

		BindingsSetData TextureParametersBindings;

		Buffer Buffer;
		HostRenderAllocation Allocation;

		
		GTSL::StaticMap<uint16, 16> DynamicParameters;
		GTSL::StaticMap<uint16, 16> Parameters;
		BindingType BindingType;

		
		/**
		 * \brief ABSOLUTE offset to texture index.
		 */
		GTSL::StaticMap<uint16, 16> Textures;

		MaterialInstance() = default;
	};

	[[nodiscard]] const GTSL::KeepVector<MaterialInstance, BE::PersistentAllocatorReference>& GetMaterialInstances() const { return materials; }

	struct RenderGroupData
	{
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool;
		PipelineLayout PipelineLayout;
		BindingsSet BindingsSets[MAX_CONCURRENT_FRAMES];
		
		Buffer Buffer;
		HostRenderAllocation Allocation;
		BindingType BindingType;

		RenderGroupData() = default;
	};

	GTSL::FlatHashMap<RenderGroupData, BE::PersistentAllocatorReference>& GetRenderGroups() { return renderGroups; }
	
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

	void* GetRenderGroupDataPointer(const Id name) { return renderGroups.At(name).Allocation.Data; }
	
	struct UpdateRenderGroupDataInfo
	{
		Id RenderGroup;
		GTSL::Range<const byte*> Data;
		uint32 Offset = 0;
	};
	void UpdateRenderGroupData(const UpdateRenderGroupDataInfo& updateRenderGroupDataInfo);
	
	GTSL::Array<BindingsSetLayout, 6> globalBindingsSetLayout;
	BindingsSet globalBindingsSets[MAX_CONCURRENT_FRAMES];
	BindingsPool globalBindingsPool;
	PipelineLayout globalPipelineLayout;

	bool IsMaterialReady(const MaterialHandle material)
	{
		if (isMaterialReady.IsSlotOccupied(material.MaterialInstance))
		{
			return isMaterialReady[material.MaterialInstance];
		}
	}
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
	
	GTSL::FlatHashMap<uint8, BE::PersistentAllocatorReference> isRenderGroupReady;
	GTSL::KeepVector<uint8, BE::PersistentAllocatorReference> isMaterialReady;

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
	
	ComponentReference material;

	GTSL::FlatHashMap<RenderGroupData, BE::PersistentAllocatorReference> renderGroups;
	GTSL::KeepVector<MaterialInstance, BE::PersistentAllocatorReference> materials;
	
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
