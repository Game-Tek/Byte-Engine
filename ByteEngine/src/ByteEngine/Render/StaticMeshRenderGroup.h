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

	[[nodiscard]] auto GetPositions() const { return positions.GetRange(); }
	[[nodiscard]] GTSL::Range<const GTSL::Id64*> GetResourceNames() const { return resourceNames; }

	void SetPosition(StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) { positions[staticMeshHandle()] = vector3; }
	uint32 GetStaticMeshCount() const { return staticMeshCount; }

	auto GetMeshHandles() const { return meshes.GetRange(); }
	
	auto GetAddedMeshes()
	{
		return addedMeshes.GetReference();
	}

	void ClearAddedMeshes() { addedMeshes.Clear(); }
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, uint32 instance, MaterialInstanceHandle material) : RenderSystem(renderDevice), InstanceId(instance), Material(material)
		{
		}
		
		RenderSystem* RenderSystem;
		RenderSystem::MeshHandle MeshHandle;
		uint32 InstanceId;
		MaterialInstanceHandle Material;
	};
	
	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad);
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo);

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	uint32 staticMeshCount = 0;
	GTSL::KeepVector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
	GTSL::KeepVector<RenderSystem::MeshHandle, BE::PAR> meshes;
	GTSL::PagedVector<GTSL::Pair<RenderSystem::MeshHandle, uint32>, BE::PAR> addedMeshes;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshLoadHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshInfoLoadHandle;
};
