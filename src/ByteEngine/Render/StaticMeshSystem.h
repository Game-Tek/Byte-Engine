#pragma once

#include <GTSL/PagedVector.h>
#include <GTSL/Math/Vectors.hpp>

#include "RenderSystem.h"
#include "ByteEngine/Resources/StaticMeshResourceManager.h"

#include "ByteEngine/Handle.hpp"

#include <GTSL/String.hpp>

class WorldRendererPipeline;

class StaticMeshSystem final : public BE::System {
public:
	StaticMeshSystem(const InitializeInfo& initializeInfo);

	DECLARE_BE_TYPE(StaticMesh)

	StaticMeshHandle AddStaticMesh(GTSL::StringView mesh_name);

	GTSL::Matrix4 GetMeshTransform(StaticMeshHandle index) { return transformations[index()]; }
	GTSL::Matrix4& GetTransformation(StaticMeshHandle staticMeshHandle) { return transformations[staticMeshHandle()]; }
	GTSL::Vector3 GetMeshPosition(StaticMeshHandle staticMeshHandle) const { return GTSL::Math::GetTranslation(transformations[staticMeshHandle()]); }

	GTSL::StaticString<64> GetMeshName(const StaticMeshHandle static_mesh_handle) const { 
		return meshes[static_mesh_handle()].meshResourceName;
	}

	DECLARE_BE_EVENT(OnAddMesh, StaticMeshHandle, GTSL::StaticString<64>);
	DECLARE_BE_EVENT(OnUpdateMesh, StaticMeshHandle, GTSL::Matrix3x4);

	void SetPosition(StaticMeshHandle staticMeshHandle, GTSL::Vector3 vector3) {
		GTSL::Math::SetTranslation(transformations[staticMeshHandle()], vector3);
		GetApplicationManager()->DispatchEvent(this, GetOnUpdateMeshEventHandle(), GTSL::MoveRef(staticMeshHandle), GTSL::Matrix3x4(transformations[staticMeshHandle()]));
	}

	void SetRotation(StaticMeshHandle staticMeshHandle, GTSL::Quaternion quaternion) {
		GTSL::Math::SetRotation(transformations[staticMeshHandle()], quaternion);
		GetApplicationManager()->DispatchEvent(this, GetOnUpdateMeshEventHandle(), GTSL::MoveRef(staticMeshHandle), GTSL::Matrix3x4(transformations[staticMeshHandle()]));
	}
private:	
	GTSL::FixedVector<GTSL::Matrix4, BE::PersistentAllocatorReference> transformations;
	TaskHandle<StaticMeshHandle> DeleteStaticMesh;

	void deleteMesh(const TaskInfo, StaticMeshHandle handle) {
		meshes.Pop(handle());
	}

	struct Mesh {
		GTSL::StaticString<64> meshResourceName;
	};	
	GTSL::FixedVector<Mesh, BE::PAR> meshes;
};
