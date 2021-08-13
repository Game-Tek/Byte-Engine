#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/ApplicationManager.h"

class RenderStaticMeshCollection;

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 index = transformations.Emplace();
	
	auto resourceLookup = resourceNames.TryEmplace(addStaticMeshInfo.MeshName);
	
	ResourceData* resource = nullptr;
	
	if(resourceLookup.State()) {
		resource = &resourceLookup.Get();
		
		RenderSystem::MeshHandle meshHandle = addStaticMeshInfo.RenderSystem->CreateMesh(addStaticMeshInfo.MeshName, index);
		resource->MeshHandle = meshHandle;
		
		if (BE::Application::Get()->GetOption(u8"rayTracing")) {
			addStaticMeshInfo.RenderSystem->CreateRayTracedMesh(meshHandle);
		}
	
		addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMeshInfo(addStaticMeshInfo.GameInstance, addStaticMeshInfo.MeshName, onStaticMeshInfoLoadHandle, MeshLoadInfo(addStaticMeshInfo.RenderSystem, index, meshHandle));
		addedMeshes.EmplaceBack(AddedMeshData{ false, StaticMeshHandle(index), resource->MeshHandle });
	} else {
		resource = &resourceLookup.Get();
	
		if (resource->Loaded) {
			addedMeshes.EmplaceBack(AddedMeshData{ true, StaticMeshHandle(index), resource->MeshHandle });
		}
	}
	
	resource->DependentMeshes.EmplaceBack(StaticMeshHandle(index));
	meshes.Emplace(Mesh{ resource->MeshHandle, addStaticMeshInfo.Material });
	dirtyMeshes.EmplaceBack(StaticMeshHandle(index));
	
	return StaticMeshHandle(index);
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

void StaticMeshRenderGroup::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad)
{
	meshLoad.RenderSystem->UpdateMesh(meshLoad.MeshHandle, staticMeshInfo.VertexCount, staticMeshInfo.VertexSize, staticMeshInfo.IndexCount, staticMeshInfo.IndexSize, staticMeshInfo.VertexDescriptor);
	
	staticMeshResourceManager->LoadStaticMesh(taskInfo.ApplicationManager, staticMeshInfo, meshLoad.RenderSystem->GetBufferSubDataAlignment(), GTSL::Range<byte*>(meshLoad.RenderSystem->GetMeshSize(meshLoad.MeshHandle), meshLoad.RenderSystem->GetMeshPointer(meshLoad.MeshHandle)), onStaticMeshLoadHandle, GTSL::MoveRef(meshLoad));
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo)
{	
	meshLoadInfo.RenderSystem->UpdateMesh(meshLoadInfo.MeshHandle);

	if (BE::Application::Get()->GetOption(u8"rayTracing"))
	{
		meshLoadInfo.RenderSystem->UpdateRayTraceMesh(meshLoadInfo.MeshHandle);
	}

	//meshLoadInfo.RenderSystem->SetWillWriteMesh(meshLoadInfo.MeshHandle, false);

	auto& resource = resourceNames[staticMeshInfo.Name];
	resource.Loaded = true;
	
	for (uint32 i = 0; i < resource.DependentMeshes.GetLength(); ++i) {
		addedMeshes.EmplaceBack(AddedMeshData{ true, resource.DependentMeshes[i], meshLoadInfo.MeshHandle });
	}
}