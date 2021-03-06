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
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMeshInfo(addStaticMeshInfo.GameInstance, addStaticMeshInfo.MeshName, onStaticMeshInfoLoadHandle, MeshLoadInfo(addStaticMeshInfo.RenderSystem, index, addStaticMeshInfo.Material));

	++staticMeshCount;
	
	return StaticMeshHandle(index);
}

void StaticMeshRenderGroup::onStaticMeshInfoLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoad)
{
	meshLoad.MeshHandle = meshLoad.RenderSystem->CreateMesh(staticMeshInfo.Name, staticMeshInfo.VertexCount, staticMeshInfo.VertexSize, staticMeshInfo.IndexCount, staticMeshInfo.IndexSize, meshLoad.Material);

	staticMeshResourceManager->LoadStaticMesh(taskInfo.GameInstance, staticMeshInfo, meshLoad.RenderSystem->GetBufferSubDataAlignment(), GTSL::Range<byte*>(meshLoad.RenderSystem->GetMeshSize(meshLoad.MeshHandle), meshLoad.RenderSystem->GetMeshPointer(meshLoad.MeshHandle)), onStaticMeshLoadHandle, GTSL::MoveRef(meshLoad));
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager* staticMeshResourceManager, StaticMeshResourceManager::StaticMeshInfo staticMeshInfo, MeshLoadInfo meshLoadInfo)
{
	auto meshHandle = meshLoadInfo.RenderSystem->UpdateMesh(meshLoadInfo.MeshHandle);

	if(BE::Application::Get()->GetOption("rayTracing"))
	{
		RenderSystem::CreateRayTracingMeshInfo meshInfo;
		meshInfo.SharedMesh = meshLoadInfo.MeshHandle;
		meshInfo.VertexCount = staticMeshInfo.VertexCount;
		meshInfo.VertexSize = staticMeshInfo.VertexSize;
		meshInfo.IndexCount = staticMeshInfo.IndexCount;
		meshInfo.IndexSize = staticMeshInfo.IndexSize;
		GTSL::Matrix3x4 matrix(1.0f);
		meshInfo.Matrix = &matrix;

		auto meshHandle = meshLoadInfo.RenderSystem->CreateRayTracedMesh(meshInfo);
	}
	
	meshes.EmplaceAt(meshLoadInfo.InstanceId, meshHandle);
	addedMeshes.EmplaceBack(meshHandle, meshLoadInfo.InstanceId);
}