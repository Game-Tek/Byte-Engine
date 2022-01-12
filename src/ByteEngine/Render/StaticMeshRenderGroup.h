#pragma once

#include <GTSL/PagedVector.h>
#include <GTSL/Math/Vectors.hpp>

#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

class WorldRendererPipeline;
MAKE_BE_HANDLE(StaticMesh)

class StaticMeshRenderGroup final : public BE::System
{
public:
	StaticMeshRenderGroup(const InitializeInfo& initializeInfo);

	GTSL::Matrix4 GetMeshTransform(StaticMeshHandle index) { return transformations[index()]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }
	ShaderGroupHandle GetMaterialHandle(StaticMeshHandle i) const { return meshes[i()].MaterialInstanceHandle; }

	StaticMeshHandle AddStaticMesh(Id MeshName, RenderSystem* RenderSystem, ApplicationManager* GameInstance, ShaderGroupHandle Material);

	void SetPosition(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
		application_manager->AddStoredDynamicTask(OnUpdateMesh, GTSL::MoveRef(staticMeshHandle));
	}

	void SetRotation(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
		application_manager->AddStoredDynamicTask(OnUpdateMesh, GTSL::MoveRef(staticMeshHandle));
	}

	void Init(WorldRendererPipeline*);
private:	
	GTSL::FixedVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;
	DynamicTaskHandle<StaticMeshHandle, Id, ShaderGroupHandle> OnAddMesh;
	DynamicTaskHandle<StaticMeshHandle> OnUpdateMesh;
	DynamicTaskHandle<GTSL::Range<const StaticMeshHandle*>> DeleteStaticMeshes;

	void deleteMeshes(const TaskInfo, GTSL::Range<const StaticMeshHandle*> handles) {
		for(auto e : handles) { meshes.Pop(e()); }
	}

	struct Mesh {
		ShaderGroupHandle MaterialInstanceHandle;
	};
	
	GTSL::FixedVector<Mesh, BE::PAR> meshes;

	BE::TypeIdentifer staticMeshEntityIdentifier;
};