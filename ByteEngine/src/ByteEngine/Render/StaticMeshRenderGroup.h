#pragma once

#include "MaterialSystem.h"
#include "RenderGroup.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "RenderTypes.h"

class RenderSystem;

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

	[[nodiscard]] GTSL::Ranger<GTSL::Vector3> GetPositions() const { return positions; }
	[[nodiscard]] GTSL::Ranger<const GTSL::Id64> GetResourceNames() const { return resourceNames; }

	void SetPosition(ComponentReference component, GTSL::Vector3 vector3) { positions[component] = vector3; }


	struct Mesh
	{
		Buffer Buffer;
		uint32 IndicesOffset;
		uint32 IndicesCount;
		IndexType IndexType;

		MaterialHandle Material;
	};
	
private:
	struct MeshLoadInfo
	{
		MeshLoadInfo(RenderSystem* renderDevice, const Buffer& buffer, HostRenderAllocation renderAllocation, uint32 instance, MaterialHandle material) : RenderSystem(renderDevice), ScratchBuffer(buffer),
		Allocation(renderAllocation), InstanceId(instance), Material(material)
		{
		}
		
		RenderSystem* RenderSystem = nullptr;
		Buffer ScratchBuffer;
		HostRenderAllocation Allocation;
		uint32 InstanceId;
		MaterialHandle Material;
	};
	
	void onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	uint32 index = 0;

	GTSL::FlatHashMap<GTSL::KeepVector<Mesh, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> meshes;
	GTSL::Vector<RenderAllocation, BE::PersistentAllocatorReference> renderAllocations;

	GTSL::Array<GTSL::Id64, 16> resourceNames;
	GTSL::Vector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;
	
public:
	GTSL::FlatHashMap<GTSL::KeepVector<Mesh, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>& GetMeshes() { return meshes; }
};
