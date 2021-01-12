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
	
	BE_LOG_MESSAGE("Initialized StaticMeshRenderGroup");
}

void StaticMeshRenderGroup::Shutdown(const ShutdownInfo& shutdownInfo)
{
}

ComponentReference StaticMeshRenderGroup::AddStaticMesh(const AddStaticMeshInfo& addStaticMeshInfo)
{
	uint32 vertexCount = 0, vertexSize = 0, indexCount = 0, indexSize = 0;
	addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &vertexCount, &vertexSize, &indexCount, &indexSize);

	auto sharedMesh = addStaticMeshInfo.RenderSystem->CreateSharedMesh(addStaticMeshInfo.MeshName, vertexCount, vertexSize, indexCount, indexSize);

	uint32 index = positions.Emplace();
	
	auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, sharedMesh, index, addStaticMeshInfo.Material);

	auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

	auto bufferSize = vertexCount * vertexSize + indexCount * indexSize;
	
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
	load_static_meshInfo.OnStaticMeshLoad = Task<StaticMeshResourceManager::OnStaticMeshLoad>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, addStaticMeshInfo.RenderSystem->GetSharedMeshPointer(sharedMesh));
	load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
	load_static_meshInfo.IndicesAlignment = indexSize;
	load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);	
	load_static_meshInfo.ActsOn = acts_on;
	load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
	addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);

	resourceNames.EmplaceBack(addStaticMeshInfo.MeshName.GetHash());
	

	++staticMeshCount;
	
	return ComponentReference(GetSystemId(), index);
}

ComponentReference StaticMeshRenderGroup::AddRayTracedStaticMesh(const AddRayTracedStaticMeshInfo& addStaticMeshInfo)
{
	uint32 index = 0;
	if (BE::Application::Get()->GetOption("rayTracing"))
	{
		uint32 vertexCount = 0, vertexSize = 0, indexCount = 0, indexSize = 0;
		addStaticMeshInfo.StaticMeshResourceManager->GetMeshSize(addStaticMeshInfo.MeshName, &vertexCount, &vertexSize, &indexCount, &indexSize);

		auto sharedMesh = addStaticMeshInfo.RenderSystem->CreateSharedMesh(addStaticMeshInfo.MeshName, vertexCount, vertexSize, indexCount, indexSize);

		//uint32 index = positions.Emplace();

		auto* mesh_load_info = GTSL::New<MeshLoadInfo>(GetPersistentAllocator(), addStaticMeshInfo.RenderSystem, sharedMesh, index, addStaticMeshInfo.Material);

		auto acts_on = GTSL::Array<TaskDependency, 16>{ { "RenderSystem", AccessType::READ_WRITE }, { "StaticMeshRenderGroup", AccessType::READ_WRITE } };

		auto bufferSize = GTSL::Math::RoundUpByPowerOf2(vertexCount * vertexSize, addStaticMeshInfo.RenderSystem->GetBufferSubDataAlignment()) + indexCount * indexSize;

		StaticMeshResourceManager::LoadStaticMeshInfo load_static_meshInfo;
		load_static_meshInfo.OnStaticMeshLoad = GTSL::Delegate<void(TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onRayTracedStaticMeshLoaded>(this);
		load_static_meshInfo.DataBuffer = GTSL::Range<byte*>(bufferSize, addStaticMeshInfo.RenderSystem->GetSharedMeshPointer(sharedMesh));
		load_static_meshInfo.Name = addStaticMeshInfo.MeshName;
		load_static_meshInfo.IndicesAlignment = addStaticMeshInfo.RenderSystem->GetBufferSubDataAlignment();
		load_static_meshInfo.UserData = DYNAMIC_TYPE(MeshLoadInfo, mesh_load_info);
		load_static_meshInfo.ActsOn = acts_on;
		load_static_meshInfo.GameInstance = addStaticMeshInfo.GameInstance;
		addStaticMeshInfo.StaticMeshResourceManager->LoadStaticMesh(load_static_meshInfo);
	}
	
	//resourceNames.EmplaceBack(addStaticMeshInfo.MeshName.GetHash());

	return ComponentReference(GetSystemId(), index);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
	{
		MeshLoadInfo* loadInfo = DYNAMIC_CAST(MeshLoadInfo, onStaticMeshLoad.UserData);

		auto meshRef = loadInfo->RenderSystem->CreateGPUMesh(loadInfo->MeshHandle);
		loadInfo->RenderSystem->AddMeshToId(meshRef, loadInfo->Material.MaterialType);

		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};
	
	taskInfo.GameInstance->AddDynamicTask("loadStaticMeshFromManager", Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
	GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}

void StaticMeshRenderGroup::onRayTracedStaticMeshLoaded(TaskInfo taskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
	auto loadStaticMesh = [](TaskInfo, StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad, StaticMeshRenderGroup* staticMeshRenderGroup)
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
		loadInfo->RenderSystem->CreateRayTracedMesh(meshInfo);
		
		GTSL::Delete(loadInfo, staticMeshRenderGroup->GetPersistentAllocator());
	};
	
	taskInfo.GameInstance->AddDynamicTask("onRTLoad", Task<StaticMeshResourceManager::OnStaticMeshLoad, StaticMeshRenderGroup*>::Create(loadStaticMesh),
		GTSL::Array<TaskDependency, 2>{ {"StaticMeshRenderGroup", AccessType::READ_WRITE} }, GTSL::MoveRef(onStaticMeshLoad), this);
}
