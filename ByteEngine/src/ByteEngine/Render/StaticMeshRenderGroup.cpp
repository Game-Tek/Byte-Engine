#include "StaticMeshRenderGroup.h"

#include "RenderSystem.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

StaticMeshRenderGroup::StaticMeshRenderGroup()
{
}

void StaticMeshRenderGroup::Initialize(const InitializeInfo& initializeInfo)
{
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>("RenderSystem");
	positions.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	meshes.Initialize(32, GetPersistentAllocator());
	addedMeshes.Initialize(2, 16, GetPersistentAllocator());

	{
		auto acts_on = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "StaticMeshRenderGroup", AccessTypes::READ_WRITE } };
		onStaticMeshInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onStaticMeshInfoLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshInfoLoaded>(this), acts_on);
	}
	
	{
		auto acts_on = GTSL::Array<TaskDependency, 4>{ { "RenderSystem", AccessTypes::READ_WRITE }, { "StaticMeshRenderGroup", AccessTypes::READ_WRITE } };
		onStaticMeshLoadHandle = initializeInfo.GameInstance->StoreDynamicTask("onStaticMeshLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this), acts_on);
	}
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 index = positions.Emplace();
	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName.GetHash());

	auto meshHandle = addStaticMeshInfo.RenderSystem->CreateMesh(addStaticMeshInfo.MeshName, index, addStaticMeshInfo.Material);

	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		addStaticMeshInfo.RenderSystem->CreateRayTracedMesh(meshHandle);
	}
	
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMeshInfo(addStaticMeshInfo.GameInstance, addStaticMeshInfo.MeshName, onStaticMeshInfoLoadHandle, MeshLoadInfo(addStaticMeshInfo.RenderSystem, index, meshHandle));
	
	return StaticMeshHandle(index);
}

void StaticMeshRenderGroup::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad)
{
	meshLoad.RenderSystem->UpdateMesh(meshLoad.MeshHandle, staticMeshInfo.VertexCount, staticMeshInfo.VertexSize, staticMeshInfo.IndexCount, staticMeshInfo.IndexSize, staticMeshInfo.VertexDescriptor);

	staticMeshResourceManager->LoadStaticMesh(taskInfo.GameInstance, staticMeshInfo, meshLoad.RenderSystem->GetBufferSubDataAlignment(), GTSL::Range<byte*>(meshLoad.RenderSystem->GetMeshSize(meshLoad.MeshHandle), meshLoad.RenderSystem->GetMeshPointer(meshLoad.MeshHandle)), onStaticMeshLoadHandle, GTSL::MoveRef(meshLoad));
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo)
{
	meshLoadInfo.RenderSystem->UpdateMesh(meshLoadInfo.MeshHandle);

	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		meshLoadInfo.RenderSystem->UpdateRayTraceMesh(meshLoadInfo.MeshHandle);
	}

	//meshLoadInfo.RenderSystem->SetWillWriteMesh(meshLoadInfo.MeshHandle, false);
	
	++staticMeshCount;
	meshes.EmplaceAt(meshLoadInfo.InstanceId, meshLoadInfo.MeshHandle);
	addedMeshes.EmplaceBack(meshLoadInfo.MeshHandle, meshLoadInfo.InstanceId);
}