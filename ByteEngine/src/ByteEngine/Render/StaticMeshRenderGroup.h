#pragma once

#include <GTSL/Math/Vector3.h>

#include "MaterialSystem.h"
#include "RenderGroup.h"
#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

MAKE_HANDLE(uint32, StaticMesh)

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
		MaterialInstanceHandle Material;
	};
	StaticMeshHandle AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);

	struct AddRayTracedStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialInstanceHandle Material;
	};
	StaticMeshHandle AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo);

	[[nodiscard]] auto GetPositions() const { return positions.GetRange(); }
	[[nodiscard]] GTSL::Range<const GTSL::Id64*> GetResourceNames() const { return resourceNames; }

	void SetPosition(StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) { positions[staticMeshHandle()] = vector3; }
	//void SetPosition(ComponentReference component, GTSL::Vector3 vector3) { positions[component.Component] = vector3; }
	uint32 GetStaticMesheCount() const { return staticMeshCount; }

	auto GetAddedMeshes()
	{
		return addedMeshes.GetReference();
	}

	void ClearAddedMeshes() { addedMeshes.Clear(); }
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, RenderSystem::MeshHandle meshHandle, uint32 instance, MaterialInstanceHandle material) : RenderSystem(renderDevice), MeshHandle(meshHandle),
		InstanceId(instance), Material(material)
		{
		}
		
		RenderSystem* RenderSystem;
		RenderSystem::MeshHandle MeshHandle;
		uint32 InstanceId;
		MaterialInstanceHandle Material;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);
	void onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	uint32 staticMeshCount = 0;
	GTSL::KeepVector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
	GTSL::KeepVector<RenderSystem::MeshHandle, BE::PAR> meshes;
	GTSL::PagedVector<RenderSystem::MeshHandle, BE::PAR> addedMeshes;
};
