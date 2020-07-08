#include "StaticMeshRenderGroup.h"

#include "RenderStaticMeshCollection.h"
#include "ByteEngine/Application/Application.h"
#include "ByteEngine/Game/GameInstance.h"

class RenderStaticMeshCollection;

void StaticMeshRenderGroup::AddStaticMesh(ComponentReference componentReference, RenderStaticMeshCollection* renderStaticMeshCollection)
{
	StaticMeshResourceManager::LoadStaticMeshInfo load_static_mesh_info;
	load_static_mesh_info.OnStaticMeshLoad = GTSL::Delegate<void(StaticMeshResourceManager::OnStaticMeshLoad)>::Create<StaticMeshRenderGroup, &StaticMeshRenderGroup::onStaticMeshLoaded>(this);
	load_static_mesh_info.MeshDataBuffer = GTSL::Ranger<byte>(4 * 1024 * 1024, static_cast<byte*>(data));
	load_static_mesh_info.Name = renderStaticMeshCollection->ResourceNames[componentReference];
	//static_cast<StaticMeshResourceManager*>(BE::Application::Get()->GetGameInstance()->GetSubResourceManager("StaticMeshResourceManager"))->LoadStaticMesh(load_static_mesh_info);
}

void StaticMeshRenderGroup::onStaticMeshLoaded(StaticMeshResourceManager::OnStaticMeshLoad onStaticMeshLoad)
{
}
