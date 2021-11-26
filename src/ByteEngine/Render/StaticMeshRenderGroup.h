#pragma once

#include <GTSL/PagedVector.h>
#include <GTSL/Math/Vectors.hpp>

#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

class StaticMeshRenderManager;
MAKE_HANDLE(uint32, StaticMesh)

class StaticMeshRenderGroup final : public ::System
{
public:
	StaticMeshRenderGroup(const InitializeInfo& initializeInfo);

	GTSL::Matrix4 GetMeshTransform(StaticMeshHandle index) { return transformations[index()]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }
	MaterialInstanceHandle GetMaterialHandle(StaticMeshHandle i) const { return meshes[i()].MaterialInstanceHandle; }

	StaticMeshHandle AddStaticMesh(Id MeshName, RenderSystem* RenderSystem, ApplicationManager* GameInstance, MaterialInstanceHandle Material);

	void SetPosition(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
		application_manager->AddStoredDynamicTask(OnUpdateMesh, GTSL::MoveRef(staticMeshHandle));
	}

	void SetRotation(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
		application_manager->AddStoredDynamicTask(OnUpdateMesh, GTSL::MoveRef(staticMeshHandle));
	}

	void Init(StaticMeshRenderManager*);
private:	
	GTSL::FixedVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;
	DynamicTaskHandle<Handle<unsigned, StaticMesh_tag>, Id, MaterialInstanceHandle> OnAddMesh;
	DynamicTaskHandle<Handle<unsigned, StaticMesh_tag>> OnUpdateMesh;

	struct Mesh {
		MaterialInstanceHandle MaterialInstanceHandle;
	};
	
	GTSL::FixedVector<Mesh, BE::PAR> meshes;
};
