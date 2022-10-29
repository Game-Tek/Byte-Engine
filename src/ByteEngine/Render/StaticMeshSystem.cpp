#include "StaticMeshSystem.h"

#include "RenderOrchestrator.h"
#include "RenderSystem.h"
#include "ByteEngine/Game/ApplicationManager.h"

class RenderStaticMeshCollection;

StaticMeshSystem::StaticMeshSystem(const InitializeInfo& initializeInfo): System(initializeInfo, u8"StaticMeshSystem"), transformations(16, GetPersistentAllocator()), meshes(16, GetPersistentAllocator()), StaticMeshTypeIndentifier(GetApplicationManager()->RegisterType(this, u8"StaticMesh")), OnAddMeshEventHandle(GetApplicationManager()->RegisterEvent<StaticMeshHandle, GTSL::StaticString<64>>(this, u8"OnAddMesh")), OnUpdateMeshEventHandle(GetApplicationManager()->RegisterEvent<StaticMeshHandle, GTSL::Matrix3x4>(this, u8"OnUpdateMesh")) {
	DeleteStaticMesh = GetApplicationManager()->RegisterTask(this, u8"deleteStaticMeshes", {}, &StaticMeshSystem::deleteMesh);
	GetApplicationManager()->BindDeletionTaskToType(StaticMeshTypeIndentifier, DeleteStaticMesh);
}

StaticMeshSystem::StaticMeshHandle StaticMeshSystem::AddStaticMesh(GTSL::StringView mesh_name) {
	uint32 index = transformations.Emplace();

	meshes.Emplace(Mesh{mesh_name});

	auto handle = GetApplicationManager()->MakeHandle<StaticMeshHandle>(StaticMeshTypeIndentifier, index);

	GetApplicationManager()->DispatchEvent(this, GetOnAddMeshEventHandle(), GTSL::MoveRef(handle), GTSL::StaticString<64>(mesh_name));

	return handle;
}

GTSL::ShortString<64> ToString(const GAL::ShaderDataType type) {
	switch (type) { case GAL::ShaderDataType::FLOAT: break;
	case GAL::ShaderDataType::FLOAT2: return u8"FLOAT2";
	case GAL::ShaderDataType::FLOAT3: return u8"FLOAT3";
	case GAL::ShaderDataType::FLOAT4: break;
	case GAL::ShaderDataType::INT: break;
	case GAL::ShaderDataType::INT2: break;
	case GAL::ShaderDataType::INT3: break;
	case GAL::ShaderDataType::INT4: break;
	case GAL::ShaderDataType::BOOL: break;
	case GAL::ShaderDataType::MAT3: break;
	case GAL::ShaderDataType::MAT4: break;
	default: return u8"Uh oh";
	}

	return u8"Uh oh";
}
