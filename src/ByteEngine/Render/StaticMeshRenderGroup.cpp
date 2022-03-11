#include "StaticMeshRenderGroup.h"

#include "RenderOrchestrator.h"
#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ApplicationManager.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup(const InitializeInfo& initializeInfo): System(initializeInfo, u8"StaticMeshRenderGroup"),
	transformations(16, GetPersistentAllocator()), meshes(16, GetPersistentAllocator()) {

	staticMeshEntityIdentifier = GetApplicationManager()->RegisterType(this, u8"StaticMesh");

	DeleteStaticMeshes = GetApplicationManager()->RegisterTask(this, u8"deleteStaticMeshes", {}, &StaticMeshRenderGroup::deleteMeshes);
	GetApplicationManager()->BindDeletionTaskToType(staticMeshEntityIdentifier, DeleteStaticMeshes);
}

StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(Id MeshName, RenderSystem* RenderSystem, ApplicationManager* GameInstance) {
	uint32 index = transformations.Emplace();

	meshes.Emplace(Mesh{});

	auto handle = GetApplicationManager()->MakeHandle<StaticMeshHandle>(staticMeshEntityIdentifier, index);

	GameInstance->EnqueueTask(OnAddMesh, GTSL::MoveRef(handle), GTSL::MoveRef(MeshName));

	return handle;
}

void StaticMeshRenderGroup::Init(WorldRendererPipeline* s) {
	OnAddMesh = s->GetOnAddMeshHandle();
	OnUpdateMesh = s->GetOnMeshUpdateHandle();
}

GTSL::ShortString<64> ToString(const GAL::ShaderDataType type)
{
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
