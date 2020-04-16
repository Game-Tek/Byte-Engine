#include "StaticMesh.h"

#include "Resources/StaticMeshResourceManager.h"
#include "Application/Application.h"

StaticMesh::StaticMesh(const GTSL::String& _Name)
{
	BE::Application::Get()->GetResourceManager()->GetSubResourceManager<StaticMeshResourceManager>()->TryGetResource(_Name);
}

StaticMesh::~StaticMesh()
{
	BE::Application::Get()->GetResourceManager()->GetSubResourceManager<StaticMeshResourceManager>()->ReleaseResource("Name");
}

//Model StaticMesh::GetModel() const
//{
//	return Model{ staticMeshResource->GetStaticMeshData().VertexArray, staticMeshResource->GetStaticMeshData().IndexArray, staticMeshResource->GetStaticMeshData().VertexCount, staticMeshResource->GetStaticMeshData().IndexCount };
//}
