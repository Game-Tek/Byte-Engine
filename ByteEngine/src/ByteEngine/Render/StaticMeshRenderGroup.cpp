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
	auto render_device = initializeInfo.GameInstance->GetSystem<RenderSystem>(u8"RenderSystem");
	transformations.Initialize(initializeInfo.ScalingFactor, GetPersistentAllocator());
	meshes.Initialize(32, GetPersistentAllocator());
	addedMeshes.Initialize(2, 16, GetPersistentAllocator());
	resourceNames.Initialize(8, GetPersistentAllocator());

	{
		auto acts_on = GTSL::Array<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"StaticMeshRenderGroup", AccessTypes::READ_WRITE } };
		onStaticMeshInfoLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"onStaticMeshInfoLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshInfoLoaded>(this), acts_on);
	}
	
	{
		auto acts_on = GTSL::Array<TaskDependency, 4>{ { u8"RenderSystem", AccessTypes::READ_WRITE }, { u8"StaticMeshRenderGroup", AccessTypes::READ_WRITE } };
		onStaticMeshLoadHandle = initializeInfo.GameInstance->StoreDynamicTask(u8"onStaticMeshLoad", Task<StaticMeshResourceManager*, StaticMeshResourceManager::StaticMeshInfo, MeshLoadInfo>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this), acts_on);
	}
	
	BE_LOG_MESSAGE(u8"Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 index = transformations.Emplace();

	auto resourceLookup = resourceNames.TryEmplace(addStaticMeshInfo.MeshName);

	ResourceData* resource;

	if(resourceLookup.State()) {
		resource = &resourceLookup.Get();
		
		RenderSystem::MeshHandle meshHandle = addStaticMeshInfo.RenderSystem->CreateMesh(addStaticMeshInfo.MeshName, index);
		resource->MeshHandle = meshHandle;
		
		if (BE::Application::Get()->GetOption(u8"rayTracing")) {
			addStaticMeshInfo.RenderSystem->CreateRayTracedMesh(meshHandle);
		}

		addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMeshInfo(addStaticMeshInfo.GameInstance, addStaticMeshInfo.MeshName, onStaticMeshInfoLoadHandle, MeshLoadInfo(addStaticMeshInfo.RenderSystem, index, meshHandle));
	} else {
		resource = &resourceLookup.Get();

		if (resource->Loaded) {
			addedMeshes.EmplaceBack(AddedMeshData{ StaticMeshHandle(index), resource->MeshHandle });
		}
	}

	resource->DependentMeshes.EmplaceBack(StaticMeshHandle(index));
	meshes.EmplaceAt(index, Mesh{ resource->MeshHandle, addStaticMeshInfo.Material });
	dirtyMeshes.EmplaceBack(StaticMeshHandle(index));
	
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

	if (BE::Application::Get()->GetOption(u8"rayTracing"))
	{
		meshLoadInfo.RenderSystem->UpdateRayTraceMesh(meshLoadInfo.MeshHandle);
	}

	//meshLoadInfo.RenderSystem->SetWillWriteMesh(meshLoadInfo.MeshHandle, false);	

	auto& resource = resourceNames[staticMeshInfo.Name];
	resource.Loaded = true;
	
	for (uint32 i = 0; i < resource.DependentMeshes.GetLength(); ++i) {
		addedMeshes.EmplaceBack(AddedMeshData{ resource.DependentMeshes[i], meshLoadInfo.MeshHandle });
	}
}