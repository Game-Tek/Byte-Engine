#pragma once

#include <GTSL/Math/Vector3.h>

#include "MaterialSystem.h"
#include "RenderGroup.h"
#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

class StaticMeshRenderGroup final : public RenderGroup
{
public:
	StaticMeshRenderGroup();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;

	struct AddStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialHandle Material;
	};
	ComponentReference AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);

	struct AddRayTracedStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialHandle Material;
	};
	ComponentReference AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo);

	[[nodiscard]] auto GetPositions() const { return positions.GetRange(); }
	[[nodiscard]] GTSL::Range<const GTSL::Id64*> GetResourceNames() const { return resourceNames; }

	void SetPosition(ComponentReference component, GTSL::Vector3 vector3) { positions[component.Component] = vector3; }
	uint32 GetStaticMeshes() const { return staticMeshCount; }
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, RenderSystem::SharedMeshHandle meshHandle, uint32 instance, MaterialHandle material) : RenderSystem(renderDevice), MeshHandle(meshHandle),
		InstanceId(instance), Material(material)
		{
		}
		
		RenderSystem* RenderSystem;
		RenderSystem::SharedMeshHandle MeshHandle;
		uint32 InstanceId;
		MaterialHandle Material;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);
	void onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	uint32 staticMeshCount = 0;
	GTSL::KeepVector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
};
