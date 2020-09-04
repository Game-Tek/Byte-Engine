#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.h>
#include <GTSL/Vector.hpp>
#include <GTSL/StaticMap.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

struct TaskInfo;
class RenderSystem;

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
	
	struct MaterialInstance
	{
		BindingsSetLayout BindingsSetLayout;
		RasterizationPipeline Pipeline;
		BindingsPool BindingsPool;
		PipelineLayout PipelineLayout;
		GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> BindingsSets;

		Buffer Buffer;
		void* Data;
		RenderAllocation Allocation;

		uint32 DataSize = 0;
		
		GTSL::StaticMap<uint32, 16> Parameters;
		BindingType BindingType;

		MaterialInstance() = default;
	};

	[[nodiscard]] const GTSL::KeepVector<MaterialInstance, BE::PersistentAllocatorReference>& GetMaterialInstances() const { return materials; }

	struct RenderGroupData
	{
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool;
		PipelineLayout PipelineLayout;
		GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> BindingsSets;
		
		Buffer Buffer;
		void* Data;
		RenderAllocation Allocation;
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
	};
	ComponentReference CreateMaterial(const CreateMaterialInfo& info);

	void SetMaterialParameter(const ComponentReference material, GAL::ShaderDataType type, Id parameterName,
	                          void* data);

	void SetMaterialTexture(const ComponentReference material, Id parameterName, const uint8 n, TextureView* image, TextureSampler* sampler);

	void* GetRenderGroupDataPointer(const Id name) { return renderGroups.At(name).Data; }
	
	struct UpdateRenderGroupDataInfo
	{
		Id RenderGroup;
		GTSL::Ranger<const byte> Data;
		uint32 Offset = 0;
	};
	void UpdateRenderGroupData(const UpdateRenderGroupDataInfo& updateRenderGroupDataInfo);
	
	GTSL::Array<BindingsSetLayout, 6> globalBindingsSetLayout;
	GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> globalBindingsSets;
	BindingsPool globalBindingsPool;
	PipelineLayout globalPipelineLayout;

	bool IsMaterialReady(const uint64 material) { return isMaterialReady[material]; }
private:
	void updateDescriptors(TaskInfo taskInfo);
	void updateCounter(TaskInfo taskInfo);

	GTSL::FlatHashMap<uint8, BE::PersistentAllocatorReference> isRenderGroupReady;
	GTSL::KeepVector<uint8, BE::PersistentAllocatorReference> isMaterialReady;

	struct BindingsUpdateData
	{
		struct Updates
		{			
			Vector<BindingsSet::TextureBindingsUpdateInfo> TextureBindingDescriptorsUpdates;
			Vector<BindingsSet::BufferBindingsUpdateInfo> BufferBindingDescriptorsUpdates;
			Vector<BindingType> BufferBindingTypes;
		};

		Updates Global;
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
	
	ComponentReference material = 0;

	GTSL::FlatHashMap<RenderGroupData, BE::PersistentAllocatorReference> renderGroups;
	GTSL::KeepVector<MaterialInstance, BE::PersistentAllocatorReference> materials;
	
	struct MaterialLoadInfo
	{
		MaterialLoadInfo(RenderSystem* renderSystem, GTSL::Buffer&& buffer, uint32 index) : RenderSystem(renderSystem), Buffer(MoveRef(buffer)), Component(index)
		{

		}

		RenderSystem* RenderSystem = nullptr;
		GTSL::Buffer Buffer;
		uint32 Component;
	};
	void onMaterialLoaded(TaskInfo taskInfo, MaterialResourceManager::OnMaterialLoadInfo onMaterialLoadInfo);

	uint16 minUniformBufferOffset = 0;

	uint8 frame;
};
