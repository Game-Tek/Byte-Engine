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
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

StaticMeshHandle StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 vertexCount = 0, vertexSize = 0, indexCount = 0, indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &vertexCount, &vertexSize, &indexCount, &indexSize);

	auto sharedMesh = addStaticMeshInfo.RenderSystem->CreateMesh(addStaticMeshInfo.MeshName, vertexCount, vertexSize, indexCount, indexSize, addStaticMeshInfo.Material);

	uint32 index = positions.Emplace();
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, sharedMesh, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = Task<StaticMeshResourceManager::OnStaticMeshLoad>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(addStaticMeshInfo.RenderSystem->GetMeshSize(sharedMesh), addStaticMeshInfo.RenderSystem->GetMeshPointer(sharedMesh));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = addStaticMeshInfo.RenderSystem->GetBufferSubDataAlignment();
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName.GetHash());

	++staticMeshCount;
	
	return StaticMeshHandle(index);
}

StaticMeshHandle StaticMeshRenderGroup::AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo)
{
	uint32 index = 0;
	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		uint32 vertexCount = 0, vertexSize = 0, indexCount = 0, indexSize = 0;
		addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &vertexCount, &vertexSize, &indexCount, &indexSize);

		auto sharedMesh = addStaticMeshInfo.RenderSystem->CreateMesh(addStaticMeshInfo.MeshName, vertexCount, vertexSize, indexCount, indexSize, addStaticMeshInfo.Material);

		index = positions.Emplace();

		auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, sharedMesh, index, addStaticMeshInfo.Material);

		auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

		auto bufferSize = GTSL::Math::RoundUpByPowerOf2(vertexCount * vertexSize, addStaticMeshInfo.RenderSystem->GetBufferSubDataAlignment()) + indexCount * indexSize;

		StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
		load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onRayTracedStaticMeshLoaded>(this);
		load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, addStaticMeshInfo.RenderSystem->GetMeshPointer(sharedMesh));
		load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
		load_static_meshInfo.IndicesAlignment = addStaticMeshInfo.RenderSystem->GetBufferSubDataAlignment();
		load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);
		load_static_meshInfo.ActsOn = acts_on;
		load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
		addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);
	}

	return StaticMeshHandle(index);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

	auto meshHandle = loadInfo->RenderSystem->UpdateMesh(loadInfo->MeshHandle);
	meshes.EmplaceAt(loadInfo->InstanceId, meshHandle);
	addedMeshes.EmplaceBack(meshHandle);
	
	GTSL::Delete(loadInfo, GetPersistentAllocator());
}

void StaticMeshRenderGroup::onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

	RenderSystem::CreateRayTracingMeshInfo meshInfo;
	meshInfo.SharedMesh = loadInfo->MeshHandle;
	meshInfo.VertexCount = onStaticMeshLoad.VertexCount;
	meshInfo.VertexSize = onStaticMeshLoad.VertexSize;
	meshInfo.IndexCount = onStaticMeshLoad.IndexCount;
	meshInfo.IndexSize = onStaticMeshLoad.IndexSize;

	GTSL::Matrix3x4 matrix(1.0f);
	meshInfo.Matrix = &matrix;
	auto meshHandle = loadInfo->RenderSystem->CreateRayTracedMesh(meshInfo);

	GTSL::Delete(loadInfo, GetPersistentAllocator());
}
