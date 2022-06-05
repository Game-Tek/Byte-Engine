#pragma once

#include <GTSL/PagedVector.h>
#include <GTSL/Math/Vectors.hpp>

#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

class WorldRendererPipeline;

class StaticMeshRenderGroup final : public BE::System
{
public:
	StaticMeshRenderGroup(const InitializeInfo& initializeInfo);

	DECLARE_BE_TYPE(StaticMesh)

	GTSL::Matrix4 GetMeshTransform(StaticMeshHandle index) { return transformations[index()]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }

	StaticMeshHandle AddStaticMesh(Id MeshName, RenderSystem* RenderSystem, ApplicationManager* GameInstance);

	static auto GetOnAddMeshEventHandle() {
		return EventHandle<StaticMeshHandle, Id>(u8"OnAddMesh");
	}

	static auto GetOnUpdateMeshEventHandle() {
		return EventHandle<StaticMeshHandle, GTSL::Matrix3x4>(u8"OnUpdateMesh");
	}

	void SetPosition(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
		application_manager->DispatchEvent(this, GetOnUpdateMeshEventHandle(), GTSL::MoveRef(staticMeshHandle), GTSL::Matrix3x4(transformations[staticMeshHandle()]));
	}

	void SetRotation(ApplicationManager* application_manager, StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
		application_manager->DispatchEvent(this, GetOnUpdateMeshEventHandle(), GTSL::MoveRef(staticMeshHandle), GTSL::Matrix3x4(transformations[staticMeshHandle()]));
	}
private:	
	GTSL::FixedVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;
	TaskHandle<StaticMeshHandle> DeleteStaticMesh;

	void deleteMesh(const TaskInfo, StaticMeshHandle handle) {
		meshes.Pop(handle());
	}

	struct Mesh {
	};
	
	GTSL::FixedVector<Mesh, BE::PAR> meshes;
};
