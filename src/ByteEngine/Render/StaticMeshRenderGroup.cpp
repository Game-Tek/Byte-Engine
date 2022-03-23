#include "StaticMeshRenderGroup.h"

#include "RenderOrchestrator.h"
#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ApplicationManager.h"

#include "ByteEngine/Render/WorldRenderPipeline.hpp"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup(const InitializeInfo& initializeInfo): System(initializeInfo, u8"StaticMeshRenderGroup"),
	transformations(16, GetPersistentAllocator()), meshes(16, GetPersistentAllocator()) {

	StaticMeshTypeIndentifier = GetApplicationManager()->RegisterType(this, u8"StaticMesh");

	DeleteStaticMesh = GetApplicationManager()->RegisterTask(this, u8"deleteStaticMeshes", {}, &StaticMeshRenderGroup::deleteMesh);
	GetApplicationManager()->BindDeletionTaskToType(StaticMeshTypeIndentifier, DeleteStaticMesh);

	GetApplicationManager()->AddEvent(u8"SMRG", GetOnAddMeshEventHandle());
	GetApplicationManager()->AddEvent(u8"SMRG", GetOnUpdateMeshEventHandle());
}

StaticMeshRenderGroup::StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(Id MeshName, RenderSystem* RenderSystem, ApplicationManager* GameInstance) {
	uint32 index = transformations.Emplace();

	meshes.Emplace(Mesh{});

	auto handle = GetApplicationManager()->MakeHandle<StaticMeshHandle>(StaticMeshTypeIndentifier, index);

	GameInstance->DispatchEvent(u8"SMRG", GetOnAddMeshEventHandle(), GTSL::MoveRef(handle), GTSL::MoveRef(MeshName));

	return handle;
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
