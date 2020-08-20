#pragma once

#include <GAL/RenderCore.h>
#include <GTSL/Array.hpp>
#include <GTSL/Buffer.h>
#include <GTSL/Vector.hpp>

#include "RenderTypes.h"
#include "ByteEngine/Game/System.h"
#include "ByteEngine/Resources/MaterialResourceManager.h"

struct TaskInfo;
class RenderSystem;

class MaterialSystem : public System
{
public:
	MaterialSystem() : System("MaterialSystem"), materialNames(16, GetPersistentAllocator())
	{}

	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	void SetGlobalState(GameInstance* gameInstance, const GTSL::Array<GTSL::Array<BindingType, 6>, 6>& globalState);
	void AddRenderGroup(GameInstance* gameInstance, const GTSL::Id64 renderGroupName, const GTSL::Array<GTSL::Array<BindingType, 6>, 6>& bindings);
	
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
		
		struct ShaderParameters
		{
			GTSL::Array<Id, 6> ParameterNames;
			GTSL::Array<uint32, 6> ParameterOffset;
		} ShaderParameters;
		
		MaterialInstance() = default;
	};

	struct RenderGroupData
	{
		Id RenderGroupName;
		BindingsSetLayout BindingsSetLayout;
		BindingsPool BindingsPool;
		PipelineLayout PipelineLayout;
		GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> BindingsSets;

		Buffer Buffer;
		void* Data;
		RenderAllocation Allocation;

		GTSL::FlatHashMap<MaterialInstance, BE::PersistentAllocatorReference> Instances;

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

	void SetMaterialTexture(const ComponentReference material, Image* image)
	{
		
	}

	GTSL::Array<BindingsSetLayout, 6> globalBindingsSetLayout;
	GTSL::Array<BindingsSet, MAX_CONCURRENT_FRAMES> globalBindingsSets;
	BindingsPool globalBindingsPool;
	PipelineLayout globalPipelineLayout;
	
private:
	Vector<GTSL::Pair<Id, Id>> materialNames;

	ComponentReference component = 0;

	GTSL::FlatHashMap<RenderGroupData, BE::PersistentAllocatorReference> renderGroups;
	
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
};
