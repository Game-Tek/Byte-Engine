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
	GTSL::Matrix4 GetMeshTransform(uint32 index) { return transformations[index]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }

	struct AddStaticMeshInfo
	{
		Id MeshName;
		RenderSystem* RenderSystem = nullptr;
		class GameInstance* GameInstance = nullptr;
		StaticMeshResourceManager* StaticMeshResourceManager = nullptr;
		MaterialInstanceHandle Material;
	};
	StaticMeshHandle AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo);

	[[nodiscard]] auto GetTransformations() const { return transformations.GetRange(); }
	[[nodiscard]] GTSL::Range<const GTSL::Id64*> GetResourceNames() const { return resourceNames; }

	void SetPosition(StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
	}

	void SetRotation(StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
	}
	
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
		MeshLoadInfo(RenderSystem* renderDevice, uint32 instance, RenderSystem::MeshHandle meshHandle) : RenderSystem(renderDevice), InstanceId(instance), MeshHandle(meshHandle)
		{
		}
		
		RenderSystem* RenderSystem;
		RenderSystem::MeshHandle MeshHandle;
		uint32 InstanceId;
	};
	
	void onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad);
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo);

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	uint32 staticMeshCount = 0;
	
	GTSL::KeepVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;
	GTSL::KeepVector<RenderSystem::MeshHandle, BE::PAR> meshes;
	GTSL::PagedVector<GTSL::Pair<RenderSystem::MeshHandle, uint32>, BE::PAR> addedMeshes;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshLoadHandle;
	DynamicTaskHandle<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo> onStaticMeshInfoLoadHandle;
};
