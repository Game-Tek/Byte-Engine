#pragma once

#include <GTSL/StaticMap.hpp>
#include <GTSL/Math/Vectors.h>

#include "MaterialSystem.h"
#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

MAKE_HANDLE(uint32, StaticMesh)

class StaticMeshRenderGroup final : public System
{
public:
	StaticMeshRenderGroup();
	
	void Initialize(const InitializeInfo& initializeInfo) override;
	void Shutdown(const ShutdownInfo& shutdownInfo) override;
	GTSL::Matrix4 GetMeshTransform(StaticMeshHandle index) { return transformations[index()]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }
	MaterialInstanceHandle GetMaterialHandle(StaticMeshHandle i) const { return meshes[i()].MaterialInstanceHandle; }
	RenderSystem::MeshHandle GetMeshHandle(StaticMeshHandle i) const { return meshes[i()].MeshHandle; }
	uint32 GetMeshIndex(const StaticMeshHandle meshHandle) const { return meshHandle(); }
	
	struct AddStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialInstanceHandle Material;
	};
	StaticMeshHandle AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);

	void SetPosition(StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
		dirtyMeshes.EmplaceBack(staticMeshHandle);
	}

	void SetRotation(StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
		dirtyMeshes.EmplaceBack(staticMeshHandle);
	}

	struct AddedMeshData {
		StaticMeshHandle StaticMeshHandle;
		RenderSystem::MeshHandle MeshHandle;
	};
	
	auto& GetAddedMeshes() {
		return addedMeshes;
	}

	void ClearAddedMeshes() { addedMeshes.Clear(); }

	auto GetDirtyMeshes() const { return GTSL::Range(dirtyMeshes.begin(), dirtyMeshes.end()); }
	void ClearDirtyMeshes() { return dirtyMeshes.Resize(0); }
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, uint32 instance, RenderSystem::MeshHandle meshHandle) : RenderSystem(renderDevice), InstanceId(instance), MeshHandle(meshHandle)
		{
		}
		
		RenderSystem* RenderSystem;
		RenderSystem::MeshHandle MeshHandle;
		uint32 InstanceId;
	};
	
	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad);
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo);
	
	GTSL::KeepVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;

	struct Mesh {
		RenderSystem::MeshHandle MeshHandle; MaterialInstanceHandle MaterialInstanceHandle;
	};

	GTSL::Array<StaticMeshHandle, 8> dirtyMeshes;
	
	struct ResourceData {
		bool Loaded;
		GTSL::Array<StaticMeshHandle, 8> DependentMeshes;
		RenderSystem::MeshHandle MeshHandle;
	};
	GTSL::FlatHashMap<Id, ResourceData, BE::PAR> resourceNames;
	
	GTSL::KeepVector<Mesh, BE::PAR> meshes;
	GTSL::PagedVector<AddedMeshData, BE::PAR> addedMeshes;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshLoadHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshInfoLoadHandle;
};
