#pragma once

#include <GTSL/Math/Vector3.h>

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


	struct Mesh
	{
		Buffer Buffer;
		uint32 IndicesOffset;
		uint32 IndicesCount;
		IndexType IndexType;

		MaterialHandle Material;
	};

	struct RayTracingMesh : Mesh
	{
		AccelerationStructure AccelerationStructure;
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
	void onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad);

	
	GTSL::FlatHashMap<GTSL::Vector<uint32, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference> meshesRefTable;
	GTSL::KeepVector<RenderAllocation, BE::PersistentAllocatorReference> renderAllocations;

	GTSL::Array<GTSL::Id64, 16> resourceNames;

	GTSL::KeepVector<Mesh, BE::PersistentAllocatorReference> meshes;
	GTSL::KeepVector<GTSL::Vector3, BE::PersistentAllocatorReference> positions;

	uint32 meshCount = 0;
	
public:
	const GTSL::FlatHashMap<GTSL::Vector<uint32, BE::PersistentAllocatorReference>, BE::PersistentAllocatorReference>& GetMeshesByMaterial() { return meshesRefTable; }
	[[nodiscard]] auto GetMeshes() const { return meshes.begin(); }
	[[nodiscard]] uint32 GetMeshCount() const { return meshCount; }
};
